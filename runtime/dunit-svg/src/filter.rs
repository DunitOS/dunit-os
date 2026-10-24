//! Bounded static Gaussian blur over SourceGraphic, with SVG filter regions.
use crate::{geometry, length, paint::prop, Error, Result, MAX_DEPTH, MAX_NODES};
use alloc::{vec, vec::Vec};
use roxmltree::Node;
use tiny_skia::{Pixmap, Rect, Transform};

fn bounds(
    n: Node<'_, '_>,
    view: (f32, f32),
    ts: Transform,
    depth: usize,
    visits: &mut usize,
    segments: &mut usize,
) -> Result<Option<Rect>> {
    *visits += 1;
    if depth > MAX_DEPTH || *visits > MAX_NODES as usize {
        return Err(Error::Limit);
    }
    if crate::css::metadata(n)
        || matches!(n.tag_name().name(), "defs" | "style" | "title" | "desc")
        || prop(n, "display") == Some("none")
    {
        return Ok(None);
    }
    match n.tag_name().name() {
        "g" | "svg" => {
            let mut result: Option<Rect> = None;
            for child in n.children().filter(|n| n.is_element()) {
                let ts = ts.pre_concat(geometry::transform(prop(child, "transform"))?);
                if let Some(b) = bounds(child, view, ts, depth + 1, visits, segments)? {
                    result = Some(match result {
                        None => b,
                        Some(r) => Rect::from_ltrb(
                            r.left().min(b.left()),
                            r.top().min(b.top()),
                            r.right().max(b.right()),
                            r.bottom().max(b.bottom()),
                        )
                        .ok_or(Error::Invalid)?,
                    });
                }
            }
            Ok(result)
        }
        "use" => {
            let href = n
                .attribute("href")
                .or_else(|| n.attribute(("http://www.w3.org/1999/xlink", "href")))
                .ok_or(Error::Invalid)?;
            let id = href.strip_prefix('#').ok_or(Error::Unsupported)?;
            let target = n
                .document()
                .descendants()
                .find(|n| n.attribute("id") == Some(id))
                .ok_or(Error::Invalid)?;
            let ts = ts
                .pre_translate(
                    length(n.attribute("x").unwrap_or("0"), view.0)?,
                    length(n.attribute("y").unwrap_or("0"), view.1)?,
                )
                .pre_concat(geometry::transform(prop(target, "transform"))?);
            bounds(target, view, ts, depth + 1, visits, segments)
        }
        _ => Ok(geometry::shape(n, view, segments)?
            .and_then(|p| p.transform(ts))
            .and_then(|p| p.compute_tight_bounds())),
    }
}
fn linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        libm::powf((v + 0.055) / 1.055, 2.4)
    }
}
fn srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * libm::powf(v, 1.0 / 2.4) - 0.055
    }
}

