//! Painting a laid-out DUI tree into a [`Surface`].

use dunit_style::value::Color;
use dunit_style::Style;
use dunit_text::Font;
use dunit_ui::layout::Layout;
use dunit_ui::tree::{NodeId, Tree};
use dunit_ui::Rect;

use crate::{surface::Surface, PaintResult, TextInput};

/// Resolve retained input snapshots at their place in document paint order.
pub type InputOf<'a> = &'a dyn Fn(NodeId) -> Option<&'a TextInput>;

/// A resolver from a node to its already-cascaded DSS style. Callers build these
/// from [`dunit_style::cascade`]; the painter treats the result as read-only.
pub type StyleOf<'a> = &'a dyn Fn(NodeId) -> Style;

/// Default text/border color when a style declares none: opaque black.
const DEFAULT_INK: Color = Color::rgb(0, 0, 0);
/// Default font size, matching the widget measurer.
const DEFAULT_FONT_PX: f32 = 14.0;

/// Paint the whole tree into `surface`, in document order (parents before
/// children, so children paint on top). Each node contributes its background
/// fill, border stroke and text, as declared by its resolved [`Style`].
pub fn paint(
    tree: &Tree,
    layout: &Layout,
    style_of: StyleOf,
    font: &Font,
    surface: &mut Surface,
) -> PaintResult {
    if tree.is_empty() {
        return Ok(());
    }
    paint_node(
        tree,
        layout,
        tree.root(),
        style_of,
        font,
        surface,
        false,
        &|_| None,
    )
}

/// Paint within `clip`, additionally containing each node and its descendants
/// in its layout box, with its shadow clipped by its parent. Retained
/// renderers invalidate visual bounds including shadows: glyph
/// overhang and oversized children cannot paint outside the recorded bounds.
pub fn paint_clipped(
    tree: &Tree,
    layout: &Layout,
    style_of: StyleOf,
    font: &Font,
    surface: &mut Surface,
    clip: Rect,
) -> PaintResult {
    paint_clipped_with_inputs(tree, layout, style_of, font, surface, clip, &|_| None)
}

/// Paint retained input snapshots in document order, respecting ancestor clips.
pub fn paint_clipped_with_inputs(
    tree: &Tree,
    layout: &Layout,
    style_of: StyleOf,
    font: &Font,
    surface: &mut Surface,
    clip: Rect,
    input_of: InputOf<'_>,
) -> PaintResult {
    let mut result = Ok(());
    if !tree.is_empty() {
        surface.with_clip(clip, |s| {
            result = paint_node(tree, layout, tree.root(), style_of, font, s, true, input_of);
        });
    }
    result
}

fn paint_node(
    tree: &Tree,
    layout: &Layout,
    id: NodeId,
    style_of: StyleOf,
    font: &Font,
    surface: &mut Surface,
    contained: bool,
    input_of: InputOf<'_>,
) -> PaintResult {
    let rect = layout.rect(id);
    let style = style_of(id);
    if let Some(shadow) = style
        .get("shadow")
        .and_then(dunit_style::effects::Shadow::parse)
    {
        surface.drop_shadow(rect, style.number("radius").unwrap_or(0.), shadow);
    }
    let rounded = style.keyword("clip") == Some("rounded");
    let draw = |s: &mut Surface| {
        if rounded {
            s.with_rounded_clip(rect, style.number("radius").unwrap_or(0.), |s| {
                paint_contents(
                    tree, layout, id, &style, style_of, font, s, contained, true, input_of,
                )
            })
        } else {
            paint_contents(
                tree, layout, id, &style, style_of, font, s, contained, false, input_of,
            )
        }
    };
    if contained || style.keyword("clip") == Some("rect") {
        let mut result = Ok(());
        surface.with_clip(rect, |s| {
            result = draw(s);
        });
        result
    } else {
        draw(surface)
    }
}

