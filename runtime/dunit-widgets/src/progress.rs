//! Passive progress arithmetic and immutable paint geometry.
use dunit_style::Style;
use dunit_ui::Rect;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressOptions {
    pub min: i64,
    pub max: i64,
}
impl ProgressOptions {
    pub const fn new(min: i64, max: i64) -> Self {
        Self { min, max }
    }
    pub fn valid(self, value: i64) -> bool {
        self.min <= self.max && value >= self.min && value <= self.max
    }
    /// A zero-span range is complete. Validated integer ranges cannot overflow.
    pub fn fraction(self, value: i64) -> f32 {
        let span = self.max as i128 - self.min as i128;
        if span == 0 {
            1.
        } else {
            ((value as i128 - self.min as i128) as f64 / span as f64) as f32
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgressPaint {
    pub start: f32,
    pub end: f32,
}
impl ProgressPaint {
    pub fn determinate(fraction: f32) -> Self {
        Self {
            start: 0.,
            end: fraction.clamp(0., 1.),
        }
    }
    /// A bounded segment travels left-to-right and back without jumping at wrap.
    pub fn busy(now: Option<u64>, duration: u64, segment: f32, running: bool) -> Self {
        if !running {
            return Self::determinate(0.);
        }
        let segment = segment.clamp(0., 1.);
        let position = now.filter(|_| duration > 0).map_or(0.5, |now| {
            let phase = (now % duration) as f64 / duration as f64;
            (if phase <= 0.5 {
                phase * 2.
            } else {
                2. - phase * 2.
            }) as f32
        });
        let start = position * (1. - segment);
        Self {
            start,
            end: start + segment,
        }
    }
    pub fn geometry(self, style: &Style, rect: Rect) -> (Rect, Rect) {
        let border = style.number("border-width").unwrap_or(0.).max(0.);
        let [t, r, b, l] = style.padding().unwrap_or([0.; 4]);
        let area = Rect {
            x: rect.x + border + l,
            y: rect.y + border + t,
            w: (rect.w - 2. * border - l - r).max(0.),
            h: (rect.h - 2. * border - t - b).max(0.),
        };
        let thickness = style
            .number("progress-thickness")
            .unwrap_or(0.)
            .max(0.)
            .min(area.h);
        let track = Rect {
            y: area.y + (area.h - thickness) / 2.,
            h: thickness,
            ..area
        };
        let start = self.start.clamp(0., 1.);
        let end = self.end.clamp(start, 1.);
        (
            track,
            Rect {
                x: track.x + track.w * start,
                w: track.w * (end - start),
                ..track
            },
        )
    }
}
