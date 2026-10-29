//! Scroll snapshot geometry; controller policy remains in the SDK.
use crate::Surface;
use dunit_style::Style;
use dunit_ui::Rect;
use dunit_widgets::ScrollAxis;

/// Absolute document geometry, after ancestor scrolling has been applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollArea {
    pub viewport: Rect,
    pub track: Option<Rect>,
    pub thumb: Option<Rect>,
    pub horizontal_track: Option<Rect>,
    pub horizontal_thumb: Option<Rect>,
}

impl ScrollArea {
    /// Build auto-hidden scrollbars outside the content viewport, in gutters
    /// already reserved by the caller. Both axes share theme geometry.
    pub fn from_axes(
        viewport: Rect,
        style: &Style,
        x: Option<&ScrollAxis>,
        y: Option<&ScrollAxis>,
    ) -> Self {
        let width = style
            .number("scrollbar-width")
            .filter(|v| v.is_finite())
            .unwrap_or(0.)
            .max(0.);
        let min = style
            .number("scrollbar-min-thumb")
            .filter(|v| v.is_finite())
            .unwrap_or(0.)
            .max(0.);
        // A collapsed content box has no usable gutter geometry. Reserving a
        // full-width bar there would paint back over padding or the border.
        let usable =
            viewport.w.is_finite() && viewport.h.is_finite() && viewport.w > 0. && viewport.h > 0.;
        let vertical = y
            .filter(|_| usable && width > 0.)
            .and_then(|axis| axis.thumb(viewport.y, viewport.h, min));
        let horizontal = x
            .filter(|_| usable && width > 0.)
            .and_then(|axis| axis.thumb(viewport.x, viewport.w, min));
        Self {
            viewport,
            track: vertical.map(|_| Rect {
                x: viewport.x + viewport.w,
                y: viewport.y,
                w: width,
                h: viewport.h,
            }),
            thumb: vertical.map(|thumb| Rect {
                x: viewport.x + viewport.w,
                y: thumb.start,
                w: width,
                h: thumb.length,
            }),
            horizontal_track: horizontal.map(|_| Rect {
                x: viewport.x,
                y: viewport.y + viewport.h,
                w: viewport.w,
                h: width,
            }),
            horizontal_thumb: horizontal.map(|thumb| Rect {
                x: thumb.start,
                y: viewport.y + viewport.h,
                w: thumb.length,
                h: width,
            }),
        }
    }
}

pub(crate) fn paint(area: &ScrollArea, style: &Style, surface: &mut Surface) {
    for (rect, color) in [
        (area.track, style.color("scrollbar-track")),
        (area.horizontal_track, style.color("scrollbar-track")),
        (area.thumb, style.color("scrollbar-thumb")),
        (area.horizontal_thumb, style.color("scrollbar-thumb")),
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
