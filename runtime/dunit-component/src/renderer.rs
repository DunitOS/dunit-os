use crate::{Color, Result, UiError, View};
use alloc::{collections::BTreeMap, string::String, vec::Vec};
use dunit_render::{paint_clipped_with_lists, ScrollArea, Surface, TextInput};
#[path = "collection_layout.rs"]
mod collection_layout;
#[path = "input_click_style.rs"]
mod input_click_style;
#[path = "input_scroll.rs"]
mod input_scroll;
#[path = "input_visual.rs"]
mod input_visual;
#[path = "list_render.rs"]
mod list_render;
#[path = "overlays.rs"]
mod overlays;
#[path = "progress_render.rs"]
mod progress_render;
#[path = "scroll.rs"]
mod scroll;
#[path = "scroll_visibility.rs"]
mod scroll_visibility;
#[path = "workspace_layout.rs"]
mod workspace_layout;
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
    /// Visible moving busy indicators that currently need timed sampling.
    pub active_busy_indicators: usize,
    /// Caller-supplied monotonic milliseconds; None for immediate rendering.
    pub animation_time_ms: Option<u64>,
}
struct Snapshot {
    lists: Vec<Option<dunit_render::ListPaint>>,
    list_current: Vec<Option<(String, usize)>>,
    overlays: overlays::Scene,
    modal_owner: Option<String>,
    popup_owner: Option<String>,
    splitters: Vec<Option<(i64, crate::SplitAxis)>>,
    tree: Tree,
    layout: Layout,
    styles: Vec<Style>,
    targets: Vec<Style>,
    inputs: Vec<Option<TextInput>>,
    sliders: Vec<Option<f32>>,
    progress: Vec<Option<dunit_widgets::progress::ProgressPaint>>,
    busy_interval_ms: Option<u64>,
    input_activity: Vec<u64>,
    input_axes: Vec<Option<(dunit_widgets::ScrollAxis, dunit_widgets::ScrollAxis)>>,
    input_scroll_axes: Vec<Option<(dunit_widgets::ScrollAxis, dunit_widgets::ScrollAxis)>>,
    areas: Vec<Option<ScrollArea>>,
    focused_key: Option<String>,
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
    scroll_axes: BTreeMap<String, dunit_widgets::ScrollAxis>,
    scroll_capture: Option<(String, crate::ScrollDirection, f32)>,
    scroll_axes_x: BTreeMap<String, dunit_widgets::ScrollAxis>,
    input_offsets: BTreeMap<String, (f32, f32)>,
    scroll_focus: Option<String>,
    scroll_sizes: BTreeMap<String, (f32, f32)>,
    scroll_clock: Option<u64>,
    scroll_visibility: BTreeMap<String, scroll_visibility::Visibility>,
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
            scroll_axes: BTreeMap::new(),
            scroll_capture: None,
            scroll_axes_x: BTreeMap::new(),
            input_offsets: BTreeMap::new(),
            scroll_focus: None,
            scroll_sizes: BTreeMap::new(),
            scroll_clock: None,
            scroll_visibility: BTreeMap::new(),
        }
    }
    pub fn slider_geometry(&self, key: &str) -> Option<crate::SliderGeometry> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        Some(crate::SliderGeometry::oriented(
            p.layout.rect(id),
            &p.styles[id.0 as usize],
            p.sliders[id.0 as usize]?,
            crate::SliderOrientation::from_tag(p.tree.node(id).kind.tag()),
        ))
    }
    pub fn spinbox_button_at(&self, key: &str, x: f32, y: f32) -> Option<crate::SpinButton> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        p.inputs[id.0 as usize].as_ref()?.spinbox_button_at(
            &p.styles[id.0 as usize],
            p.layout.rect(id),
            x,
            y,
        )
    }
    pub fn scroll_capture_key(&self) -> Option<&str> {
        self.scroll_capture.as_ref().map(|c| c.0.as_str())
    }
    pub fn cancel_scroll_capture(&mut self) -> bool {
        self.scroll_capture.take().is_some()
    }
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }
    /// Stable key of the topmost node under the pointer in the last painted
    /// layout. Ancestor viewport clipping uses the shared widget hit tester.
    pub fn hit_key(&self, x: f32, y: f32) -> Option<&str> {
        let snapshot = self.previous.as_ref()?;
        let id = scroll::hit(snapshot, x, y)?;
        snapshot.tree.node(id).name.as_deref()
    }
    /// UTF-8 scalar boundary under a pointer, using the last painted viewport.
    pub fn input_index(&self, key: &str, x: f32) -> Option<usize> {
        let snapshot = self.previous.as_ref()?;
        let id = snapshot.tree.by_name(key)?;
        let input = snapshot.inputs[id.0 as usize].as_ref()?;
        Some(input.hit_index(
            &self.font,
            &snapshot.styles[id.0 as usize],
            snapshot.layout.rect(id),
            x,
        ))
    }
    pub fn input_index_at(&self, key: &str, x: f32, y: f32) -> Option<usize> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        let input = p.inputs[id.0 as usize].as_ref()?;
        Some(input.hit_index_at(
            &self.font,
            &p.styles[id.0 as usize],
            p.layout.rect(id),
            x,
            y,
        ))
    }
    pub fn input_scroll_y(&self, key: &str) -> Option<f32> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        p.inputs[id.0 as usize].as_ref().map(|i| i.scroll_y)
    }
    /// Queue a manual viewport change. The previous painted snapshot stays
    /// immutable until diff/paint have consumed the request successfully.
    pub fn scroll_input(&mut self, key: &str, lines: f32) -> bool {
        if !lines.is_finite() || lines == 0. {
            return false;
        }
        let Some(p) = self.previous.as_ref() else {
            return false;
        };
        let Some(id) = p.tree.by_name(key) else {
            return false;
        };
        if p.input_axes[id.0 as usize].is_none() {
            return false;
        }
        let height = self
            .font
            .line_metrics(p.styles[id.0 as usize].number("font-size").unwrap_or(0.))
            .line_height();
        let pixels =
            (lines as f64 * height as f64).clamp(-(f32::MAX as f64), f32::MAX as f64) as f32;
        // Line, wheel and bar requests compose in one retained controller.
        self.scroll_by(key, pixels)
    }
    /// Native wheel convention: negative upward, positive downward.
    pub fn scroll_input_wheel(&mut self, key: &str, notches: i32) -> bool {
        let Some(step) = self.previous.as_ref().and_then(|p| {
            p.tree
                .by_name(key)
                .map(|id| p.styles[id.0 as usize].number("scroll-step").unwrap_or(0.))
        }) else {
            return false;
        };
        self.scroll_input(key, notches as f32 * step)
    }
    pub fn input_scroll_x(&self, key: &str) -> Option<f32> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        p.inputs[id.0 as usize].as_ref().map(|i| i.scroll_x)
    }
    /// Zero duration disables blinking; absence of focused input needs no timer.
    pub fn caret_blink_ms(&self) -> Option<u64> {
        let p = self.previous.as_ref()?;
        p.inputs
            .iter()
            .enumerate()
            .filter(|(_, i)| i.as_ref().is_some_and(|i| i.focused))
            .filter_map(|(n, _)| p.styles[n].get("caret-blink").and_then(|v| v.as_duration()))
            .filter(|ms| *ms > 0.)
            .map(|ms| (ms as u64).max(1))
            .min()
    }
    /// Apply a fully validated candidate. A font-path change also invalidates
    /// measurement; application state and stable view keys remain untouched.
    /// Preflight a candidate against the actual view without changing the
    /// published canvas, hit geometry, scroll state or active transitions.
    /// Reload is rare; a temporary renderer also validates final clipped geometry.
    pub fn validate_theme_application<C: crate::Component>(
        &self,
        app: &crate::Application<C>,
        theme: &crate::Theme,
        font: Option<&Font>,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let mut candidate = Self::new(
            font.unwrap_or(&self.font).clone(),
            theme.cascade.clone(),
            theme.background,
        );
        candidate.set_motion_enabled(false);
        candidate.render_application(app, width, height).map(|_| ())
    }
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
        if let Some(p) = &mut self.previous {
            p.busy_interval_ms = None;
        }
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
        self.render_inner(view, width, height, None, None)
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
        self.render_inner(view, width, height, Some(now_ms), None)
    }
    /// Render the application including active menus and delayed tooltips.
    pub fn render_application<C: crate::Component>(
        &mut self,
        app: &crate::Application<C>,
        width: u32,
        height: u32,
    ) -> Result<RenderStats> {
        self.render_inner(app.view(), width, height, None, Some(&app.overlays))
    }
    pub fn render_application_at<C: crate::Component>(
        &mut self,
        app: &crate::Application<C>,
        width: u32,
        height: u32,
        now: u64,
    ) -> Result<RenderStats> {
        self.render_inner(app.view(), width, height, Some(now), Some(&app.overlays))
    }
    fn render_inner<Msg>(
        &mut self,
        view: &View<Msg>,
        width: u32,
        height: u32,
        now: Option<u64>,
        overlays: Option<&crate::popup::OverlayState>,
    ) -> Result<RenderStats> {
        if width == 0 || height == 0 {
            return Err(UiError::InvalidSize);
        }
        let tree = view.tree()?;
        let mut views = Vec::new();
        view.flatten(&mut views);
        let modal_owner = views
            .iter()
            .find(|v| v.dialog.is_some())
            .map(|v| v.key.clone());
        let popup_owner = overlays
            .and_then(|s| s.open.as_ref())
            .map(|p| p.owner.clone());
        if self
            .previous
            .as_ref()
            .is_some_and(|p| p.modal_owner != modal_owner || p.popup_owner != popup_owner)
        {
            self.cancel_scroll_capture();
        }

        let targets: Vec<_> = tree
            .iter()
            .zip(&views)
            .map(|(n, v)| {
                let classes: Vec<_> = v.classes.iter().map(|c| c.as_str()).collect();
                let mut states = v.interaction.state_names();
                if overlays
                    .and_then(|s| s.open.as_ref())
                    .is_some_and(|p| p.owner == v.key)
                {
                    states.push("expanded");
                }
                if v.input.as_ref().is_some_and(|i| i.options.read_only) {
                    states.push("read-only");
                }
                self.cascade.resolve(&NodeStyle {
                    element: n.kind.tag(),
                    id: n.name.as_deref(),
                    classes: &classes,
                    states: &states,
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
        let tree = container_padding(&tree, &styles, &views)?;
        let measure = |id| measure_widget(&self.font, &tree, &styles, &views, id);
        let mut layout = layout_measured(&tree, width as f32, height as f32, &measure);
        self.layout_splitters(&tree, &mut layout, &styles, &views, &measure)?;
        if tree.iter().any(|n| {
            let r = layout.rect(n.id);
            [r.x, r.y, r.w, r.h].iter().any(|v| !v.is_finite()) || r.w < 0.0 || r.h < 0.0
        }) {
            return Err(UiError::InvalidLayout);
        }
        let mut areas = self.prepare_scroll(&tree, &mut layout, &styles, &views)?;
        let measure_after = |id| measure_widget(&self.font, &tree, &styles, &views, id);
        self.layout_collection_editors(
            &tree,
            &mut layout,
            &styles,
            &views,
            &areas,
            &measure_after,
        )?;
        let inputs: Vec<Option<TextInput>> = tree
            .iter()
            .map(|n| {
                let view = views[n.id.0 as usize];
                view.input.as_ref().map(|input| {
                    let previous = self.previous.as_ref().and_then(|p| {
                        p.tree
                            .by_name(&view.key)
                            .and_then(|id| p.inputs[id.0 as usize].as_ref())
                    });
                    let mut paint = TextInput {
                        spinbox: view.spinbox.as_ref().map(|s| {
                            let base = s.draft_value(input.editor.text());
                            let enabled = !s.options.read_only && !view.interaction.disabled;
                            dunit_render::SpinboxPaint {
                                increase: enabled
                                    && s.options
                                        .range
                                        .command(base, crate::SliderCommand::Increase)
                                        != base,
                                decrease: enabled
                                    && s.options
                                        .range
                                        .command(base, crate::SliderCommand::Decrease)
                                        != base,
                                pressed: if view.interaction.pressed {
                                    s.pressed
                                } else {
                                    None
                                },
                            }
                        }),
                        value: input.editor.text().into(),
                        placeholder: input.options.placeholder.clone(),
                        password: input.options.password,
                        soft_wrap: input.options.soft_wrap,
                        affinity: input.affinity,
                        cursor: input.editor.cursor(),
                        anchor: input.editor.anchor(),
                        focused: view.interaction.focused && !view.interaction.disabled,
                        caret_visible: input.caret_visible
                            || styles[n.id.0 as usize]
                                .get("caret-blink")
                                .and_then(|v| v.as_duration())
                                .map_or(true, |v| v == 0.),
                        scroll_x: previous.map_or(0., |p| p.scroll_x),
                        multiline: input.editor.is_multiline(),
                        scroll_y: previous.map_or(0., |p| p.scroll_y),
                    };
                    let rect = layout.rect(n.id);
                    let geometry_changed = self
                        .previous
                        .as_ref()
                        .and_then(|p| {
                            p.tree.by_name(&view.key).map(|id| {
                                p.layout.rect(id) != rect
                                    || [
                                        "font-size",
                                        "padding",
                                        "border-width",
                                        "caret-width",
                                        "scrollbar-width",
                                    ]
                                    .iter()
                                    .any(|property| {
                                        p.styles[id.0 as usize].get(property)
                                            != styles[n.id.0 as usize].get(property)
                                    })
                            })
                        })
                        .unwrap_or(true);
                    let activity_changed = self
                        .previous
                        .as_ref()
                        .and_then(|p| {
                            p.tree
                                .by_name(&view.key)
                                .map(|id| p.input_activity[id.0 as usize] != input.caret_revision)
                        })
                        .unwrap_or(true);
                    let caret_changed = previous.map_or(true, |p| {
                        p.value != paint.value
                            || p.cursor != paint.cursor
                            || p.anchor != paint.anchor
                            || p.multiline != paint.multiline
                            || p.soft_wrap != paint.soft_wrap
                            || p.affinity != paint.affinity
                            || p.password != paint.password
                            || (!p.focused && paint.focused)
                    });
                    if geometry_changed || caret_changed || activity_changed || !paint.multiline {
                        paint.fit_scroll(&self.font, &styles[n.id.0 as usize], rect);
                    }
                    if !view.interaction.disabled {
                        if let Some(&(x, y)) = self.input_offsets.get(&view.key) {
                            let (mut ax, mut ay) =
                                paint.scroll_axes(&self.font, &styles[n.id.0 as usize], rect);
                            ax.move_by(x - ax.offset);
                            ay.move_by(y - ay.offset);
                            paint.scroll_x = ax.offset;
                            paint.scroll_y = ay.offset;
                        }
                    }
                    paint
                })
            })
            .collect();
        // Measure final painted input extents once; bars and selection timers
        // share these values. input_axes retains enabled-Textarea semantics.
        let input_scroll_axes: Vec<_> = tree
            .iter()
            .map(|n| {
                inputs[n.id.0 as usize].as_ref().map(|input| {
                    input.scroll_axes(&self.font, &styles[n.id.0 as usize], layout.rect(n.id))
                })
            })
            .collect();
        let input_axes = tree
            .iter()
            .map(|n| {
                let index = n.id.0 as usize;
                inputs[index]
                    .as_ref()
                    .filter(|input| input.multiline)
                    .and_then(|input| {
                        let (x, y) = input_scroll_axes[index]?;
                        let enabled = !views[index].interaction.disabled;
                        areas[index] = Some(ScrollArea::from_axes(
                            input.content_rect(&styles[index], layout.rect(n.id)),
                            &styles[index],
                            enabled.then_some(&x),
                            enabled.then_some(&y),
                        ));
                        enabled.then_some((x, y))
                    })
            })
            .collect();
        self.validate_scroll_capture(&tree, &areas);
        self.prepare_scroll_visibility(&tree, &styles, &mut areas, &views, now);
        let input_activity = views
            .iter()
            .map(|v| v.input.as_ref().map_or(0, |i| i.caret_revision))
            .collect();
        let sliders = views
            .iter()
            .map(|v| v.slider.as_ref().map(|s| s.options.fraction(s.value)))
            .collect();
        let (progress, busy_interval_ms, active_busy_indicators) =
            self.prepare_progress(&views, &tree, &layout, &styles, &areas, now, width, height);
        let overlay_scene = self.overlays(view, &tree, &layout, overlays, &areas, width, height);
        let splitters = views
            .iter()
            .map(|v| v.split.as_ref().map(|s| (s.value, s.options.axis)))
            .collect();
        let lists = self.prepare_lists(&tree, &layout, &styles, &views, &areas, width, height);
        let list_current = views
            .iter()
            .map(|v| {
                v.list.as_ref().and_then(|l| {
                    l.selection.current.as_ref().and_then(|key| {
                        l.items
                            .iter()
                            .position(|i| &i.key == key)
                            .map(|index| (key.clone(), index))
                    })
                })
            })
            .collect();
        let next = Snapshot {
            lists,
            list_current,
            modal_owner,
            popup_owner,
            splitters,
            overlays: overlay_scene,
            progress,
            busy_interval_ms,
            sliders,
            focused_key: views
                .iter()
                .find(|v| v.interaction.focused && !v.interaction.disabled)
                .map(|v| v.key.clone()),
            areas,
            input_axes,
            input_scroll_axes,
            input_activity,
            inputs,
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
        if let Some(dirty) = damage {
            // A retained final canvas is not a valid pre-filter backdrop.
            // Rebuild when damage touches any filter output or sampled halo.
            // Disjoint edits cannot affect a filter and need no blur scratch.
            if next.overlays.requires_backdrop_rebuild()
                || self
                    .previous
                    .as_ref()
                    .is_some_and(|p| p.overlays.requires_backdrop_rebuild())
                || touches_backdrop(&next, dirty)
                || self
                    .previous
                    .as_ref()
                    .is_some_and(|p| touches_backdrop(p, dirty))
            {
                damage = Some(full);
            }
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
            let painted = paint_clipped_with_lists(
                &next.tree,
                &next.layout,
                &|id| next.styles[id.0 as usize].clone(),
                &self.font,
                &mut surface,
                rect,
                &|id| next.inputs[id.0 as usize].as_ref(),
                &|id| next.areas[id.0 as usize].as_ref(),
                &|id| next.sliders[id.0 as usize],
                &|id| next.progress[id.0 as usize],
                &|id| next.lists[id.0 as usize].as_ref(),
            );
            let painted =
                painted.and_then(|()| next.overlays.paint(&self.font, &mut surface, rect));
            if painted.is_err() {
                // A scratch failure may leave a partially painted private canvas.
                // Never diff against it or publish it as a completed frame.
                self.previous = None;
                self.motion.clear();
                return Err(UiError::Allocation);
            }
        }
        self.scroll_focus = next.focused_key.clone();
        self.previous = Some(next);
        self.input_offsets.clear();
        Ok(RenderStats {
            damaged_pixels: damage.map_or(0, |d| d.width as u64 * d.height as u64),
            damage,
            active_transitions: self.motion.active(),
            active_busy_indicators,
            animation_time_ms: now,
        })
    }
}

fn touches_backdrop(snapshot: &Snapshot, dirty: Damage) -> bool {
    snapshot.tree.iter().any(|node| {
        let support = dunit_render::blur::backdrop_support(
            snapshot.styles[node.id.0 as usize]
                .number("backdrop-blur")
                .unwrap_or(0.),
        );
        if support == 0 {
            return false;
        }
        let rect = snapshot.layout.rect(node.id);
        // Enclose nearest-pixel output rounding. Ancestor clips/rounded masks
        // may reduce this dependency, never enlarge it; using the whole box
        // deliberately keeps nested and overlapping filters conservative.
        let halo = support as f32 + 1.;
        (dirty.x as f32) < rect.x + rect.w + halo
            && (dirty.y as f32) < rect.y + rect.h + halo
            && ((dirty.x + dirty.width) as f32) > rect.x - halo
            && ((dirty.y + dirty.height) as f32) > rect.y - halo
    })
}

fn container_padding<M>(tree: &Tree, styles: &[Style], views: &[&View<M>]) -> Result<Tree> {
    let mut builder = TreeBuilder::new();
    for n in tree.iter() {
        let mut attrs = n.attrs.clone();
        crate::layout_style::apply(&mut attrs, &styles[n.id.0 as usize])?;
        if n.kind.tag() == "ToolbarSeparator" {
            attrs.width = dunit_ui::Sizing::Px(
                styles[n.id.0 as usize]
                    .number("separator-thickness")
                    .unwrap_or(0.),
            );
            attrs.height = dunit_ui::Sizing::Grow(1.);
        }
        if views[n.id.0 as usize]
            .classes
            .iter()
            .any(|c| c == "message-body")
        {
            if let Some(height) = styles[n.id.0 as usize].number("message-body-height") {
                attrs.height = dunit_ui::Sizing::Px(height);
            }
        }
        if views[n.id.0 as usize].dialog.is_some() && attrs.width == dunit_ui::Sizing::Auto {
            if let Some(width) = styles[n.id.0 as usize].number("dialog-width") {
                attrs.width = dunit_ui::Sizing::Px(width);
            }
        }

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
        if views[n.id.0 as usize].list.is_some() {
            let style = &styles[n.id.0 as usize];
            let border = style.number("border-width").unwrap_or(0.);
            attrs.padding = Edges {
                top: border,
                bottom: border
                    + if views[n.id.0 as usize].scroll_axes.horizontal() {
                        style.number("scrollbar-width").unwrap_or(0.)
                    } else {
                        0.
                    },
                left: border,
                right: border + style.number("scrollbar-width").unwrap_or(0.),
            };
        }
        if views[n.id.0 as usize].table.is_some() {
            attrs.padding.top += styles[n.id.0 as usize]
                .number("table-header-height")
                .ok_or(UiError::InvalidLayout)?;
        }
        if n.kind == dunit_ui::Kind::Scroll {
            let style = &styles[n.id.0 as usize];
            let border = style.number("border-width").unwrap_or(0.);
            attrs.padding.top += border;
            attrs.padding.bottom += border;
            attrs.padding.left += border;
            let axes = views[n.id.0 as usize].scroll_axes;
            let width = style.number("scrollbar-width").unwrap_or(0.);
            attrs.padding.right += border + if axes.vertical() { width } else { 0. };
            attrs.padding.bottom += if axes.horizontal() { width } else { 0. };
        }
        let id = builder.push(
            n.name.clone(),
            n.kind.clone(),
            attrs,
            if let Some(suffix) = styles[n.id.0 as usize].keyword("control-suffix") {
                n.text.as_ref().map(|t| alloc::format!("{} {}", t, suffix))
            } else {
                n.text.clone()
            },
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
    let mut rectangles = Vec::new();
    if old.overlays != new.overlays {
        rectangles.extend(old.overlays.bounds());
        rectangles.extend(new.overlays.bounds());
    }
    for (a, b) in old.tree.iter().zip(new.tree.iter()) {
        let ra = old.layout.rect(a.id);
        let rb = new.layout.rect(b.id);
        if a.text == b.text
            && a.attrs == b.attrs
            && ra == rb
            && old.styles[a.id.0 as usize] == new.styles[b.id.0 as usize]
            && old.lists[a.id.0 as usize] == new.lists[b.id.0 as usize]
            && old.inputs[a.id.0 as usize] == new.inputs[b.id.0 as usize]
            && old.sliders[a.id.0 as usize] == new.sliders[b.id.0 as usize]
            && old.progress[a.id.0 as usize] == new.progress[b.id.0 as usize]
            && old.areas[a.id.0 as usize] == new.areas[b.id.0 as usize]
        {
            continue;
        }
        rectangles.extend([
            dunit_render::visual_bounds(ra, &old.styles[a.id.0 as usize]),
            dunit_render::visual_bounds(rb, &new.styles[b.id.0 as usize]),
        ]);
    }
    for r in rectangles {
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

/// Effective rectangular clip shared by timed and virtualized content.
fn visible_rect(
    tree: &Tree,
    layout: &Layout,
    areas: &[Option<ScrollArea>],
    id: dunit_ui::NodeId,
    rect: Rect,
    width: u32,
    height: u32,
) -> Option<Rect> {
    let root = layout.rect(tree.root());
    let mut left = rect.x.max(root.x).max(0.);
    let mut top = rect.y.max(root.y).max(0.);
    let mut right = (rect.x + rect.w).min(root.x + root.w).min(width as f32);
    let mut bottom = (rect.y + rect.h).min(root.y + root.h).min(height as f32);
    let mut parent = tree.node(id).parent;
    while let Some(id) = parent {
        let clip = areas[id.0 as usize]
            .as_ref()
            .map_or(layout.rect(id), |a| a.viewport);
        left = left.max(clip.x);
        top = top.max(clip.y);
        right = right.min(clip.x + clip.w);
        bottom = bottom.min(clip.y + clip.h);
        parent = tree.node(id).parent;
    }
    (left < right && top < bottom).then_some(Rect {
        x: left,
        y: top,
        w: (right - left).max(0.),
        h: (bottom - top).max(0.),
    })
}

fn measure_widget<M>(
    font: &Font,
    tree: &Tree,
    styles: &[Style],
    views: &[&View<M>],
    id: dunit_ui::NodeId,
) -> (f32, f32) {
    let n = tree.node(id);
    Widget::from_tag(n.kind.tag())
        .map(|w| {
            intrinsic_size(
                w,
                if views[id.0 as usize].input.is_some() {
                    None
                } else {
                    n.text.as_deref()
                },
                &styles[id.0 as usize],
                &FontMeasure { font },
            )
        })
        .unwrap_or((0., 0.))
}
