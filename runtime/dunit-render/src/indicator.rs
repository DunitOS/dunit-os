//! Paint resolved checkable indicators without consulting mutable widget state.

use crate::Surface;
use dunit_style::{indicator::Indicator, Color, Style};
use dunit_text::Font;
use dunit_ui::Rect;

pub(crate) fn paint(tag: &str, style: &Style, content: Rect, font: &Font, surface: &mut Surface) {
    let geometry = Indicator::from_style(style);
    let rect = Rect {
        x: content.x,
        y: content.y + (content.h - geometry.height).max(0.) / 2.,
        w: geometry.width,
        h: geometry.height,
    };
    let transparent = Color::rgba(0, 0, 0, 0);
    surface.paint_box(
        rect,
        style.number("indicator-radius").unwrap_or(0.),
        style.color("indicator-background"),
        None,
        style.number("indicator-border-width").unwrap_or(0.),
        style.color("indicator-border-color").unwrap_or(transparent),
        false,
    );
    let ink = style.color("indicator-mark-color").unwrap_or(transparent);
    let inset = style.number("indicator-inset").unwrap_or(0.).max(0.);
    let size = style
        .number("indicator-mark-size")
        .unwrap_or(0.)
        .max(0.)
        .min((rect.w - 2. * inset).max(0.))
        .min((rect.h - 2. * inset).max(0.));
    if size <= 0. || ink.a == 0 {
        return;
    }
    let position = if tag == "Switch" {
        style
            .number("indicator-position")
            .unwrap_or(0.)
            .clamp(0., 1.)
    } else {
        0.5
    };
    let mark = Rect {
        x: rect.x + inset + (rect.w - 2. * inset - size) * position,
        y: rect.y + (rect.h - size) / 2.,
        w: size,
        h: size,
    };
    surface.with_clip(rect, |s| {
        if tag == "Checkbox" {
            let (glyphs, _) = font.layout_line("✓", size);
            if let Some(glyph) = glyphs.first() {
                if let Some(bitmap) = font.rasterize(glyph.glyph, size) {
                    let x = round(mark.x + (mark.w - bitmap.width as f32) / 2.);
                    let y = round(mark.y + (mark.h - bitmap.height as f32) / 2.);
                    s.blit_glyph(&bitmap, x, y, ink);
                }
            }
        } else {
            s.paint_box(mark, size / 2., Some(ink), None, 0., transparent, false);
        }
    });
}

fn round(value: f32) -> i32 {
    (value + if value < 0. { -0.5 } else { 0.5 }) as i32
}
