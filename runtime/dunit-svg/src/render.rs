use crate::{
    geometry,
    paint::{opacity, prop, Style},
};
use crate::{length, Error, Result, MAX_DEPTH, MAX_NODES};
use alloc::vec::Vec;
use roxmltree::{Document, Node, ParsingOptions};
use tiny_skia::{Paint, Pixmap, PixmapPaint, Transform};

const MAX_PIXELS: u64 = 12 * 1024 * 1024;

/// Render a bounded static SVG to straight-alpha ARGB8888 at the requested size.
/// Unsupported active/external content is rejected before drawing any pixels.
pub fn render(source: &str, width: u32, height: u32) -> Result<Vec<u32>> {
    let pixels = width as u64 * height as u64;
    if source.len() > 1024 * 1024
        || width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || pixels > MAX_PIXELS
    {
        return Err(Error::Limit);
    }
    let doc = Document::parse_with_options(
        source,
        ParsingOptions {
            nodes_limit: MAX_NODES,
            ..ParsingOptions::default()
        },
    )
    .map_err(|_| Error::Xml)?;
    if doc.descendants().any(|n| n.is_pi()) {
        return Err(Error::Unsupported);
    }
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(Error::Invalid);
    }
    validate(root, 0)?;
    let mut css_bytes = 0usize;
    let mut css_rules = 0usize;
    for sheet in doc.descendants().filter(|n| crate::css::stylesheet(*n)) {
        let text = sheet.text().unwrap_or("");
        css_bytes += text.len();
        css_rules += crate::css::rules(text).count();
        if css_bytes > 65536 || css_rules > 128 {
            return Err(Error::Limit);
        }
    }

    // IDs must be unique so paint references cannot depend on traversal order.
    let mut ids = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        if let Some(id) = n.attribute("id") {
            if ids.contains(&id) {
                return Err(Error::Invalid);
            }
            ids.push(id);
        }
    }
    let (x, y, w, h) = if let Some(v) = root.attribute("viewBox") {
        let v: svgtypes::ViewBox = v.parse().map_err(|_| Error::Invalid)?;
        (v.x as f32, v.y as f32, v.w as f32, v.h as f32)
    } else {
        (
            0.0,
            0.0,
            length(root.attribute("width").ok_or(Error::Invalid)?, width as f32)?,
            length(
                root.attribute("height").ok_or(Error::Invalid)?,
                height as f32,
            )?,
        )
    };
    if [x, y, w, h].iter().any(|v| !v.is_finite() || v.abs() > 1e7) || w <= 0.0 || h <= 0.0 {
        return Err(Error::Invalid);
    }
    let ratio: svgtypes::AspectRatio = root
        .attribute("preserveAspectRatio")
        .unwrap_or("xMidYMid meet")
        .parse()
        .map_err(|_| Error::Invalid)?;
    let (mut sx, mut sy) = (width as f32 / w, height as f32 / h);
    let (mut dx, mut dy) = (0.0, 0.0);
    use svgtypes::Align::*;
    if ratio.align != None {
        let scale = if ratio.slice { sx.max(sy) } else { sx.min(sy) };
        sx = scale;
        sy = scale;
        let (rx, ry) = (width as f32 - w * scale, height as f32 - h * scale);
        dx = match ratio.align {
            XMidYMin | XMidYMid | XMidYMax => rx / 2.0,
            XMaxYMin | XMaxYMid | XMaxYMax => rx,
            _ => 0.0,
        };
        dy = match ratio.align {
            XMinYMid | XMidYMid | XMaxYMid => ry / 2.0,
            XMinYMax | XMidYMax | XMaxYMax => ry,
            _ => 0.0,
        };
    }
    let ts = Transform::from_row(sx, 0.0, 0.0, sy, dx - x * sx, dy - y * sy);
    let mut pix = Pixmap::new(width, height).ok_or(Error::Allocation)?;
    let mut ctx = Context {
        view: (w, h),
        draws: 0,
        segments: 0,
        layer_pixels: pixels,
    };
    ctx.node(root, &Style::default(), ts, &mut pix, 0, true)?;
    Ok(pix
        .pixels()
        .iter()
        .map(|p| {
            let c = p.demultiply();
            ((c.alpha() as u32) << 24)
                | ((c.red() as u32) << 16)
                | ((c.green() as u32) << 8)
                | c.blue() as u32
        })
        .collect())
}

