use crate::Surface;
use dunit_style::{Color, Style};
use dunit_ui::Rect;
use dunit_widgets::slider::{SliderGeometry, SliderOrientation};
pub(crate) fn paint(
    rect: Rect,
    style: &Style,
    fraction: f32,
    orientation: SliderOrientation,
    surface: &mut Surface,
) {
    let geometry = SliderGeometry::oriented(rect, style, fraction, orientation);
    let transparent = Color::rgba(0, 0, 0, 0);
    surface.paint_box(
        geometry.track,
        geometry.track.w.min(geometry.track.h) / 2.,
        style.color("slider-track-color"),
        None,
        0.,
        transparent,
        false,
    );
    let fill = geometry.fill(fraction);
    surface.paint_box(
        fill,
        geometry.track.w.min(geometry.track.h) / 2.,
        style.color("slider-fill-color"),
        None,
        0.,
        transparent,
        false,
    );
    surface.paint_box(
        geometry.thumb,
        geometry.thumb.w / 2.,
        style.color("slider-thumb-color"),
        None,
        0.,
        transparent,
        false,
    );
}
