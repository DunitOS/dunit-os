//! Retained viewport geometry and routing; native adapters supply coordinates.
use super::{Renderer, Snapshot};
use crate::{PointerEvent, Result, ScrollDirection, UiError, View};
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use dunit_render::ScrollArea;
use dunit_style::Style;
use dunit_ui::{Kind, Layout, NodeId, Rect, Tree};
use dunit_widgets::{ScrollAxis, ScrollCommand};

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
fn viewport(tree: &Tree, layout: &Layout, id: NodeId) -> Rect {
    let rect = layout.rect(id);
    let pad = tree.node(id).attrs.padding;
    Rect {
        x: rect.x + pad.left,
        y: rect.y + pad.top,
        w: (rect.w - pad.horizontal()).max(0.),
        h: (rect.h - pad.vertical()).max(0.),
    }
}
fn bar(area: &ScrollArea, direction: ScrollDirection) -> (Option<Rect>, Option<Rect>) {
    match direction {
        ScrollDirection::Horizontal => (area.horizontal_track, area.horizontal_thumb),
        ScrollDirection::Vertical => (area.track, area.thumb),
    }
}
impl Renderer {
    /// Read the current controller without changing the last painted snapshot.
    /// Input controllers overlay pending absolute offsets on their painted extents.
    fn scroll_axis(&self, key: &str, direction: ScrollDirection) -> Option<ScrollAxis> {
        let generic = match direction {
            ScrollDirection::Horizontal => self.scroll_axes_x.get(key),
            ScrollDirection::Vertical => self.scroll_axes.get(key),
        };
        if let Some(axis) = generic {
            return Some(*axis);
        }
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        let (mut x, mut y) = p.input_axes[id.0 as usize]?;
        if let Some(&(offset_x, offset_y)) = self.input_offsets.get(key) {
            x.offset = offset_x;
            x.set_extents(x.viewport, x.content);
            y.offset = offset_y;
            y.set_extents(y.viewport, y.content);
        }
        Some(match direction {
            ScrollDirection::Horizontal => x,
            ScrollDirection::Vertical => y,
        })
    }
    fn set_scroll_axis(&mut self, key: &str, direction: ScrollDirection, axis: ScrollAxis) -> bool {
        let generic = match direction {
            ScrollDirection::Horizontal => self.scroll_axes_x.get_mut(key),
            ScrollDirection::Vertical => self.scroll_axes.get_mut(key),
        };
        if let Some(current) = generic {
            let changed = *current != axis;
            *current = axis;
            return changed;
        }
        let Some(x) = self.scroll_axis(key, ScrollDirection::Horizontal) else {
            return false;
        };
        let Some(y) = self.scroll_axis(key, ScrollDirection::Vertical) else {
            return false;
        };
        let old = (x.offset, y.offset);
        let next = match direction {
            ScrollDirection::Horizontal => (axis.offset, y.offset),
            ScrollDirection::Vertical => (x.offset, axis.offset),
        };
        if next == old {
            return false;
        }
        self.input_offsets.insert(String::from(key), next);
        true
    }
    pub fn scroll_offset(&self, key: &str) -> Option<f32> {
        self.scroll_axis(key, ScrollDirection::Vertical)
            .map(|a| a.offset)
    }
    pub fn scroll_offset_x(&self, key: &str) -> Option<f32> {
        self.scroll_axis(key, ScrollDirection::Horizontal)
            .map(|a| a.offset)
    }
    pub fn scroll_viewport(&self, key: &str) -> Option<Rect> {
        let p = self.previous.as_ref()?;
        p.areas[p.tree.by_name(key)?.0 as usize]
            .as_ref()
            .map(|a| a.viewport)
    }
    fn scroll_bar(
        &self,
        key: &str,
        direction: ScrollDirection,
    ) -> Option<(Option<Rect>, Option<Rect>)> {
        let p = self.previous.as_ref()?;
        Some(bar(
            p.areas[p.tree.by_name(key)?.0 as usize].as_ref()?,
            direction,
        ))
    }
    pub fn scroll_thumb(&self, key: &str) -> Option<Rect> {
        self.scroll_bar(key, ScrollDirection::Vertical)?.1
    }
    pub fn scroll_track(&self, key: &str) -> Option<Rect> {
        self.scroll_bar(key, ScrollDirection::Vertical)?.0
    }
    pub fn scroll_thumb_x(&self, key: &str) -> Option<Rect> {
        self.scroll_bar(key, ScrollDirection::Horizontal)?.1
    }
    pub fn scroll_track_x(&self, key: &str) -> Option<Rect> {
        self.scroll_bar(key, ScrollDirection::Horizontal)?.0
    }
    fn scroll_by_axis(&mut self, key: &str, pixels: f32, direction: ScrollDirection) -> bool {
        let Some(mut axis) = self.scroll_axis(key, direction) else {
            return false;
        };
        if !axis.move_by(pixels) {
            return false;
        }
        self.set_scroll_axis(key, direction, axis)
    }
    /// Pixels in content coordinates. The painted snapshot remains immutable.
    pub fn scroll_by(&mut self, key: &str, pixels: f32) -> bool {
        self.scroll_by_axis(key, pixels, ScrollDirection::Vertical)
    }
    pub fn scroll_by_x(&mut self, key: &str, pixels: f32) -> bool {
        self.scroll_by_axis(key, pixels, ScrollDirection::Horizontal)
    }

