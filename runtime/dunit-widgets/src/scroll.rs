//! A finite, clamped scroll axis shared by viewport and scrollbar interactions.
//! Geometry is independent of styles, input protocols and painting.

/// Scroll positions and extents use the same caller-selected coordinate unit.
/// Constructors and mutations sanitize non-finite or negative extents to zero.
/// Public fields may be inspected or supplied directly; calculations sanitize
/// them before use. Finite deltas use wider intermediates to avoid overflow.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct ScrollAxis {
    pub offset: f32,
    pub viewport: f32,
    pub content: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thumb {
    pub start: f32,
    pub length: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollCommand {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Home,
    End,
}

fn extent(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.)
    } else {
        0.
    }
}

fn coordinate(value: f64) -> f32 {
    value.clamp(-(f32::MAX as f64), f32::MAX as f64) as f32
}

impl ScrollAxis {
    pub fn new(viewport: f32, content: f32) -> Self {
        let mut axis = Self::default();
        axis.set_extents(viewport, content);
        axis
    }

    pub fn set_extents(&mut self, viewport: f32, content: f32) -> bool {
        let old = *self;
        self.viewport = extent(viewport);
        self.content = extent(content);
        self.offset = extent(self.offset).min(self.max_offset());
        *self != old
    }

    pub fn max_offset(&self) -> f32 {
        (extent(self.content) - extent(self.viewport)).max(0.)
    }

    fn position(&mut self, position: f64) -> bool {
        let old = *self;
        self.viewport = extent(self.viewport);
        self.content = extent(self.content);
        self.offset = position.clamp(0., self.max_offset() as f64) as f32;
        *self != old
    }

    pub fn move_by(&mut self, delta: f32) -> bool {
        if !delta.is_finite() {
            return false;
        }
        self.position(extent(self.offset).min(self.max_offset()) as f64 + delta as f64)
    }

    /// Reveal a content interval using the smallest offset change. Endpoints
    /// are clamped to content; invalid or reversed intervals are rejected.
    /// If the interval exceeds the viewport, keep the viewport inside it with
    /// minimum movement (including no movement when already inside).
    pub fn reveal(&mut self, start: f32, end: f32) -> bool {
        if !start.is_finite() || !end.is_finite() || start > end {
            return false;
        }
        let content = extent(self.content) as f64;
        let viewport = extent(self.viewport) as f64;
        let start = (start as f64).clamp(0., content);
        let end = (end as f64).clamp(0., content);
        let offset = extent(self.offset).min(self.max_offset()) as f64;
        let position = if end - start <= viewport {
            offset.clamp((end - viewport).max(0.), start)
        } else {
            offset.clamp(start, end - viewport)
        };
        self.position(position)
    }

    /// Return the proportional thumb only when content overflows and the track
    /// has positive finite length. Negative minimum sizes behave as zero.
    /// A minimum larger than the track fills it; such a thumb cannot be dragged.
    pub fn thumb(&self, track_start: f32, track_length: f32, min_thumb: f32) -> Option<Thumb> {
        if !track_start.is_finite()
            || !track_length.is_finite()
            || !min_thumb.is_finite()
            || track_length <= 0.
            || self.max_offset() <= 0.
        {
            return None;
        }
        let track = track_length as f64;
        let length = (track * extent(self.viewport) as f64 / extent(self.content) as f64)
            .max(min_thumb.max(0.) as f64)
            .min(track);
        let offset = extent(self.offset).min(self.max_offset()) as f64;
        let start = track_start as f64 + (track - length) * offset / self.max_offset() as f64;
        Some(Thumb {
            start: coordinate(start),
            length: length as f32,
        })
    }

    /// `grab` is the pointer's distance from the thumb's leading edge at press.
    /// Preserve that anchor while mapping pointer travel to the content offset.
    pub fn drag_thumb(
        &mut self,
        pointer: f32,
        grab: f32,
        track_start: f32,
        track_length: f32,
        min_thumb: f32,
    ) -> bool {
        if !pointer.is_finite() || !grab.is_finite() {
            return false;
        }
        let Some(thumb) = self.thumb(track_start, track_length, min_thumb) else {
            return false;
        };
        let travel = track_length as f64 - thumb.length as f64;
        if travel <= 0. {
            return false;
        }
        let grab = (grab as f64).clamp(0., thumb.length as f64);
        let fraction = ((pointer as f64 - grab - track_start as f64) / travel).clamp(0., 1.);
        self.position(fraction * self.max_offset() as f64)
    }

    /// Page commands move one viewport. Invalid line steps reject line commands;
    /// non-line commands do not depend on the caller's line-step setting.
    pub fn apply(&mut self, command: ScrollCommand, line_step: f32) -> bool {
        match command {
            ScrollCommand::LineUp | ScrollCommand::LineDown => {
                if !line_step.is_finite() || line_step < 0. {
                    return false;
                }
                self.move_by(if command == ScrollCommand::LineUp {
                    -line_step
                } else {
                    line_step
                })
            }
            ScrollCommand::PageUp => self.move_by(-extent(self.viewport)),
            ScrollCommand::PageDown => self.move_by(extent(self.viewport)),
            ScrollCommand::Home => self.position(0.),
            ScrollCommand::End => self.position(self.max_offset() as f64),
        }
    }
}