pub fn apply(
    n: Node<'_, '_>,
    value: &str,
    ts: Transform,
    view: (f32, f32),
    pix: &mut Pixmap,
) -> Result<()> {
    let id = value
        .strip_prefix("url(#")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or(Error::Unsupported)?;
    let f = n
        .document()
        .descendants()
        .find(|n| n.attribute("id") == Some(id))
        .ok_or(Error::Invalid)?;
    if f.tag_name().name() != "filter" {
        return Err(Error::Invalid);
    }
    // An axis-aligned convolution cannot represent a skewed/rotated anisotropic
    // kernel. Reject those transforms rather than returning the wrong effect.
    if ts.kx != 0.0
        || ts.ky != 0.0
        || f.attribute("primitiveUnits")
            .is_some_and(|v| v != "userSpaceOnUse")
    {
        return Err(Error::Unsupported);
    }
    let children: Vec<_> = f.children().filter(|n| n.is_element()).collect();
    if children.len() != 1 || children[0].tag_name().name() != "feGaussianBlur" {
        return Err(Error::Unsupported);
    }
    let primitive = children[0];
    let values: Vec<_> =
        svgtypes::NumberListParser::from(primitive.attribute("stdDeviation").unwrap_or("0"))
            .collect();
    if values.is_empty() || values.len() > 2 {
        return Err(Error::Invalid);
    }
    let x = *values[0].as_ref().map_err(|_| Error::Invalid)? as f32;
    let y = *values
        .get(1)
        .unwrap_or(&values[0])
        .as_ref()
        .map_err(|_| Error::Invalid)? as f32;
    if x < 0.0 || y < 0.0 {
        return Err(Error::Invalid);
    }
    let (sx, sy) = (x * ts.sx.abs(), y * ts.sy.abs());
    if !sx.is_finite() || !sy.is_finite() || sx > 32.0 || sy > 32.0 {
        return Err(Error::Limit);
    }
    let (w, h) = (pix.width() as usize, pix.height() as usize);
    let work = w as u64
        * h as u64
        * 4
        * (2 * libm::ceilf(sx * 3.0) as u64 + 2 * libm::ceilf(sy * 3.0) as u64 + 2);
    if w * h > 1024 * 1024 || work > 64 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let mut visits = 0;
    let mut segments = 0;
    let Some(b) = bounds(
        n,
        view,
        Transform::identity(),
        0,
        &mut visits,
        &mut segments,
    )?
    else {
        return Ok(());
    };
    let object = match f.attribute("filterUnits").unwrap_or("objectBoundingBox") {
        "objectBoundingBox" => true,
        "userSpaceOnUse" => false,
        _ => return Err(Error::Invalid),
    };
    let coordinate = |k, default, axis: f32, origin: f32, size: f32| -> Result<f32> {
        let v = length(
            f.attribute(k).unwrap_or(default),
            if object { 1.0 } else { axis },
        )?;
        Ok(if object { origin + v * size } else { v })
    };
    let rx = coordinate("x", "-10%", view.0, b.x(), b.width())?;
    let ry = coordinate("y", "-10%", view.1, b.y(), b.height())?;
    let rw = coordinate("width", "120%", view.0, 0.0, b.width())?;
    let rh = coordinate("height", "120%", view.1, 0.0, b.height())?;
    if rw < 0.0 || rh < 0.0 {
        return Err(Error::Invalid);
    }
    if rw == 0.0 || rh == 0.0 {
        pix.data_mut().fill(0);
        return Ok(());
    }
    let region = Rect::from_xywh(rx, ry, rw, rh)
        .and_then(|r| r.transform(ts))
        .ok_or(Error::Invalid)?;
    let color_space = primitive
        .ancestors()
        .filter(|n| n.is_element())
        .find_map(|n| prop(n, "color-interpolation-filters"))
        .unwrap_or("linearRGB");
    let use_linear = match color_space {
        "linearRGB" => true,
        "sRGB" | "auto" => false,
        _ => return Err(Error::Invalid),
    };
    let mut data: Vec<[f32; 4]> = pix
        .data()
        .chunks_exact(4)
        .map(|p| {
            let a = p[3] as f32 / 255.0;
            let convert = |v: u8| {
                if use_linear && a > 0.0 {
                    linear((v as f32 / 255.0 / a).clamp(0.0, 1.0)) * a
                } else {
                    v as f32 / 255.0
                }
            };
            [convert(p[0]), convert(p[1]), convert(p[2]), a]
        })
        .collect();
    let mut out = vec![[0.0f32; 4]; data.len()];
    for (horizontal, sigma) in [(true, sx), (false, sy)] {
        if sigma == 0.0 {
            continue;
        }
        let radius = libm::ceilf(sigma * 3.0) as i32;
        let mut weights = Vec::new();
        let mut sum = 0.0;
        for i in -radius..=radius {
            let v = if i == 0 {
                1.0
            } else {
                libm::expf(-(i * i) as f32 / (2.0 * sigma * sigma))
            };
            weights.push(v);
            sum += v;
        }
        for v in &mut weights {
            *v /= sum;
        }
        out.fill([0.0; 4]);
        for y in 0..h {
            for x in 0..w {
                for (i, &weight) in weights.iter().enumerate() {
                    let d = i as i32 - radius;
                    let (px, py) = if horizontal {
                        (x as i32 + d, y as i32)
                    } else {
                        (x as i32, y as i32 + d)
                    };
                    if px >= 0 && py >= 0 && px < w as i32 && py < h as i32 {
                        for c in 0..4 {
                            out[y * w + x][c] += data[py as usize * w + px as usize][c] * weight;
                        }
                    }
                }
            }
        }
        core::mem::swap(&mut data, &mut out);
    }
    for (i, (src, dst)) in data
        .iter()
        .zip(pix.data_mut().chunks_exact_mut(4))
        .enumerate()
    {
        let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
        if x < region.left() || y < region.top() || x >= region.right() || y >= region.bottom() {
            dst.fill(0);
            continue;
        }
        let a = src[3];
        for c in 0..3 {
            let v = if use_linear && a > 0.0 {
                srgb((src[c] / a).clamp(0.0, 1.0)) * a
            } else {
                src[c]
            };
            dst[c] = libm::roundf(v * 255.0).clamp(0.0, 255.0) as u8;
        }
        dst[3] = libm::roundf(a * 255.0).clamp(0.0, 255.0) as u8;
    }
    Ok(())
}