    pub(super) fn prepare_scroll<M>(
        &mut self,
        tree: &Tree,
        layout: &mut Layout,
        styles: &[Style],
        views: &[&View<M>],
    ) -> Result<Vec<Option<ScrollArea>>> {
        let mut axes_x = BTreeMap::new();
        let mut axes_y = BTreeMap::new();
        let focused = views
            .iter()
            .position(|v| v.interaction.focused && !v.interaction.disabled)
            .map(|n| NodeId(n as u32));
        let ids: Vec<_> = tree
            .iter()
            .filter(|n| n.kind == Kind::Scroll)
            .map(|n| n.id)
            .collect();
        // Reveal inner viewports before calculating outer descendant coordinates.
        for &id in ids.iter().rev() {
            let node = tree.node(id);
            let view = views[id.0 as usize];
            let key = node.name.as_deref().unwrap_or("");
            let viewport = viewport(tree, layout, id);
            let (mut width, mut height) = (0_f32, 0_f32);
            for &child in &node.children {
                let rect = layout.rect(child);
                let right = rect.x + rect.w - viewport.x;
                let bottom = rect.y + rect.h - viewport.y;
                if !right.is_finite() || !bottom.is_finite() {
                    return Err(UiError::InvalidLayout);
                }
                width = width.max(right);
                height = height.max(bottom);
            }
            let mut x = self.scroll_axes_x.get(key).copied().unwrap_or_default();
            let mut y = self.scroll_axes.get(key).copied().unwrap_or_default();
            x.set_extents(viewport.w, width);
            y.set_extents(viewport.h, height);
            let enabled_x = view.scroll_axes.horizontal() && !view.interaction.disabled;
            let enabled_y = view.scroll_axes.vertical() && !view.interaction.disabled;
            if !enabled_x {
                x.offset = 0.;
            }
            if !enabled_y {
                y.offset = 0.;
            }
            if let Some(focus) = focused.filter(|&f| within(tree, f, id)) {
                let changed = self
                    .scroll_sizes
                    .get(key)
                    .is_none_or(|&size| size != (viewport.w, viewport.h));
                if self.scroll_focus.as_deref() != Some(views[focus.0 as usize].key.as_str())
                    || changed
                {
                    let rect = layout.rect(focus);
                    if enabled_x {
                        x.reveal(rect.x - viewport.x, rect.x + rect.w - viewport.x);
                    }
                    if enabled_y {
                        y.reveal(rect.y - viewport.y, rect.y + rect.h - viewport.y);
                    }
                }
            }
            for &child in &node.children {
                if !layout.translate_subtree(tree, child, -x.offset, -y.offset) {
                    return Err(UiError::InvalidLayout);
                }
            }
            if enabled_x {
                axes_x.insert(String::from(key), x);
            }
            if enabled_y {
                axes_y.insert(String::from(key), y);
            }
        }
        let mut areas = vec![None; tree.len()];
        for id in ids {
            let key = tree.node(id).name.as_deref().unwrap_or("");
            areas[id.0 as usize] = Some(ScrollArea::from_axes(
                viewport(tree, layout, id),
                &styles[id.0 as usize],
                axes_x.get(key),
                axes_y.get(key),
            ));
        }
        self.scroll_sizes = tree
            .iter()
            .filter_map(|n| {
                areas[n.id.0 as usize]
                    .as_ref()
                    .map(|a| (n.name.clone().unwrap(), (a.viewport.w, a.viewport.h)))
            })
            .collect();
        self.scroll_axes_x = axes_x;
        self.scroll_axes = axes_y;
        Ok(areas)
    }
    /// Called after both generic viewport and textarea scrollbar geometry exists.
    pub(super) fn validate_scroll_capture(&mut self, tree: &Tree, areas: &[Option<ScrollArea>]) {
        if self
            .scroll_capture
            .as_ref()
            .is_some_and(|(key, direction, _)| {
                tree.by_name(key).is_none_or(|id| {
                    areas[id.0 as usize]
                        .as_ref()
                        .is_none_or(|a| bar(a, *direction).1.is_none())
                })
            })
        {
            self.scroll_capture = None;
        }
    }

