use crate::Surface;
use dunit_style::{Color, Style};
use dunit_ui::Rect;
use dunit_widgets::progress::ProgressPaint;
pub(crate) fn paint(rect: Rect, style: &Style, progress: ProgressPaint, surface: &mut Surface) {
    let (track, fill) = progress.geometry(style, rect);
    let radius = style.number("progress-radius").unwrap_or(0.).max(0.);
    let transparent = Color::rgba(0, 0, 0, 0);
    surface.paint_box(
        track,
        radius,
        style.color("progress-track-color"),
        None,
        0.,
        transparent,
        false,
    );
    surface.paint_box(
        fill,
        radius,
        style.color("progress-fill-color"),
        None,
        0.,
        transparent,
        false,
    );
}
