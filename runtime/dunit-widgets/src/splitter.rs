//! Split pane rectangles and pointer mapping share a single geometry.
use dunit_ui::Rect;
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitGeometry {
    pub first: Rect,
    pub handle: Rect,
    pub second: Rect,
    pub vertical: bool,
    pub start: f32,
    pub available: f32,
    pub low: f32,
    pub high: f32,
}
impl SplitGeometry {
    pub fn new(
        area: Rect,
        thickness: f32,
        vertical: bool,
        value: i64,
        first_min: f32,
        first_max: Option<f32>,
        second_min: f32,
        second_max: Option<f32>,
    ) -> Self {
        let length = if vertical { area.h } else { area.w };
        let thickness = thickness.max(0.).min(length);
        let available = (length - thickness).max(0.);
        let a = first_min.max(0.);
        let b = second_min.max(0.);
        let amin = a.min(available);
        let bmin = b.min(available);
        let amax = first_max.unwrap_or(available).max(amin).min(available);
        let bmax = second_max.unwrap_or(available).max(bmin).min(available);
        let low = amin.max(available - bmax);
        let high = amax.min(available - bmin);
        let (low, high) = if low <= high {
            (low, high)
        } else if a + b > available {
            let fixed = if a + b > 0. {
                ((available as f64) * (a as f64) / (a as f64 + b as f64)) as f32
            } else {
                0.
            };
            (fixed, fixed)
        } else {
            (amax, amax)
        };
        let first = (available * (value.clamp(0, 1000) as f32 / 1000.)).clamp(low, high);
        let second = (available - first).min(bmax);
        let (first, handle, second) = if vertical {
            (
                Rect { h: first, ..area },
                Rect {
                    y: area.y + first,
                    h: thickness,
                    ..area
                },
                Rect {
                    y: area.y + first + thickness,
                    h: second,
                    ..area
                },
            )
        } else {
            (
                Rect { w: first, ..area },
                Rect {
                    x: area.x + first,
                    w: thickness,
                    ..area
                },
                Rect {
                    x: area.x + first + thickness,
                    w: second,
                    ..area
                },
            )
        };
        Self {
            first,
            handle,
            second,
            vertical,
            start: if vertical { area.y } else { area.x },
            available,
            low,
            high,
        }
    }
    pub fn coordinate(self, x: f32, y: f32) -> f32 {
        if self.vertical {
            y
        } else {
            x
        }
    }
    pub fn grab(self, coordinate: f32) -> f32 {
        let start = if self.vertical {
            self.handle.y
        } else {
            self.handle.x
        };
        let size = if self.vertical {
            self.handle.h
        } else {
            self.handle.w
        };
        coordinate - start - size / 2.
    }
    pub fn value(self, coordinate: f32, grab: f32) -> Option<i64> {
        if !coordinate.is_finite() || !grab.is_finite() || self.available <= 0. {
            return None;
        }
        let thickness = if self.vertical {
            self.handle.h
        } else {
            self.handle.w
        };
        let position = (coordinate - grab - self.start - thickness / 2.).clamp(self.low, self.high);
        Some((position as f64 / self.available as f64 * 1000. + 0.5) as i64)
    }
}