    pub fn scroll_wheel(&mut self, target: Option<&str>, notches: i32) -> bool {
        self.scroll_wheel_axis(target, notches, ScrollDirection::Vertical)
    }
    /// Wheels bubble to a movable ancestor. Input vertical steps remain logical
    /// lines; horizontal input steps use font-size * scroll-step pixels.
    pub fn scroll_wheel_axis(
        &mut self,
        target: Option<&str>,
        notches: i32,
        direction: ScrollDirection,
    ) -> bool {
        if direction == ScrollDirection::Vertical {
            if let Some(key) = target {
                if self.scroll_input_wheel(key, notches) {
                    return true;
                }
            }
        }
        let Some(p) = &self.previous else {
            return false;
        };
        let mut current = target.and_then(|key| p.tree.by_name(key));
        let mut route = Vec::new();
        while let Some(id) = current {
            if let Some(key) = p.tree.node(id).name.as_deref() {
                let style = &p.styles[id.0 as usize];
                let mut step = style.number("scroll-step").unwrap_or(0.);
                let input = p.input_axes[id.0 as usize].is_some();
                if input && direction == ScrollDirection::Horizontal {
                    step *= style.number("font-size").unwrap_or(0.);
                }
                // Vertical input already had its line-based opportunity above.
                if !(input && direction == ScrollDirection::Vertical) {
                    route.push((String::from(key), step));
                }
            }
            current = p.tree.node(id).parent;
        }
        route
            .into_iter()
            .any(|(key, step)| self.scroll_by_axis(&key, notches as f32 * step, direction))
    }
    pub fn scroll_key<M>(&mut self, view: &View<M>, command: ScrollCommand) -> bool {
        self.scroll_key_axis(view, command, ScrollDirection::Vertical)
    }
    pub fn scroll_key_axis<M>(
        &mut self,
        view: &View<M>,
        command: ScrollCommand,
        direction: ScrollDirection,
    ) -> bool {
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
        let mut current = p.tree.by_name(key);
        let mut route = Vec::new();
        while let Some(id) = current {
            if let Some(key) = p.tree.node(id).name.as_deref() {
                let style = &p.styles[id.0 as usize];
                let mut step = style.number("scroll-step").unwrap_or(0.);
                if p.input_axes[id.0 as usize].is_some() {
                    step *= match direction {
                        ScrollDirection::Horizontal => style.number("font-size").unwrap_or(0.),
                        ScrollDirection::Vertical => self
                            .font
                            .line_metrics(style.number("font-size").unwrap_or(0.))
                            .line_height(),
                    };
                }
                route.push((String::from(key), step));
            }
            current = p.tree.node(id).parent;
        }
        for (key, step) in route {
            if let Some(mut axis) = self.scroll_axis(&key, direction) {
                axis.apply(command, step);
                self.set_scroll_axis(&key, direction, axis);
                return true; // consume even at boundaries
            }
        }
        false
    }
    /// Returns the scrollbar target; callers route focus without selecting text.
    pub fn scroll_pointer(&mut self, event: PointerEvent, x: f32, y: f32) -> Option<String> {
        if event == PointerEvent::Leave {
            return self.scroll_capture.take().map(|(key, _, _)| key);
        }
        if let Some((key, direction, grab)) = self.scroll_capture.clone() {
            if event == PointerEvent::Up {
                self.scroll_capture = None;
                return Some(key);
            }
            if event == PointerEvent::Move {
                let (track, _) = self.scroll_bar(&key, direction)?;
                let track = track?;
                let p = self.previous.as_ref()?;
                let id = p.tree.by_name(&key)?;
                let min = p.styles[id.0 as usize]
                    .number("scrollbar-min-thumb")
                    .unwrap_or(0.);
                let (pointer, start, length) = match direction {
                    ScrollDirection::Horizontal => (x, track.x, track.w),
                    ScrollDirection::Vertical => (y, track.y, track.h),
                };
                let mut axis = self.scroll_axis(&key, direction)?;
                axis.drag_thumb(pointer, grab, start, length, min);
                self.set_scroll_axis(&key, direction, axis);
                return Some(key);
            }
        }
        if event != PointerEvent::Down {
            return None;
        }
        let key: String = self.hit_key(x, y)?.into();
        let input_gutter = self.previous.as_ref().is_some_and(|p| {
            let Some(id) = p.tree.by_name(&key) else {
                return false;
            };
            let index = id.0 as usize;
            if p.input_axes[index].is_none() {
                return false;
            }
            let base = dunit_render::input_content_rect(&p.styles[index], p.layout.rect(id));
            contains(base, x, y)
                && p.areas[index]
                    .as_ref()
                    .is_some_and(|a| !contains(a.viewport, x, y))
        });
        // Focus can trigger caret fitting before the first drag Move. Pin both
        // current offsets even for a stationary press so that fitting cannot
        // replace the viewport from which the thumb grab anchor was measured.
        if input_gutter {
            self.pin_input_offsets(&key);
        }
        for direction in [ScrollDirection::Vertical, ScrollDirection::Horizontal] {
            let Some((Some(track), Some(thumb))) = self.scroll_bar(&key, direction) else {
                continue;
            };
            if !contains(track, x, y) {
                continue;
            }
            if !input_gutter {
                self.pin_input_offsets(&key);
            }
            let pointer = match direction {
                ScrollDirection::Horizontal => x,
                ScrollDirection::Vertical => y,
            };
            let leading = match direction {
                ScrollDirection::Horizontal => thumb.x,
                ScrollDirection::Vertical => thumb.y,
            };
            if contains(thumb, x, y) {
                self.scroll_capture = Some((key.clone(), direction, pointer - leading));
            } else {
                let command = if pointer < leading {
                    ScrollCommand::PageUp
                } else {
                    ScrollCommand::PageDown
                };
                let mut axis = self.scroll_axis(&key, direction)?;
                axis.apply(command, 0.);
                self.set_scroll_axis(&key, direction, axis);
            }
            return Some(key);
        }
        input_gutter.then_some(key)
    }

    fn pin_input_offsets(&mut self, key: &str) {
        let input = self.previous.as_ref().and_then(|p| {
            let id = p.tree.by_name(key)?;
            p.input_axes[id.0 as usize]
        });
        if input.is_none() {
            return;
        }
        if let (Some(x), Some(y)) = (
            self.scroll_axis(key, ScrollDirection::Horizontal),
            self.scroll_axis(key, ScrollDirection::Vertical),
        ) {
            self.input_offsets
                .insert(String::from(key), (x.offset, y.offset));
        }
    }
}
