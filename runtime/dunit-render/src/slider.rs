use crate::Surface;
use dunit_style::{Color, Style};
use dunit_ui::Rect;
use dunit_widgets::slider::SliderGeometry;
pub(crate) fn paint(rect: Rect, style: &Style, fraction: f32, surface: &mut Surface) {
    let geometry = SliderGeometry::new(rect, style, fraction);
    let transparent = Color::rgba(0, 0, 0, 0);
    surface.paint_box(
        geometry.track,
        geometry.track.h / 2.,
        style.color("slider-track-color"),
        None,
        0.,
        transparent,
        false,
    );
    let fill = Rect {
        w: geometry.track.w * fraction.clamp(0., 1.),
        ..geometry.track
    };
    surface.paint_box(
        fill,
        fill.h / 2.,
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