fn validate(n: Node<'_, '_>, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::Limit);
    }
    if crate::css::metadata(n) {
        return Ok(());
    }
    if n.tag_name()
        .namespace()
        .is_some_and(|v| v != "http://www.w3.org/2000/svg")
    {
        return Err(Error::Unsupported);
    }
    let name = n.tag_name().name();
    if !matches!(
        name,
        "svg"
            | "g"
            | "defs"
            | "title"
            | "desc"
            | "path"
            | "rect"
            | "circle"
            | "ellipse"
            | "line"
            | "polygon"
            | "polyline"
            | "linearGradient"
            | "radialGradient"
            | "stop"
            | "use"
            | "metadata"
            | "style"
            | "filter"
            | "feGaussianBlur"
    ) {
        return Err(Error::Unsupported);
    }
    if name == "style" {
        for rule in crate::css::rules(n.text().unwrap_or("")) {
            let (_, body) = rule?;
            for declaration in body.split(';').filter(|s| !s.trim().is_empty()) {
                let (k, v) = declaration.split_once(':').ok_or(Error::Invalid)?;
                if !style_value(k.trim(), v.trim()) {
                    return Err(Error::Unsupported);
                }
            }
        }
    }
    for a in n.attributes() {
        if matches!(
            a.namespace(),
            Some(
                "http://www.inkscape.org/namespaces/inkscape"
                    | "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
            )
        ) {
            continue;
        }
        if a.namespace() == Some("http://www.w3.org/XML/1998/namespace") && a.name() == "space" {
            continue;
        }

        if a.namespace()
            .is_some_and(|v| v != "http://www.w3.org/1999/xlink")
        {
            return Err(Error::Unsupported);
        }
        if a.name() == "href" && !matches!(name, "use" | "linearGradient" | "radialGradient") {
            return Err(Error::Unsupported);
        }
        if a.name() == "style" {
            for declaration in a.value().split(';').filter(|s| !s.trim().is_empty()) {
                let (k, v) = declaration.split_once(':').ok_or(Error::Invalid)?;
                if !style_value(k.trim(), v.trim()) {
                    return Err(Error::Unsupported);
                }
            }
        } else if !style_value(a.name(), a.value())
            && !matches!(
                a.name(),
                "id" | "version"
                    | "class"
                    | "type"
                    | "stdDeviation"
                    | "filterUnits"
                    | "primitiveUnits"
                    | "color-interpolation-filters"
                    | "viewBox"
                    | "preserveAspectRatio"
                    | "width"
                    | "height"
                    | "x"
                    | "y"
                    | "x1"
                    | "x2"
                    | "y1"
                    | "y2"
                    | "cx"
                    | "cy"
                    | "r"
                    | "rx"
                    | "ry"
                    | "fx"
                    | "fy"
                    | "d"
                    | "points"
                    | "gradientUnits"
                    | "gradientTransform"
                    | "spreadMethod"
                    | "offset"
                    | "href"
            )
        {
            return Err(Error::Unsupported);
        }
    }
    if name == "svg" && n.parent_element().is_some() {
        return Err(Error::Unsupported);
    }
    for child in n.children().filter(|n| n.is_element()) {
        validate(child, depth + 1)?;
    }
    Ok(())
}
fn style_key(k: &str) -> bool {
    matches!(
        k,
        "filter"
            | "paint-order"
            | "fill"
            | "fill-rule"
            | "fill-opacity"
            | "stroke"
            | "stroke-opacity"
            | "stroke-width"
            | "stroke-linecap"
            | "stroke-linejoin"
            | "stroke-miterlimit"
            | "stroke-dasharray"
            | "stroke-dashoffset"
            | "color"
            | "opacity"
            | "transform"
            | "display"
            | "visibility"
            | "stop-color"
            | "stop-opacity"
    )
}

/// Neutral presentation defaults emitted by SVG editors do not alter this
/// profile. Non-default effects still fail instead of being silently discarded.
fn style_value(k: &str, v: &str) -> bool {
    style_key(k)
        || matches!(
            (k, v),
            ("clip-rule", "nonzero" | "evenodd")
                | ("color-interpolation", "sRGB" | "auto")
                | ("color-interpolation-filters", "sRGB" | "linearRGB" | "auto")
                | (
                    "color-rendering" | "image-rendering" | "shape-rendering" | "text-rendering",
                    "auto"
                )
                | ("vector-effect", "none")
                | ("isolation", "auto")
                | ("mix-blend-mode" | "filter-blend-mode", "normal")
                | ("filter-gaussianBlur-deviation", "0")
                | ("marker", "none")
                | ("overflow", "visible")
                | ("enable-background", "accumulate")
                | ("solid-color", "#000000")
                | ("solid-opacity", "1")
        )
}