fn paint_contents(
    tree: &Tree,
    layout: &Layout,
    id: NodeId,
    style: &Style,
    style_of: StyleOf,
    font: &Font,
    surface: &mut Surface,
    contained: bool,
    group: bool,
    input_of: InputOf<'_>,
) -> PaintResult {
    let rect = layout.rect(id);
    surface.backdrop_blur(
        rect,
        style.number("backdrop-blur").unwrap_or(0.),
        if group {
            0.
        } else {
            style.number("radius").unwrap_or(0.)
        },
    )?;

    let border = style.number("border-width").unwrap_or(0.0);
    let radius = style.number("radius").unwrap_or(0.0);
    let gradient = style
        .get("background-gradient")
        .and_then(dunit_style::effects::Gradient::parse);
    if group || radius > 0. || gradient.is_some() {
        surface.paint_box(
            rect,
            radius,
            style.color("background"),
            gradient,
            border,
            style.color("border-color").unwrap_or(DEFAULT_INK),
            group,
        );
    } else {
        if let Some(bg) = style.color("background") {
            surface.fill_rect(rect.x, rect.y, rect.w, rect.h, bg);
        }
        if border > 0. {
            surface.stroke_rect(
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                border,
                style.color("border-color").unwrap_or(DEFAULT_INK),
            );
        }
    }

    let tag = tree.node(id).kind.tag();
    if let Some(input) = matches!(tag, "Input" | "Textarea")
        .then(|| input_of(id))
        .flatten()
    {
        input.paint(font, style, rect, surface);
    } else if dunit_style::indicator::Indicator::supports(tag) {
        let [top, right, bottom, left] = style.padding().unwrap_or([0.; 4]);
        let content = Rect {
            x: rect.x + border + left,
            y: rect.y + border + top,
            w: (rect.w - 2. * border - left - right).max(0.),
            h: (rect.h - 2. * border - top - bottom).max(0.),
        };
        surface.with_clip(rect, |surface| {
            crate::indicator::paint(tag, style, content, font, surface);
            if let Some(text) = node_text(tree, id) {
                let offset =
                    dunit_style::indicator::Indicator::from_style(style).label_offset(true);
                let label = Rect {
                    x: rect.x + offset,
                    w: (rect.w - offset).max(0.),
                    ..rect
                };
                paint_text(style, label, border, text, font, surface, true);
            }
        });
    } else if let Some(text) = node_text(tree, id) {
        paint_text(
            style,
            rect,
            border,
            text,
            font,
            surface,
            matches!(tag, "Button" | "ToggleButton"),
        );
    }

    for &c in &tree.node(id).children {
        paint_node(
            tree, layout, c, style_of, font, surface, contained, input_of,
        )?;
    }
    Ok(())
}

/// The non-empty text of a node, if any.
fn node_text(tree: &Tree, id: NodeId) -> Option<&str> {
    match tree.node(id).text.as_deref() {
        Some(t) if !t.is_empty() => Some(t),
        _ => None,
    }
}

/// Draw a single line of text inside `rect`'s content box (inset by border and
/// padding), on the font baseline.
fn paint_text(
    style: &Style,
    rect: Rect,
    border: f32,
    text: &str,
    font: &Font,
    surface: &mut Surface,
    center: bool,
) {
    let px = style.number("font-size").unwrap_or(DEFAULT_FONT_PX);
    let ink = style.color("color").unwrap_or(DEFAULT_INK);
    let [pad_t, _, pad_b, pad_l] = style.padding().unwrap_or([0.0; 4]);
    let origin_x = rect.x + border + pad_l;
    let metrics = font.line_metrics(px);
    let offset_y = if center {
        let content_height = (rect.h - 2. * border - pad_t - pad_b).max(0.);
        (content_height - metrics.line_height()).max(0.) / 2.
    } else {
        0.
    };
    let baseline = rect.y + border + pad_t + offset_y + metrics.ascent;

    let (glyphs, _advance) = font.layout_line(text, px);
    for g in glyphs {
        if let Some(bmp) = font.rasterize(g.glyph, px) {
            let ox = round_i32(origin_x + g.x) + bmp.left;
            let oy = round_i32(baseline) - bmp.top;
            surface.blit_glyph(&bmp, ox, oy, ink);
        }
    }
}

fn round_i32(v: f32) -> i32 {
    if v <= 0.0 {
        0
    } else {
        (v + 0.5) as i32
    }
}
