//! Rounded fills, inside borders and bounded layered drop shadows.
use crate::{
    raster::{lerp_color, rr_cov, RR_ALL},
    Surface,
};
use dunit_style::{
    effects::{Gradient, Shadow},
    Color, Style,
};
use dunit_ui::Rect;

/// Conservative painted bounds, including the shadow outside the layout box.
/// Ancestor layout clips and the destination surface may reduce these bounds.
pub fn visual_bounds(rect: Rect, style: &Style) -> Rect {
    let Some(shadow) = style.get("shadow").and_then(Shadow::parse) else {
        return rect;
    };
    if shadow.color.a == 0 || shadow.spread == 0. {
        return rect;
    }
    let left = rect.x.min(rect.x + shadow.x - shadow.spread);
    let top = rect.y.min(rect.y + shadow.y - shadow.spread);
    let right = (rect.x + rect.w).max(rect.x + rect.w + shadow.x + shadow.spread);
    let bottom = (rect.y + rect.h).max(rect.y + rect.h + shadow.y + shadow.spread);
    // One extra pixel accounts for independent nearest-pixel rounding of layers.
    Rect {
        x: left - 1.,
        y: top - 1.,
        w: right - left + 2.,
        h: bottom - top + 2.,
    }
}

pub(crate) fn integer(rect: Rect) -> (i32, i32, i32, i32) {
    let round = |v: f32| {
        // Stay within exact f32 integer precision and signed raster arithmetic.
        let value = (v + 0.5).clamp(-16_777_216., 16_777_216.);
        let integer = value as i32;
        integer - i32::from((integer as f32) > value)
    };
    let x = round(rect.x);
    let y = round(rect.y);
    (x, y, round(rect.x + rect.w) - x, round(rect.y + rect.h) - y)
}
pub(crate) fn coverage(px: i32, py: i32, rect: (i32, i32, i32, i32), radius: i32) -> u32 {
    let (x, y, w, h) = rect;
    if px < x || py < y || px >= x + w || py >= y + h || w <= 0 || h <= 0 {
        return 0;
    }
    rr_cov(
        px,
        py,
        x,
        y,
        w,
        h,
        radius.max(0).min(w / 2).min(h / 2),
        RR_ALL,
    )
}
impl Surface<'_> {
    /// A rounded fill; gradient replaces the flat background when supplied.
    /// Inside border coverage is the outer mask minus the inset mask, so each
    /// border pixel is blended once even at corners and with thick strokes.
    pub fn rounded_box(
        &mut self,
        rect: Rect,
        radius: f32,
        fill: Option<Color>,
        gradient: Option<Gradient>,
        border: f32,
        ink: Color,
    ) {
        self.paint_box(rect, radius, fill, gradient, border, ink, false);
    }
    /// A group mask supplies outer coverage once, after its descendants paint.
    pub(crate) fn paint_box(
        &mut self,
        rect: Rect,
        radius: f32,
        fill: Option<Color>,
        gradient: Option<Gradient>,
        border: f32,
        ink: Color,
        group: bool,
    ) {
        let outer = integer(rect);
        let (x, y, w, h) = outer;
        if w <= 0 || h <= 0 {
            return;
        }
        let radius = ((radius.max(0.) + 0.5) as i32).min(outer.2.min(outer.3).max(0) / 2);
        let thickness = ((border.max(0.) + 0.5) as i32).min((w.min(h) + 1) / 2);
        let inner = (
            x + thickness,
            y + thickness,
            w - 2 * thickness,
            h - 2 * thickness,
        );
        let (x0, y0, x1, y1) = self.clip_rect(rect.x, rect.y, rect.w, rect.h);
        for py in y0..y1 {
            for px in x0..x1 {
                let cov = coverage(px as i32, py as i32, outer, radius);
                let color = gradient
                    .map(|g| {
                        let (position, span) = if g.horizontal {
                            (px as i32 - x, w - 1)
                        } else {
                            (py as i32 - y, h - 1)
                        };
                        let t = position.max(0) as u32 * 255 / span.max(1) as u32;
                        let rgb = lerp_color(crate::pack_argb(g.start), crate::pack_argb(g.end), t);
                        let alpha = (g.start.a as u32 * (255 - t) + g.end.a as u32 * t) / 255;
                        Color::rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, alpha as u8)
                    })
                    .or(fill);
                if let Some(color) = color {
                    self.blend(px, py, color, if group { 255 } else { cov as u8 });
                }
                if thickness > 0 {
                    let inside = coverage(px as i32, py as i32, inner, (radius - thickness).max(0));
                    let ring = cov.saturating_sub(inside);
                    let ring = if group && cov > 0 {
                        ring * 255 / cov
                    } else {
                        ring
                    };
                    self.blend(px, py, ink, ring as u8);
                }
            }
        }
    }
    /// Layered falloff around a rounded silhouette. The node interior is
    /// excluded by its coverage, including curved corners; works with alpha
    /// fills without painting a dark rectangle through the middle.
    pub fn drop_shadow(&mut self, rect: Rect, radius: f32, shadow: Shadow) {
        if !shadow.is_valid() {
            return;
        }
        let spread = (shadow.spread + 0.5) as i32;
        if spread <= 0 || shadow.color.a == 0 {
            return;
        }
        let outer = integer(rect);
        let radius = ((radius.max(0.) + 0.5) as i32).min(outer.2.min(outer.3).max(0) / 2);
        let mut color = shadow.color;
        let alpha = color.a as i32;
        for layer in (1..=spread).rev() {
            color.a = (alpha / spread + i32::from(layer <= alpha % spread)) as u8;
            if color.a == 0 {
                continue;
            }
            let layer = layer as f32;
            let shape = Rect {
                x: rect.x + shadow.x - layer,
                y: rect.y + shadow.y - layer,
                w: rect.w + 2. * layer,
                h: rect.h + 2. * layer,
            };
            let geometry = integer(shape);
            let (x0, y0, x1, y1) = self.clip_rect(shape.x, shape.y, shape.w, shape.h);
            for py in y0..y1 {
                let mut px = x0;
                while px < x1 {
                    let interior = coverage(px as i32, py as i32, outer, radius);
                    if interior == 255 {
                        // Skip the covered centre; work scales with the shadow
                        // perimeter instead of painting every interior per layer.
                        let (x, y, w, h) = outer;
                        let right = if (y + radius..y + h - radius).contains(&(py as i32)) {
                            x + w
                        } else {
                            x + w - radius
                        };
                        px = px.saturating_add(1).max(right.max(0) as usize);
                        continue;
                    }
                    let cov = coverage(px as i32, py as i32, geometry, radius + layer as i32);
                    self.blend(px, py, color, (cov * (255 - interior) / 255) as u8);
                    px += 1;
                }
            }
        }
    }
}
