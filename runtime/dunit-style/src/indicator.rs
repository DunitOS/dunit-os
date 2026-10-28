//! Geometry shared by intrinsic measurement and checkable-control painting.

use crate::Style;

/// Indicator dimensions are supplied exclusively by DSS; absent geometry is empty.
pub struct Indicator {
    pub width: f32,
    pub height: f32,
    pub gap: f32,
}

impl Indicator {
    pub fn from_style(style: &Style) -> Self {
        Self {
            width: style.number("indicator-width").unwrap_or(0.).max(0.),
            height: style.number("indicator-height").unwrap_or(0.).max(0.),
            gap: style.number("indicator-gap").unwrap_or(0.).max(0.),
        }
    }

    /// Empty labels do not introduce a trailing gap.
    pub fn label_offset(&self, has_label: bool) -> f32 {
        self.width + if has_label { self.gap } else { 0. }
    }

    pub fn supports(tag: &str) -> bool {
        matches!(tag, "Checkbox" | "Radio" | "Switch")
    }
}
