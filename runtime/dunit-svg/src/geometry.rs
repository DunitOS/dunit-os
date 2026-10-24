use crate::{length, Error, Result, MAX_SEGMENTS};
use roxmltree::Node;
use tiny_skia::{Path, PathBuilder, Rect, Transform};

pub(crate) fn transform(s: Option<&str>) -> Result<Transform> {
    let Some(s) = s else {
        return Ok(Transform::identity());
    };
    let t: svgtypes::Transform = s.parse().map_err(|_| Error::Invalid)?;
    let values = [t.a, t.b, t.c, t.d, t.e, t.f];
    if values.iter().any(|n| !n.is_finite() || n.abs() > 1e7) {
        return Err(Error::Invalid);
    }
    Ok(Transform::from_row(
        t.a as f32, t.b as f32, t.c as f32, t.d as f32, t.e as f32, t.f as f32,
    ))
}

fn coord(v: f64) -> Result<f32> {
    if !v.is_finite() || v.abs() > 1e7 {
        Err(Error::Invalid)
    } else {
        Ok(v as f32)
    }
}

pub(crate) fn shape(n: Node<'_, '_>, view: (f32, f32), budget: &mut usize) -> Result<Option<Path>> {
    let dim = |key, axis| -> Result<f32> { length(n.attribute(key).unwrap_or("0"), axis) };
    let mut p = PathBuilder::new();
    match n.tag_name().name() {
        "path" => {
            for segment in svgtypes::SimplifyingPathParser::from(n.attribute("d").unwrap_or("")) {
                *budget += 1;
                if *budget > MAX_SEGMENTS {
                    return Err(Error::Limit);
                }
                use svgtypes::SimplePathSegment::*;
                let seg = segment.map_err(|_| Error::Invalid)?;
                // The tokenizer handles relative paths, shorthand curves and arcs.
                match seg {
                    MoveTo { x, y } => p.move_to(coord(x)?, coord(y)?),
                    LineTo { x, y } => p.line_to(coord(x)?, coord(y)?),
                    CurveTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    } => p.cubic_to(
                        coord(x1)?,
                        coord(y1)?,
                        coord(x2)?,
                        coord(y2)?,
                        coord(x)?,
                        coord(y)?,
                    ),
                    Quadratic { x1, y1, x, y } => {
                        p.quad_to(coord(x1)?, coord(y1)?, coord(x)?, coord(y)?)
                    }
                    ClosePath => p.close(),
                }
            }
        }
        "rect" => {
            let (x, y, w, h) = (
                dim("x", view.0)?,
                dim("y", view.1)?,
                dim("width", view.0)?,
                dim("height", view.1)?,
            );
            if w < 0.0 || h < 0.0 {
                return Err(Error::Invalid);
            }
            if w == 0.0 || h == 0.0 {
                return Ok(None);
            }
            let rx = length(
                n.attribute("rx").or(n.attribute("ry")).unwrap_or("0"),
                view.0,
            )?
            .min(w / 2.0);
            let ry = length(
                n.attribute("ry").or(n.attribute("rx")).unwrap_or("0"),
                view.1,
            )?
            .min(h / 2.0);
            if rx < 0.0 || ry < 0.0 {
                return Err(Error::Invalid);
            }
            if rx == 0.0 || ry == 0.0 {
                p.push_rect(Rect::from_xywh(x, y, w, h).ok_or(Error::Invalid)?);
            } else {
                let k = 0.5522848;
                p.move_to(x + rx, y);
                p.line_to(x + w - rx, y);
                p.cubic_to(
                    x + w - rx + k * rx,
                    y,
                    x + w,
                    y + ry - k * ry,
                    x + w,
                    y + ry,
                );
                p.line_to(x + w, y + h - ry);
                p.cubic_to(
                    x + w,
                    y + h - ry + k * ry,
                    x + w - rx + k * rx,
                    y + h,
                    x + w - rx,
                    y + h,
                );
                p.line_to(x + rx, y + h);
                p.cubic_to(
                    x + rx - k * rx,
                    y + h,
                    x,
                    y + h - ry + k * ry,
                    x,
                    y + h - ry,
                );
                p.line_to(x, y + ry);
                p.cubic_to(x, y + ry - k * ry, x + rx - k * rx, y, x + rx, y);
                p.close();
            }
        }
        "circle" | "ellipse" => {
            let (cx, cy) = (dim("cx", view.0)?, dim("cy", view.1)?);
            let (rx, ry) = if n.tag_name().name() == "circle" {
                let r = dim(
                    "r",
                    libm::sqrtf(view.0 * view.0 + view.1 * view.1) / core::f32::consts::SQRT_2,
                )?;
                (r, r)
            } else {
                (dim("rx", view.0)?, dim("ry", view.1)?)
            };
            if rx < 0.0 || ry < 0.0 {
                return Err(Error::Invalid);
            }
            if rx == 0.0 || ry == 0.0 {
                return Ok(None);
            }
            p.push_oval(
                Rect::from_xywh(cx - rx, cy - ry, rx * 2.0, ry * 2.0).ok_or(Error::Invalid)?,
            );
        }
        "line" => {
            p.move_to(dim("x1", view.0)?, dim("y1", view.1)?);
            p.line_to(dim("x2", view.0)?, dim("y2", view.1)?);
        }
        "polyline" | "polygon" => {
            let mut count = 0;
            let mut numbers = svgtypes::NumberListParser::from(n.attribute("points").unwrap_or(""));
            while let Some(x) = numbers.next() {
                *budget += 1;
                if *budget > MAX_SEGMENTS {
                    return Err(Error::Limit);
                }
                let x = x.map_err(|_| Error::Invalid)?;
                let y = numbers
                    .next()
                    .ok_or(Error::Invalid)?
                    .map_err(|_| Error::Invalid)?;
                if count == 0 {
                    p.move_to(coord(x)?, coord(y)?)
                } else {
                    p.line_to(coord(x)?, coord(y)?)
                }
                count += 1;
            }
            if n.tag_name().name() == "polygon" {
                p.close();
            }
        }
        _ => return Err(Error::Unsupported),
    }
    let path = p.finish();
    if path.as_ref().is_some_and(|p| {
        p.points()
            .iter()
            .any(|v| !v.x.is_finite() || !v.y.is_finite())
    }) {
        return Err(Error::Invalid);
    }
    Ok(path)
}
