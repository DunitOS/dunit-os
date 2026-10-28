use crate::{Color, Result, UiError, View};
use alloc::vec::Vec;
use dunit_render::{paint_clipped, Surface};
use dunit_style::{cascade::NodeStyle, Cascade, Style};
use dunit_text::Font;
use dunit_ui::{layout_measured, tree::TreeBuilder, Edges, Layout, Rect, Tree};
use dunit_widgets::{intrinsic_size, FontMeasure, Widget};

/// Integer surface-space CPU invalidation. Wire/compositor damage is separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Damage {
    fn rect(self) -> Rect {
        Rect {
            x: self.x as f32,
            y: self.y as f32,
            w: self.width as f32,
            h: self.height as f32,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderStats {
    pub damage: Option<Damage>,
    /// Number of distinct pixels reset/repainted, not a composition/frame time.
    pub damaged_pixels: u64,
    /// Number of paint-property transitions still in progress after this sample.
    pub active_transitions: usize,
    /// Caller-supplied monotonic milliseconds; None for immediate rendering.
    pub animation_time_ms: Option<u64>,
}
struct Snapshot {
    tree: Tree,
    layout: Layout,
    styles: Vec<Style>,
    targets: Vec<Style>,
}

/// Retains a private CPU canvas; no published shared buffer is ever mutated.
pub struct Renderer {
    font: Font,
    cascade: Cascade,
    background: Color,
    size: (u32, u32),
    pixels: Vec<u32>,
    previous: Option<Snapshot>,
    motion: crate::motion::Motion,
    motion_enabled: bool,
}
impl Renderer {
    pub fn new(font: Font, cascade: Cascade, background: Color) -> Self {
        Self {
            font,
            cascade,
            background,
            size: (0, 0),
            pixels: Vec::new(),
            previous: None,
            motion: crate::motion::Motion::default(),
            motion_enabled: true,
        }
    }
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }
    /// Apply a fully validated candidate. A font-path change also invalidates
    /// measurement; application state and stable view keys remain untouched.
    pub fn set_theme(&mut self, theme: crate::Theme, font: Option<Font>) {
        if let Some(font) = font {
            self.font = font;
        }
        self.set_style(theme.cascade, theme.background);
    }
    /// A theme change invalidates the whole canvas on the next render.
    pub fn set_style(&mut self, cascade: Cascade, background: Color) {
        self.cascade = cascade;
        self.background = background;
        self.previous = None;
        self.motion.clear();
    }
    /// Stop sampling. The next render settles to the current target values.
    pub fn cancel_motion(&mut self) {
        self.motion.clear();
    }
    pub fn is_animating(&self) -> bool {
        self.motion.active() > 0
    }
    /// Disable transitions or allow future timed changes. The next render
    /// settles disabled motion to its target, preserving application state.
    pub fn set_motion_enabled(&mut self, enabled: bool) {
        self.motion_enabled = enabled;
        if !enabled {
            self.motion.clear();
        }
    }
    pub fn render<Msg>(
        &mut self,
        view: &View<Msg>,
        width: u32,
        height: u32,
    ) -> Result<RenderStats> {
        self.render_inner(view, width, height, None)
    }
    /// Render using an explicit monotonic clock. Immediate `render` remains
    /// deterministic and settles all properties to their targets.
    pub fn render_at<Msg>(
        &mut self,
        view: &View<Msg>,
        width: u32,
        height: u32,
        now_ms: u64,
    ) -> Result<RenderStats> {
        self.render_inner(view, width, height, Some(now_ms))
    }
    fn render_inner<Msg>(
        &mut self,
        view: &View<Msg>,
        width: u32,
        height: u32,
        now: Option<u64>,
    ) -> Result<RenderStats> {
        if width == 0 || height == 0 {
            return Err(UiError::InvalidSize);
        }
        let tree = view.tree()?;
        let mut views = Vec::new();
        view.flatten(&mut views);
        let targets: Vec<_> = tree
            .iter()
            .zip(views)
            .map(|(n, v)| {
                let classes: Vec<_> = v.classes.iter().map(|c| c.as_str()).collect();
                self.cascade.resolve(&NodeStyle {
                    element: n.kind.tag(),
                    id: n.name.as_deref(),
                    classes: &classes,
                    states: &[],
                })
            })
            .collect();
        let resized = self.size != (width, height);
        if resized {
            self.motion.clear();
        }
        let old = self
            .previous
            .as_ref()
            .filter(|_| !resized)
            .map(|p| (&p.tree, &p.targets[..], &p.styles[..]));
        let styles = self
            .motion
            .styles(&tree, &targets, old, now, self.motion_enabled);
        let tree = container_padding(&tree, &styles)?;
        let fm = FontMeasure { font: &self.font };
        let layout = layout_measured(&tree, width as f32, height as f32, &|id| {
            let n = tree.node(id);
            Widget::from_tag(n.kind.tag())
                .map(|w| intrinsic_size(w, n.text.as_deref(), &styles[id.0 as usize], &fm))
                .unwrap_or((0.0, 0.0))
        });
        if tree.iter().any(|n| {
            let r = layout.rect(n.id);
            [r.x, r.y, r.w, r.h].iter().any(|v| !v.is_finite()) || r.w < 0.0 || r.h < 0.0
        }) {
            return Err(UiError::InvalidLayout);
        }
        let next = Snapshot {
            tree,
            layout,
            styles,
            targets,
        };
        let full = Damage {
            x: 0,
            y: 0,
            width,
            height,
        };
        let mut damage = match self.previous.as_ref().filter(|_| !resized) {
            Some(old) => diff(old, &next, width, height),
            None => Some(full),
        };
        // Blur reads previously painted neighbours. Rebuild the scene whenever
        // anything changes until a dependency-aware filter-damage planner exists.
        if damage.is_some()
            && next
                .styles
                .iter()
                .chain(self.previous.iter().flat_map(|p| p.styles.iter()))
                .any(|s| {
                    s.number("backdrop-blur")
                        .is_some_and(|r| r >= 0.5 && r <= 64.)
                })
        {
            damage = Some(full);
        }
        if resized {
            let len = (width as usize)
                .checked_mul(height as usize)
                .ok_or(UiError::InvalidSize)?;
            let mut pixels = Vec::new();
            pixels
                .try_reserve_exact(len)
                .map_err(|_| UiError::Allocation)?;
            pixels.resize(len, 0);
            self.pixels = pixels;
            self.size = (width, height);
        }
        if let Some(damage) = damage {
            let mut surface = Surface::new(&mut self.pixels, width as usize, height as usize);
            let rect = damage.rect();
            surface.with_clip(rect, |s| s.clear(self.background));
            let painted = paint_clipped(
                &next.tree,
                &next.layout,
                &|id| next.styles[id.0 as usize].clone(),
                &self.font,
                &mut surface,
                rect,
            );
            if painted.is_err() {
                // A scratch failure may leave a partially painted private canvas.
                // Never diff against it or publish it as a completed frame.
                self.previous = None;
                self.motion.clear();
                return Err(UiError::Allocation);
            }
        }
        self.previous = Some(next);
        Ok(RenderStats {
            damaged_pixels: damage.map_or(0, |d| d.width as u64 * d.height as u64),
            damage,
            active_transitions: self.motion.active(),
            animation_time_ms: now,
        })
    }
}

fn container_padding(tree: &Tree, styles: &[Style]) -> Result<Tree> {
    let mut builder = TreeBuilder::new();
    for n in tree.iter() {
        let mut attrs = n.attrs.clone();
        if n.kind.is_container() {
            if let Some([top, right, bottom, left]) = styles[n.id.0 as usize].padding() {
                if [top, right, bottom, left]
                    .iter()
                    .any(|n| !n.is_finite() || *n < 0.0)
                {
                    return Err(UiError::InvalidLayout);
                }
                attrs.padding = Edges {
                    top,
                    right,
                    bottom,
                    left,
                };
            }
        }
        let id = builder.push(
            n.name.clone(),
            n.kind.clone(),
            attrs,
            n.text.clone(),
            n.parent,
        );
        if let Some(parent) = n.parent {
            builder.add_child(parent, id);
        }
    }
    Ok(builder.finish(tree.root()))
}

fn diff(old: &Snapshot, new: &Snapshot, width: u32, height: u32) -> Option<Damage> {
    let full = Damage {
        x: 0,
        y: 0,
        width,
        height,
    };
    // Structural edits/reordering can change stacking; invalidate the scene.
    if old.tree.len() != new.tree.len()
        || old.tree.iter().zip(new.tree.iter()).any(|(a, b)| {
            a.name != b.name || a.kind != b.kind || a.parent != b.parent || a.children != b.children
        })
    {
        return Some(full);
    }
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (a, b) in old.tree.iter().zip(new.tree.iter()) {
        let ra = old.layout.rect(a.id);
        let rb = new.layout.rect(b.id);
        if a.text == b.text
            && a.attrs == b.attrs
            && ra == rb
            && old.styles[a.id.0 as usize] == new.styles[b.id.0 as usize]
        {
            continue;
        }
        for r in [
            dunit_render::visual_bounds(ra, &old.styles[a.id.0 as usize]),
            dunit_render::visual_bounds(rb, &new.styles[b.id.0 as usize]),
        ] {
            // Outward rounding also contains the painter's nearest-pixel bounds.
            let x0 = r.x.max(0.0).min(width as f32) as u32;
            let y0 = r.y.max(0.0).min(height as f32) as u32;
            let x1 = ceil_clamped(r.x + r.w, width);
            let y1 = ceil_clamped(r.y + r.h, height);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            bounds = Some(match bounds {
                Some((x, y, xx, yy)) => (x.min(x0), y.min(y0), xx.max(x1), yy.max(y1)),
                None => (x0, y0, x1, y1),
            });
        }
    }
    bounds.map(|(x, y, xx, yy)| Damage {
        x,
        y,
        width: xx - x,
        height: yy - y,
    })
}
fn ceil_clamped(value: f32, max: u32) -> u32 {
    let v = value.max(0.0).min(max as f32);
    let n = v as u32;
    if n < max && n as f32 != v {
        n + 1
    } else {
        n
    }
}
