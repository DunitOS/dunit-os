//! Integer slider arithmetic and shared paint/input geometry.
use dunit_style::Style;
use dunit_ui::Rect;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// View validation rejects invalid configurations before dispatch/painting.
/// Arithmetic helpers require a configuration/value accepted by `valid`.
pub struct SliderOptions {
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub page_step: i64,
}
impl SliderOptions {
    pub fn new(min: i64, max: i64, step: i64) -> Self {
        Self {
            min,
            max,
            step,
            page_step: if step > 0 {
                step * (i64::MAX / step).min(10)
            } else {
                0
            },
        }
    }
    pub fn valid(self, value: i64) -> bool {
        self.min <= self.max
            && self.step > 0
            && self.page_step >= self.step
            && self.page_step % self.step == 0
            && value >= self.min
            && value <= self.max
            && self.snap(value as i128) == value
    }
    pub fn snap(self, value: i128) -> i64 {
        if value <= self.min as i128 {
            return self.min;
        }
        if value >= self.max as i128 {
            return self.max;
        }
        let offset = value - self.min as i128;
        (self.min as i128
            + ((offset + self.step as i128 / 2) / self.step as i128) * self.step as i128)
            .min(self.max as i128) as i64
    }
    pub fn fraction(self, value: i64) -> f32 {
        let span = self.max as i128 - self.min as i128;
        if span == 0 {
            0.
        } else {
            ((value as i128 - self.min as i128) as f64 / span as f64) as f32
        }
    }
    pub fn at_fraction(self, fraction: f32) -> i64 {
        if fraction <= 0. {
            return self.min;
        }
        if fraction >= 1. {
            return self.max;
        }
        let span = self.max as i128 - self.min as i128;
        self.snap(self.min as i128 + (span as f64 * fraction as f64 + 0.5) as i128)
    }
    pub fn command(self, value: i64, command: SliderCommand) -> i64 {
        match command {
            SliderCommand::Home => self.min,
            SliderCommand::End => self.max,
            SliderCommand::Increase => self.snap(value as i128 + self.step as i128),
            SliderCommand::Decrease => {
                // The endpoint may be off-grid; move to the previous grid stop.
                let value = value as i128 - 1;
                if value < self.min as i128 {
                    self.min
                } else {
                    (self.min as i128
                        + (value - self.min as i128) / self.step as i128 * self.step as i128)
                        as i64
                }
            }
            SliderCommand::PageUp => self.snap(value as i128 + self.page_step as i128),
            SliderCommand::PageDown => self.snap(value as i128 - self.page_step as i128),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderCommand {
    Increase,
    Decrease,
    Home,
    End,
    PageUp,
    PageDown,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderGeometry {
    pub track: Rect,
    pub thumb: Rect,
}
impl SliderGeometry {
    pub fn new(rect: Rect, style: &Style, fraction: f32) -> Self {
        let border = style.number("border-width").unwrap_or(0.).max(0.);
        let [t, r, b, l] = style.padding().unwrap_or([0.; 4]);
        let area = Rect {
            x: rect.x + border + l,
            y: rect.y + border + t,
            w: (rect.w - 2. * border - l - r).max(0.),
            h: (rect.h - 2. * border - t - b).max(0.),
        };
        let diameter = style
            .number("slider-thumb-size")
            .unwrap_or(0.)
            .max(0.)
            .min(area.w)
            .min(area.h);
        let thickness = style
            .number("slider-track-height")
            .unwrap_or(0.)
            .max(0.)
            .min(area.h);
        let track = Rect {
            x: area.x + diameter / 2.,
            y: area.y + (area.h - thickness) / 2.,
            w: (area.w - diameter).max(0.),
            h: thickness,
        };
        let thumb = Rect {
            x: track.x + track.w * fraction.clamp(0., 1.) - diameter / 2.,
            y: area.y + (area.h - diameter) / 2.,
            w: diameter,
            h: diameter,
        };
        Self { track, thumb }
    }
    pub fn fraction(self, x: f32, grab: f32) -> Option<f32> {
        if !x.is_finite() || !grab.is_finite() || self.track.w <= 0. {
            return None;
        }
        Some(((x - grab - self.track.x) / self.track.w).clamp(0., 1.))
    }
    pub fn grab(self, x: f32) -> f32 {
        if x >= self.thumb.x && x <= self.thumb.x + self.thumb.w {
            x - self.thumb.x - self.thumb.w / 2.
        } else {
            0.
        }
    }
}
