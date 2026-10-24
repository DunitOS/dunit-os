use crate::geometry::transform;
use crate::{length, number, Error, Result};
use alloc::vec::Vec;
use roxmltree::Node;
use tiny_skia::{
    Color, GradientStop, LineCap, LineJoin, LinearGradient, Point, RadialGradient, Rect, Shader,
    SpreadMode, Stroke, StrokeDash, Transform,
};

pub(crate) fn prop<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    let inline = node.attribute("style").and_then(|s| {
        s.split(';')
            .filter_map(|v| v.split_once(':'))
            .filter(|(k, _)| k.trim() == name)
            .last()
            .map(|(_, v)| v.trim())
    });
    inline.or_else(|| node.attribute(name))
}

pub(crate) fn opacity(s: Option<&str>) -> Result<f32> {
    Ok(length(s.unwrap_or("1"), 1.0)?.clamp(0.0, 1.0))
}

#[derive(Clone)]
pub(crate) struct Style<'a> {
    pub fill: &'a str,
    pub stroke: &'a str,
    pub color: &'a str,
    pub fill_alpha: f32,
    pub stroke_alpha: f32,
    pub rule: tiny_skia::FillRule,
    pub pen: Stroke,
}
impl Default for Style<'_> {
    fn default() -> Self {
        Self {
            fill: "black",
            stroke: "none",
            color: "black",
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            rule: tiny_skia::FillRule::Winding,
            pen: Stroke::default(),
        }
    }
}
impl<'a> Style<'a> {
    pub fn inherit(&self, n: Node<'a, '_>, view: (f32, f32)) -> Result<Self> {
        let mut s = self.clone();
        for (key, dst) in [
            ("fill", &mut s.fill),
            ("stroke", &mut s.stroke),
            ("color", &mut s.color),
        ] {
            if let Some(v) = prop(n, key).filter(|v| *v != "inherit") {
                *dst = v;
            }
        }
        if let Some(v) = prop(n, "fill-opacity").filter(|v| *v != "inherit") {
            s.fill_alpha = opacity(Some(v))?;
        }
        if let Some(v) = prop(n, "stroke-opacity").filter(|v| *v != "inherit") {
            s.stroke_alpha = opacity(Some(v))?;
        }
        if let Some(v) = prop(n, "fill-rule").filter(|v| *v != "inherit") {
            s.rule = match v {
                "nonzero" => tiny_skia::FillRule::Winding,
                "evenodd" => tiny_skia::FillRule::EvenOdd,
                _ => return Err(Error::Invalid),
            };
        }
        if let Some(v) = prop(n, "stroke-width").filter(|v| *v != "inherit") {
            s.pen.width = length(
                v,
                libm::sqrtf(view.0 * view.0 + view.1 * view.1) / core::f32::consts::SQRT_2,
            )?;
            if s.pen.width < 0.0 {
                return Err(Error::Invalid);
            }
        }
        if let Some(v) = prop(n, "stroke-linecap").filter(|v| *v != "inherit") {
            s.pen.line_cap = match v {
                "butt" => LineCap::Butt,
                "round" => LineCap::Round,
                "square" => LineCap::Square,
                _ => return Err(Error::Invalid),
            };
        }
        if let Some(v) = prop(n, "stroke-linejoin").filter(|v| *v != "inherit") {
            s.pen.line_join = match v {
                "miter" => LineJoin::Miter,
                "round" => LineJoin::Round,
                "bevel" => LineJoin::Bevel,
                _ => return Err(Error::Invalid),
            };
        }
        if let Some(v) = prop(n, "stroke-miterlimit").filter(|v| *v != "inherit") {
            s.pen.miter_limit = number(v)?;
            if s.pen.miter_limit < 1.0 {
                return Err(Error::Invalid);
            }
        }
        // Dashing is local to this profile; inheritance uses the actual Stroke.
        if let Some(v) = prop(n, "stroke-dasharray").filter(|v| *v != "inherit") {
            s.pen.dash = if v == "none" {
                None
            } else {
                let mut values = Vec::new();
                for v in svgtypes::NumberListParser::from(v) {
                    let v = v.map_err(|_| Error::Invalid)? as f32;
                    if !v.is_finite() || v < 0.0 || values.len() >= 128 {
                        return Err(Error::Limit);
                    }
                    values.push(v);
                }
                if values.len() % 2 != 0 {
                    let extra = values.clone();
                    values.extend_from_slice(&extra);
                }
                Some(
                    StrokeDash::new(values, number(prop(n, "stroke-dashoffset").unwrap_or("0"))?)
                        .ok_or(Error::Invalid)?,
                )
            };
        } else if prop(n, "stroke-dashoffset").is_some() {
            return Err(Error::Unsupported);
        }
        Ok(s)
    }
}

fn color(s: &str, current: &str) -> Result<Color> {
    let s = if s == "currentColor" { current } else { s };
    let c: svgtypes::Color = s.parse().map_err(|_| Error::Invalid)?;
    Ok(Color::from_rgba8(c.red, c.green, c.blue, c.alpha))
}

