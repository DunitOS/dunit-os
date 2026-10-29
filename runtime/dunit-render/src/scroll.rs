//! Scroll snapshot geometry; controller policy remains in the SDK.
use crate::Surface;
use dunit_style::Style;
use dunit_ui::Rect;

/// Absolute document geometry, after ancestor scrolling has been applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollArea {
    pub viewport: Rect,
    pub track: Option<Rect>,
    pub thumb: Option<Rect>,
}

pub(crate) fn paint(area: &ScrollArea, style: &Style, surface: &mut Surface) {
    for (rect, color) in [
        (area.track, style.color("scrollbar-track")),
        (area.thumb, style.color("scrollbar-thumb")),
    ] {
        if let (Some(rect), Some(color)) = (rect, color) {
            surface.paint_box(
                rect,
                style.number("scrollbar-radius").unwrap_or(0.),
                Some(color),
                None,
                0.,
                color,
                false,
            );
        }
    }
}