struct Context {
    view: (f32, f32),
    draws: usize,
    segments: usize,
    layer_pixels: u64,
}
impl Context {
    fn node<'a>(
        &mut self,
        n: Node<'a, '_>,
        inherited: &Style<'a>,
        parent: Transform,
        pix: &mut Pixmap,
        depth: usize,
        visible: bool,
    ) -> Result<()> {
        if depth > MAX_DEPTH || self.draws >= MAX_NODES as usize {
            return Err(Error::Limit);
        }
        self.draws += 1;
        if crate::css::metadata(n) {
            return Ok(());
        }
        let name = n.tag_name().name();
        if matches!(
            name,
            "defs"
                | "title"
                | "desc"
                | "linearGradient"
                | "radialGradient"
                | "stop"
                | "style"
                | "filter"
        ) {
            return Ok(());
        }
        if let Some(display) = prop(n, "display") {
            match display {
                "none" => return Ok(()),
                "inline" => {}
                _ => return Err(Error::Unsupported),
            }
        }
        let visible = match prop(n, "visibility") {
            Some("hidden" | "collapse") => false,
            Some("visible") => true,
            Some("inherit") | None => visible,
            _ => return Err(Error::Invalid),
        };
        let style = inherited.inherit(n, self.view)?;
        let ts = parent.pre_concat(geometry::transform(prop(n, "transform"))?);
        let alpha = opacity(prop(n, "opacity"))?;
        let filter = prop(n, "filter").filter(|v| *v != "none");
        if alpha < 1.0 || filter.is_some() {
            let pixels = pix.width() as u64 * pix.height() as u64;
            if self.layer_pixels + pixels > MAX_PIXELS * 2 {
                return Err(Error::Limit);
            }
            self.layer_pixels += pixels;
            let mut layer = Pixmap::new(pix.width(), pix.height()).ok_or(Error::Allocation)?;
            self.content(n, &style, ts, &mut layer, depth, visible)?;
            if let Some(filter) = filter {
                crate::filter::apply(n, filter, ts, self.view, &mut layer)?;
            }
            pix.draw_pixmap(
                0,
                0,
                layer.as_ref(),
                &PixmapPaint {
                    opacity: alpha,
                    ..PixmapPaint::default()
                },
                Transform::identity(),
                None,
            );
            self.layer_pixels -= pixels;
        } else {
            self.content(n, &style, ts, pix, depth, visible)?;
        }
        Ok(())
    }
    fn content<'a>(
        &mut self,
        n: Node<'a, '_>,
        style: &Style<'a>,
        ts: Transform,
        pix: &mut Pixmap,
        depth: usize,
        visible: bool,
    ) -> Result<()> {
        match n.tag_name().name() {
            "svg" | "g" => {
                for child in n.children().filter(|n| n.is_element()) {
                    self.node(child, style, ts, pix, depth + 1, visible)?;
                }
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
                let (x, y) = (
                    length(n.attribute("x").unwrap_or("0"), self.view.0)?,
                    length(n.attribute("y").unwrap_or("0"), self.view.1)?,
                );
                // SVG width/height on use only affect svg/symbol viewports.
                // Referenced geometry and groups retain their own coordinates.
                if matches!(target.tag_name().name(), "svg" | "symbol") {
                    return Err(Error::Unsupported);
                }
                self.node(
                    target,
                    style,
                    ts.pre_translate(x, y),
                    pix,
                    depth + 1,
                    visible,
                )?;
            }
            _ => {
                if !visible {
                    return Ok(());
                }
                if let Some(path) = geometry::shape(n, self.view, &mut self.segments)? {
                    let bounds = path.compute_tight_bounds().ok_or(Error::Invalid)?;
                    for stroke in if style.stroke_first {
                        [true, false]
                    } else {
                        [false, true]
                    } {
                        if stroke && style.pen.width <= 0.0 {
                            continue;
                        }
                        let (value, alpha) = if stroke {
                            (style.stroke, style.stroke_alpha)
                        } else {
                            (style.fill, style.fill_alpha)
                        };
                        if let Some(shader) =
                            crate::paint::shader(value, alpha, style.color, n, bounds, self.view)?
                        {
                            let paint = Paint {
                                shader,
                                ..Paint::default()
                            };
                            if stroke {
                                pix.stroke_path(&path, &paint, &style.pen, ts, None);
                            } else {
                                pix.fill_path(&path, &paint, style.rule, ts, None);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