pub(crate) fn shader<'a>(
    value: &str,
    alpha: f32,
    current: &str,
    node: Node<'a, '_>,
    bounds: Rect,
    view: (f32, f32),
) -> Result<Option<Shader<'static>>> {
    let mut shader = match svgtypes::Paint::from_str(value).map_err(|_| Error::Invalid)? {
        svgtypes::Paint::None => return Ok(None),
        svgtypes::Paint::Color(c) => {
            Shader::SolidColor(Color::from_rgba8(c.red, c.green, c.blue, c.alpha))
        }
        svgtypes::Paint::CurrentColor => Shader::SolidColor(color(current, "black")?),
        svgtypes::Paint::FuncIRI(iri, fallback) => {
            let id = iri;
            if id.is_empty() || id.contains(['/', ':']) {
                return Err(Error::Unsupported);
            }
            let g = node
                .document()
                .descendants()
                .find(|n| n.attribute("id") == Some(id));
            if let Some(g) = g {
                gradient(g, bounds, view, current)?
            } else {
                match fallback {
                    Some(svgtypes::PaintFallback::None) => return Ok(None),
                    Some(svgtypes::PaintFallback::CurrentColor) => {
                        Shader::SolidColor(color(current, "black")?)
                    }
                    Some(svgtypes::PaintFallback::Color(c)) => {
                        Shader::SolidColor(Color::from_rgba8(c.red, c.green, c.blue, c.alpha))
                    }
                    None => return Err(Error::Invalid),
                }
            }
        }
        _ => return Err(Error::Unsupported),
    };
    shader.apply_opacity(alpha);
    Ok(Some(shader))
}

fn gradient(
    g: Node<'_, '_>,
    bounds: Rect,
    view: (f32, f32),
    current: &str,
) -> Result<Shader<'static>> {
    let object = match g.attribute("gradientUnits").unwrap_or("objectBoundingBox") {
        "objectBoundingBox" => true,
        "userSpaceOnUse" => false,
        _ => return Err(Error::Invalid),
    };
    let (ax, ay) = if object { (1.0, 1.0) } else { view };
    let coordinate = |key, default, axis| length(g.attribute(key).unwrap_or(default), axis);
    let gt = transform(g.attribute("gradientTransform"))?;
    let ts = if object {
        Transform::from_row(
            bounds.width(),
            0.0,
            0.0,
            bounds.height(),
            bounds.x(),
            bounds.y(),
        )
        .pre_concat(gt)
    } else {
        gt
    };
    let spread = match g.attribute("spreadMethod").unwrap_or("pad") {
        "pad" => SpreadMode::Pad,
        "repeat" => SpreadMode::Repeat,
        "reflect" => SpreadMode::Reflect,
        _ => return Err(Error::Invalid),
    };
    let mut stops = Vec::new();
    let mut previous = 0.0f32;
    let mut last_color = Color::TRANSPARENT;
    for stop in g.children().filter(|n| n.is_element()) {
        if stop.tag_name().name() != "stop" {
            return Err(Error::Unsupported);
        }
        if stops.len() >= 256 {
            return Err(Error::Limit);
        }
        let offset = length(stop.attribute("offset").unwrap_or("0"), 1.0)?.clamp(previous, 1.0);
        previous = offset;
        let mut c = color(prop(stop, "stop-color").unwrap_or("black"), current)?;
        c.apply_opacity(opacity(prop(stop, "stop-opacity"))?);
        last_color = c;
        stops.push(GradientStop::new(offset, c));
    }
    if stops.is_empty() {
        return Ok(Shader::SolidColor(Color::TRANSPARENT));
    }
    if stops.len() == 1 {
        return Ok(Shader::SolidColor(last_color));
    }
    match g.tag_name().name() {
        "linearGradient" => {
            let a = Point::from_xy(coordinate("x1", "0%", ax)?, coordinate("y1", "0%", ay)?);
            let b = Point::from_xy(coordinate("x2", "100%", ax)?, coordinate("y2", "0%", ay)?);
            if a == b {
                return Ok(Shader::SolidColor(last_color));
            }
            LinearGradient::new(a, b, stops, spread, ts).ok_or(Error::Invalid)
        }
        "radialGradient" => {
            let cx = coordinate("cx", "50%", ax)?;
            let cy = coordinate("cy", "50%", ay)?;
            let r = coordinate(
                "r",
                "50%",
                libm::sqrtf(ax * ax + ay * ay) / core::f32::consts::SQRT_2,
            )?;
            let fx = if g.attribute("fx").is_some() {
                coordinate("fx", "0", ax)?
            } else {
                cx
            };
            let fy = if g.attribute("fy").is_some() {
                coordinate("fy", "0", ay)?
            } else {
                cy
            };
            if r < 0.0 {
                return Err(Error::Invalid);
            }
            if r == 0.0 {
                return Ok(Shader::SolidColor(last_color));
            }
            RadialGradient::new(
                Point::from_xy(fx, fy),
                0.0,
                Point::from_xy(cx, cy),
                r,
                stops,
                spread,
                ts,
            )
            .ok_or(Error::Invalid)
        }
        _ => Err(Error::Invalid),
    }
}
