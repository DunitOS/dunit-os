//! Selection edge scrolling over retained input geometry. Timers belong to the
//! native adapter; each call advances by one theme-configured step.
use super::Renderer;
use alloc::string::String;
use dunit_render::TextInput;
use dunit_ui::{NodeId, Rect};
use dunit_widgets::ScrollAxis;

struct DragInput<'a> {
    id: NodeId,
    input: &'a TextInput,
    viewport: Rect,
    axes: (ScrollAxis, ScrollAxis),
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.w).min(b.x + b.w);
    let bottom = (a.y + a.h).min(b.y + b.h);
    if ![x, y, right, bottom].iter().all(|v| v.is_finite()) || right <= x || bottom <= y {
        return None;
    }
    Some(Rect {
        x,
        y,
        w: right - x,
        h: bottom - y,
    })
}
fn direction(pointer: f32, start: f32, length: f32) -> f32 {
    if pointer < start {
        -1.
    } else if pointer >= start + length {
        1.
    } else {
        0.
    }
}
fn clamped_point(viewport: Rect, x: f32, y: f32) -> (f32, f32) {
    // Bottom/right are exclusive in painting and hit testing. Stay on the last
    // representable coordinate inside the visible viewport, not the next row.
    (
        x.clamp(
            viewport.x,
            (viewport.x + viewport.w).next_down().max(viewport.x),
        ),
        y.clamp(
            viewport.y,
            (viewport.y + viewport.h).next_down().max(viewport.y),
        ),
    )
}
impl Renderer {
    fn drag_input(&self, key: &str, x: f32, y: f32) -> Option<DragInput<'_>> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        let input = p.inputs[id.0 as usize].as_ref()?;
        let (mut axis_x, mut axis_y) = p.input_scroll_axes[id.0 as usize]?;
        let style = &p.styles[id.0 as usize];
        let rect = p.layout.rect(id);
        let mut viewport = intersect(
            input.content_rect(style, rect),
            p.layout.rect(p.tree.root()),
        )?;
        let mut parent = p.tree.node(id).parent;
        while let Some(ancestor) = parent {
            // Painting clips every ancestor box, including ordinary containers;
            // scroll containers additionally restrict children to their viewport.
            viewport = intersect(viewport, p.layout.rect(ancestor))?;
            if let Some(area) = &p.areas[ancestor.0 as usize] {
                viewport = intersect(viewport, area.viewport)?;
            }
            parent = p.tree.node(ancestor).parent;
        }
        if let Some(&(x, y)) = self.input_offsets.get(key) {
            axis_x.offset = x;
            axis_x.set_extents(axis_x.viewport, axis_x.content);
            axis_y.offset = y;
            axis_y.set_extents(axis_y.viewport, axis_y.content);
        }
        Some(DragInput {
            id,
            input,
            viewport,
            axes: (axis_x, axis_y),
        })
    }

    /// Hit-test at the nearest visible point, including ancestor viewport clips.
    /// Pin the focused field's current viewport before selection placement so
    /// caret fitting on a short line cannot reset the other drag axis.
    /// Unlike the timer methods this also supports the initial unfocused press.
    pub fn input_drag_index_at(&mut self, key: &str, x: f32, y: f32) -> Option<usize> {
        let DragInput {
            id,
            input,
            viewport,
            axes: (axis_x, axis_y),
        } = self.drag_input(key, x, y)?;
        let p = self.previous.as_ref()?;
        let (x, y) = clamped_point(viewport, x, y);
        let adjusted =
            (input.scroll_x != axis_x.offset || input.scroll_y != axis_y.offset).then(|| {
                let mut input = input.clone();
                input.scroll_x = axis_x.offset;
                input.scroll_y = axis_y.offset;
                input
            });
        let input = adjusted.as_ref().unwrap_or(input);
        let index = input.hit_index_at(
            &self.font,
            &p.styles[id.0 as usize],
            p.layout.rect(id),
            x,
            y,
        );
        if input.focused {
            self.input_offsets
                .insert(String::from(key), (axis_x.offset, axis_y.offset));
        }
        Some(index)
    }

    /// Schedule only while a focused enabled painted input can scroll toward
    /// the pointer. Missing/zero interval or step disables selection scrolling.
    pub fn input_autoscroll_ms(&self, key: &str, x: f32, y: f32) -> Option<u64> {
        let DragInput {
            id,
            input,
            viewport,
            axes: (mut axis_x, mut axis_y),
        } = self.drag_input(key, x, y)?;
        if !input.focused {
            return None;
        }
        let p = self.previous.as_ref()?;
        let style = &p.styles[id.0 as usize];
        let interval = style.get("selection-scroll-interval")?.as_duration()?;
        let step = style.number("selection-scroll-step")?;
        if !interval.is_finite() || interval <= 0. || !step.is_finite() || step <= 0. {
            return None;
        }
        let move_x = axis_x.move_by(direction(x, viewport.x, viewport.w) * step);
        let move_y = input.multiline && axis_y.move_by(direction(y, viewport.y, viewport.h) * step);
        (move_x || move_y).then_some((interval as u64).max(1))
    }

    /// Queue one bounded step and return the selection index for the NEW
    /// viewport. The application extends its selection separately; text,
    /// history, painted pixels and painted hit geometry are never changed here.
    pub fn input_autoscroll_step(&mut self, key: &str, x: f32, y: f32) -> Option<usize> {
        self.input_autoscroll_ms(key, x, y)?;
        let DragInput {
            id,
            input,
            viewport,
            axes: (mut axis_x, mut axis_y),
        } = self.drag_input(key, x, y)?;
        let mut input = input.clone();
        let p = self.previous.as_ref()?;
        let style = &p.styles[id.0 as usize];
        let rect = p.layout.rect(id);
        let step = style.number("selection-scroll-step")?;
        axis_x.move_by(direction(x, viewport.x, viewport.w) * step);
        if input.multiline {
            axis_y.move_by(direction(y, viewport.y, viewport.h) * step);
        }
        input.scroll_x = axis_x.offset;
        input.scroll_y = axis_y.offset;
        let (x, y) = clamped_point(viewport, x, y);
        let index = input.hit_index_at(&self.font, style, rect, x, y);
        self.input_offsets
            .insert(String::from(key), (input.scroll_x, input.scroll_y));
        Some(index)
    }
}
