//! Retained viewport geometry and routing; native adapters supply coordinates.
use super::{Renderer, Snapshot};
use crate::{PointerEvent, Result, UiError, View};
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use dunit_render::ScrollArea;
use dunit_style::Style;
use dunit_ui::{Kind, Layout, NodeId, Rect, Tree};
use dunit_widgets::ScrollCommand;

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}
pub(super) fn hit(p: &Snapshot, x: f32, y: f32) -> Option<NodeId> {
    dunit_widgets::hit_test_with_clips(&p.tree, &p.layout, x, y, &|id| {
        p.areas[id.0 as usize].as_ref().map(|area| area.viewport)
    })
}
fn within(tree: &Tree, mut child: NodeId, ancestor: NodeId) -> bool {
    while let Some(parent) = tree.node(child).parent {
        if parent == ancestor {
            return true;
        }
        child = parent;
    }
    false
}
impl Renderer {
    pub fn scroll_offset(&self, key: &str) -> Option<f32> {
        self.scroll_axes.get(key).map(|a| a.offset)
    }
    pub fn scroll_viewport(&self, key: &str) -> Option<Rect> {
        let p = self.previous.as_ref()?;
        p.areas[p.tree.by_name(key)?.0 as usize]
            .as_ref()
            .map(|a| a.viewport)
    }
    pub fn scroll_thumb(&self, key: &str) -> Option<Rect> {
        let p = self.previous.as_ref()?;
        p.areas[p.tree.by_name(key)?.0 as usize].as_ref()?.thumb
    }
    pub fn scroll_track(&self, key: &str) -> Option<Rect> {
        let p = self.previous.as_ref()?;
        p.areas[p.tree.by_name(key)?.0 as usize].as_ref()?.track
    }
    /// Pixels in content coordinates. The painted snapshot remains immutable.
    pub fn scroll_by(&mut self, key: &str, pixels: f32) -> bool {
        self.scroll_axes
            .get_mut(key)
            .is_some_and(|a| a.move_by(pixels))
    }
    pub(super) fn prepare_scroll<M>(
        &mut self,
        tree: &Tree,
        layout: &mut Layout,
        styles: &[Style],
        views: &[&View<M>],
    ) -> Result<Vec<Option<ScrollArea>>> {
        let mut axes = BTreeMap::new();
        let focused = views
            .iter()
            .position(|v| v.interaction.focused && !v.interaction.disabled)
            .map(|n| NodeId(n as u32));
        // Inner viewports reveal their focused child before outer ones do.
        let ids: Vec<_> = tree
            .iter()
            .filter(|n| n.kind == Kind::Scroll)
            .map(|n| n.id)
            .collect();
        for &id in ids.iter().rev() {
            let node = tree.node(id);
            let view = views[id.0 as usize];
            let key = node.name.as_deref().unwrap_or("");
            let rect = layout.rect(id);
            let pad = node.attrs.padding;
            let viewport = Rect {
                x: rect.x + pad.left,
                y: rect.y + pad.top,
                w: (rect.w - pad.horizontal()).max(0.),
                h: (rect.h - pad.vertical()).max(0.),
            };
            let content = node
                .children
                .iter()
                .map(|&c| {
                    let r = layout.rect(c);
                    r.y + r.h - viewport.y
                })
                .fold(0., f32::max);
            if !content.is_finite() {
                return Err(UiError::InvalidLayout);
            }
            let mut axis = self.scroll_axes.get(key).copied().unwrap_or_default();
            axis.set_extents(viewport.h, content);
            if view.interaction.disabled {
                axis.offset = 0.;
            }
            if let Some(focus) = focused.filter(|&f| within(tree, f, id)) {
                let old_focus = self.scroll_focus.as_deref();
                let geometry_changed = self
                    .scroll_sizes
                    .get(key)
                    .is_none_or(|&size| size != (viewport.w, viewport.h));
                if old_focus != Some(views[focus.0 as usize].key.as_str()) || geometry_changed {
                    let r = layout.rect(focus);
                    axis.reveal(r.y - viewport.y, r.y + r.h - viewport.y);
                }
            }
            for &child in &node.children {
                if !layout.translate_subtree(tree, child, 0., -axis.offset) {
                    return Err(UiError::InvalidLayout);
                }
            }
            if !view.interaction.disabled {
                axes.insert(String::from(key), axis);
            }
        }
        let mut areas = vec![None; tree.len()];
        for id in ids {
            let node = tree.node(id);
            let rect = layout.rect(id);
            let pad = node.attrs.padding;
            let viewport = Rect {
                x: rect.x + pad.left,
                y: rect.y + pad.top,
                w: (rect.w - pad.horizontal()).max(0.),
                h: (rect.h - pad.vertical()).max(0.),
            };
            let style = &styles[id.0 as usize];
            let width = style
                .number("scrollbar-width")
                .unwrap_or(0.)
                .min(pad.right)
                .min(rect.w)
                .max(0.);
            let track = Rect {
                x: viewport.x + viewport.w,
                y: viewport.y,
                w: width,
                h: viewport.h,
            };
            let axis = axes.get(node.name.as_deref().unwrap_or(""));
            let visible = axis.is_some_and(|a| a.max_offset() > 0.) && width > 0. && track.h > 0.;
            let thumb = axis
                .and_then(|a| {
                    a.thumb(
                        track.y,
                        track.h,
                        style.number("scrollbar-min-thumb").unwrap_or(0.),
                    )
                })
                .map(|t| Rect {
                    x: track.x,
                    y: t.start,
                    w: track.w,
                    h: t.length,
                });
            areas[id.0 as usize] = Some(ScrollArea {
                viewport,
                track: visible.then_some(track),
                thumb: if visible { thumb } else { None },
            });
        }
        self.scroll_sizes = tree
            .iter()
            .filter_map(|n| {
                areas[n.id.0 as usize]
                    .as_ref()
                    .map(|a| (n.name.clone().unwrap(), (a.viewport.w, a.viewport.h)))
            })
            .collect();
        self.scroll_axes = axes;
        if self.scroll_capture.as_ref().is_some_and(|(key, _)| {
            !self.scroll_axes.contains_key(key)
                || tree.by_name(key).is_none_or(|id| {
                    areas[id.0 as usize]
                        .as_ref()
                        .is_none_or(|a| a.thumb.is_none())
                })
        }) {
            self.scroll_capture = None;
        }
        Ok(areas)
    }
    /// Wheel bubbles from the deepest target to its nearest movable ancestor.
    pub fn scroll_wheel(&mut self, target: Option<&str>, notches: i32) -> bool {
        if let Some(key) = target {
            // Text editing retains its own line-based viewport.
            if self.scroll_input_wheel(key, notches) {
                return true;
            }
        }
        let Some(p) = &self.previous else {
            return false;
        };
        let mut id = target.and_then(|k| p.tree.by_name(k));
        while let Some(node) = id {
            if let Some(key) = p.tree.node(node).name.as_deref() {
                if let Some(axis) = self.scroll_axes.get_mut(key) {
                    let step = p.styles[node.0 as usize]
                        .number("scroll-step")
                        .unwrap_or(0.);
                    if axis.move_by(notches as f32 * step) {
                        return true;
                    }
                }
            }
            id = p.tree.node(node).parent;
        }
        false
    }
    pub fn scroll_key<M>(&mut self, view: &View<M>, command: ScrollCommand) -> bool {
        let mut nodes = Vec::new();
        view.flatten(&mut nodes);
        let Some(key) = nodes
            .iter()
            .find(|v| v.interaction.focused && !v.interaction.disabled)
            .map(|v| v.key.as_str())
        else {
            return false;
        };
        let Some(p) = &self.previous else {
            return false;
        };
        let mut id = p.tree.by_name(key);
        while let Some(node) = id {
            if let Some(key) = p.tree.node(node).name.as_deref() {
                if let Some(axis) = self.scroll_axes.get_mut(key) {
                    let step = p.styles[node.0 as usize]
                        .number("scroll-step")
                        .unwrap_or(0.);
                    axis.apply(command, step);
                    return true; // consumed at boundaries too
                }
            }
            id = p.tree.node(node).parent;
        }
        false
    }
    /// Returns a captured scrollbar key; callers route focus without activating content.
    pub fn scroll_pointer(&mut self, event: PointerEvent, x: f32, y: f32) -> Option<String> {
        if event == PointerEvent::Leave {
            return self.scroll_capture.take().map(|(key, _)| key);
        }
        if let Some((key, grab)) = self.scroll_capture.clone() {
            if event == PointerEvent::Up {
                self.scroll_capture = None;
                return Some(key);
            }
            if event == PointerEvent::Move {
                let track = self.scroll_track(&key)?;
                let p = self.previous.as_ref()?;
                let id = p.tree.by_name(&key)?;
                let min = p.styles[id.0 as usize]
                    .number("scrollbar-min-thumb")
                    .unwrap_or(0.);
                self.scroll_axes
                    .get_mut(&key)?
                    .drag_thumb(y, grab, track.y, track.h, min);
                return Some(key);
            }
        }
        // Uncaptured Move/Up must not steal a text-selection capture.
        if event != PointerEvent::Down {
            return None;
        }
        let key: String = self.hit_key(x, y)?.into();
        let track = self.scroll_track(&key)?;
        if !contains(track, x, y) {
            return None;
        }
        if event == PointerEvent::Down {
            let thumb = self.scroll_thumb(&key)?;
            if contains(thumb, x, y) {
                self.scroll_capture = Some((key.clone(), y - thumb.y));
            } else {
                let command = if y < thumb.y {
                    ScrollCommand::PageUp
                } else {
                    ScrollCommand::PageDown
                };
                self.scroll_axes.get_mut(&key)?.apply(command, 0.);
            }
        }
        Some(key)
    }
}
