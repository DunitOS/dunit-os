//! Wallpaper, icons, font-independent sampling and configured widget resources.
use super::*;
// --- Desktop wallpaper (ported from the legacy Stack A renderer) ---------
// The backdrop image lives in the (read-only) asset VFS as a 1600x900 24-bit
// BMP. It is loaded and pre-scaled to the framebuffer once at session start
// into an XRGB8888 buffer the size of `back`, so compositing the desktop each
// frame is a single `copy_from_slice` (no per-pixel BMP sampling in the hot
// loop). A missing/garbage file yields `None` and the gradient fallback stays.
// The path is no longer hardcoded here: it comes from `[desktop] wallpaper` in
// the config (baseline `/assets/wallpapers/wallpaper.bmp`).
pub(super) const WALLPAPER_WIDTH: usize = 1920;
pub(super) const WALLPAPER_HEIGHT: usize = 1080;
pub(super) const WALLPAPER_OFFSET: usize = 54;
pub(super) const WALLPAPER_STRIDE: usize = WALLPAPER_WIDTH * 3;

/// Slurp a whole (binary) VFS file into a byte vector. Unlike
/// `settings::read_file` this imposes no UTF-8 requirement and a larger cap, so
/// it can carry the multi-megabyte wallpaper. `None` on open error.
pub(super) fn read_binary(path: &str, cap: usize) -> Option<Vec<u8>> {
    let fd = libdunit::open(path, libdunit::OPEN_READ);
    if fd < 0 {
        return None;
    }
    let fd = fd as usize;
    let mut data: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = libdunit::read(fd, &mut chunk);
        if n <= 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n as usize]);
        if data.len() > cap {
            break;
        }
    }
    libdunit::close(fd);
    Some(data)
}

/// A 1600x900 24-bit BMP: "BM" magic, 54-byte pixel offset, matching
/// dimensions and 24 bpp. Mirrors the legacy `validate_wallpaper_bmp`.
pub(super) fn validate_wallpaper_bmp(data: &[u8]) -> bool {
    data.len() >= WALLPAPER_OFFSET + WALLPAPER_STRIDE * WALLPAPER_HEIGHT
        && data[0] == b'B'
        && data[1] == b'M'
        && data.get(10).copied() == Some(WALLPAPER_OFFSET as u8)
        && data.get(18).copied() == Some((WALLPAPER_WIDTH & 0xff) as u8)
        && data.get(19).copied() == Some(((WALLPAPER_WIDTH >> 8) & 0xff) as u8)
        && data.get(22).copied() == Some((WALLPAPER_HEIGHT & 0xff) as u8)
        && data.get(23).copied() == Some(((WALLPAPER_HEIGHT >> 8) & 0xff) as u8)
        && data.get(28).copied() == Some(24)
}

/// Rasterize SVG at the framebuffer size, or scale the legacy wallpaper BMP.
/// Invalid/missing assets leave the configured fallback backdrop in place.
pub(super) fn load_wallpaper(
    path: &str,
    bw: usize,
    bh: usize,
    background: u32,
) -> Option<Vec<u32>> {
    if path.ends_with(".svg") {
        let mut image = image_result(path, dunit_image::load(path, bw as u32, bh as u32))?;
        for pixel in &mut image.pixels {
            *pixel = blend(background, *pixel);
        }
        libdunit::println(&alloc::format!(
            "gui_server: SVG wallpaper rendered {}x{}",
            bw,
            bh
        ));
        return Some(image.pixels);
    }
    let data = read_binary(path, 8 * 1024 * 1024)?;
    if !validate_wallpaper_bmp(&data) {
        return None;
    }
    let mut out: Vec<u32> = Vec::new();
    out.resize(bw * bh, background);
    for y in 0..bh {
        let src_y = y.saturating_mul(WALLPAPER_HEIGHT) / bh.max(1);
        let bmp_y = WALLPAPER_HEIGHT
            .saturating_sub(1)
            .saturating_sub(src_y.min(WALLPAPER_HEIGHT - 1));
        let row_off = WALLPAPER_OFFSET + bmp_y * WALLPAPER_STRIDE;
        let dst_base = y * bw;
        for x in 0..bw {
            let src_x = x.saturating_mul(WALLPAPER_WIDTH) / bw.max(1);
            let off = row_off + src_x.min(WALLPAPER_WIDTH - 1) * 3;
            let b = data[off] as u32;
            let g = data[off + 1] as u32;
            let r = data[off + 2] as u32;
            out[dst_base + x] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
        }
    }
    Some(out)
}

/// Resolve SVG at the configured dock size; bitmap fallback retains its own size.
pub(super) fn resolve_app_icon(
    icon: &str,
    theme: &mut Option<dunit_image::IconTheme>,
    side: u32,
) -> Option<dunit_image::Image> {
    if icon.is_empty() {
        return None;
    }
    let result = if icon.starts_with('/') {
        dunit_image::load(icon, side, side)
    } else {
        theme.as_mut()?.icon(icon, side)
    };
    image_result(icon, result)
}

pub(super) fn load_icon_theme(name: &str) -> Option<dunit_image::IconTheme> {
    match dunit_image::IconTheme::open(name) {
        Ok(theme) => Some(theme),
        Err(error) => {
            libdunit::println(&alloc::format!(
                "gui_server: icon theme rejected {} {:?}",
                name,
                error
            ));
            None
        }
    }
}

pub(super) fn image_result(
    path: &str,
    result: Result<dunit_image::Image, dunit_image::Error>,
) -> Option<dunit_image::Image> {
    match result {
        Ok(image) => Some(image),
        Err(dunit_image::Error::Missing) => None,
        Err(error) => {
            libdunit::println(&alloc::format!(
                "gui_server: image rejected {} {:?}",
                path,
                error
            ));
            None
        }
    }
}

/// Alpha-blend a packed-ARGB icon (`src`, `sw`×`sh`) into the back buffer,
/// resampled to `dw`×`dh` at `(dx,dy)`. SVG caches carry their render dimensions;
/// legacy Breeze sources are 32×32. Dock and launcher cells use different sizes,
/// so a nearest-neighbor pick visibly aliases. Instead we resample with alpha in
/// mind: bilinear when enlarging (both axes ≥ source), box-average when shrinking.
/// Both paths interpolate in PREMULTIPLIED alpha — RGB is weighted by each texel's
/// alpha before averaging and un-premultiplied at the end — so the black of the
/// fully-transparent texels around a non-square silhouette never bleeds a dark
/// fringe into the edges. A resolved alpha of 0 is skipped, so silhouettes still
/// composite cleanly over the dock buttons / menu rows.
pub(super) fn blit_icon(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    src: &[u32],
    sw: usize,
    sh: usize,
    dx: i32,
    dy: i32,
    dw: i32,
    dh: i32,
) {
    if dw <= 0 || dh <= 0 || sw == 0 || sh == 0 || src.len() < sw * sh {
        return;
    }
    let (dw, dh) = (dw as usize, dh as usize);
    // Enlarging (or 1:1) on both axes → bilinear; otherwise → area/box average.
    let upscale = dw >= sw && dh >= sh;
    for ry in 0..dh {
        for rx in 0..dw {
            let px = if upscale {
                sample_bilinear(src, sw, sh, rx, ry, dw, dh)
            } else {
                sample_area(src, sw, sh, rx, ry, dw, dh)
            };
            if (px >> 24) & 0xFF == 0 {
                continue;
            }
            blend_pixel(buf, bw, bh, dx + rx as i32, dy + ry as i32, px);
        }
    }
}

/// Un-premultiply an accumulated (Σα, Σr·α, Σg·α, Σb·α) into a straight-alpha ARGB
/// pixel. `weight` is the total filter weight (Σ of the per-texel weights) used to
/// normalise alpha; RGB is normalised by `asum` (the alpha-weighted mass) so the
/// colour of transparent texels contributes nothing. `weight == 0` yields fully
/// transparent. Integer-only (no soft-float in the no_std ELF).
#[inline]
pub(super) fn unpremul(asum: u64, rsum: u64, gsum: u64, bsum: u64, weight: u64) -> u32 {
    if weight == 0 {
        return 0;
    }
    let a = (asum / weight).min(255) as u32;
    if asum == 0 {
        return 0; // every covered texel was transparent
    }
    let r = (rsum / asum).min(255) as u32;
    let g = (gsum / asum).min(255) as u32;
    let b = (bsum / asum).min(255) as u32;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Bilinear sample of the four texels around the source point that maps to dest
/// `(rx,ry)`, using centre-aligned mapping so a 1:1 scale reproduces the source
/// exactly. Fixed-point (1/256) weights, premultiplied by alpha.
#[inline]
pub(super) fn sample_bilinear(
    src: &[u32],
    sw: usize,
    sh: usize,
    rx: usize,
    ry: usize,
    dw: usize,
    dh: usize,
) -> u32 {
    // fx = (rx + 0.5) * sw/dw - 0.5, in 1/256 units, clamped to ≥ 0.
    let fx = (((2 * rx + 1) * sw * 128) / dw).saturating_sub(128);
    let fy = (((2 * ry + 1) * sh * 128) / dh).saturating_sub(128);
    let x0 = (fx >> 8).min(sw - 1);
    let y0 = (fy >> 8).min(sh - 1);
    let x1 = (x0 + 1).min(sw - 1);
    let y1 = (y0 + 1).min(sh - 1);
    let tx = (fx & 0xFF) as u64; // 0..255 fractional weight toward x1
    let ty = (fy & 0xFF) as u64;
    let wx0 = 256 - tx;
    let wy0 = 256 - ty;
    let w = [wx0 * wy0, tx * wy0, wx0 * ty, tx * ty]; // 00,10,01,11 — Σ = 65536
    let p = [
        src[y0 * sw + x0],
        src[y0 * sw + x1],
        src[y1 * sw + x0],
        src[y1 * sw + x1],
    ];
    let (mut asum, mut rsum, mut gsum, mut bsum) = (0u64, 0u64, 0u64, 0u64);
    for i in 0..4 {
        let a = ((p[i] >> 24) & 0xFF) as u64;
        let aw = a * w[i];
        asum += aw;
        rsum += ((p[i] >> 16) & 0xFF) as u64 * aw;
        gsum += ((p[i] >> 8) & 0xFF) as u64 * aw;
        bsum += (p[i] & 0xFF) as u64 * aw;
    }
    unpremul(asum, rsum, gsum, bsum, 65536)
}

/// Box/area sample: average every source texel covered by dest `(rx,ry)` (at least
/// one), premultiplied by alpha. Correct downscale filter for shrinking 32×32 icons.
#[inline]
pub(super) fn sample_area(
    src: &[u32],
    sw: usize,
    sh: usize,
    rx: usize,
    ry: usize,
    dw: usize,
    dh: usize,
) -> u32 {
    let sx0 = (rx * sw) / dw;
    let sy0 = (ry * sh) / dh;
    let sx1 = (((rx + 1) * sw + dw - 1) / dw).max(sx0 + 1).min(sw);
    let sy1 = (((ry + 1) * sh + dh - 1) / dh).max(sy0 + 1).min(sh);
    let (mut asum, mut rsum, mut gsum, mut bsum, mut count) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for sy in sy0..sy1 {
        for sx in sx0..sx1 {
            let px = src[sy * sw + sx];
            let a = ((px >> 24) & 0xFF) as u64;
            asum += a;
            rsum += ((px >> 16) & 0xFF) as u64 * a;
            gsum += ((px >> 8) & 0xFF) as u64 * a;
            bsum += (px & 0xFF) as u64 * a;
            count += 1;
        }
    }
    unpremul(asum, rsum, gsum, bsum, count)
}

/// A desktop widget kind the compositor knows how to draw. The renderer owns the
/// per-kind drawing code (a clock formats time, a monitor reads system stats);
/// the CONFIG owns which kinds are shown and where (per-widget `widgets/*.toml`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WidgetKind {
    Clock,
    Monitor,
}

/// One resolved desktop widget: its kind plus the placement/accent resolved from
/// its per-widget file (falling back to the `[widgets] corner` default / theme).
#[derive(Clone, Copy)]
pub(super) struct DeskWidget {
    pub(super) kind: WidgetKind,
    pub(super) corner: i32,
    pub(super) accent: u32,
    pub(super) clock: settings::WidgetCfg,
}

/// Resolve the config-driven desktop-widget set. Each widget kind the compositor
/// implements is included ONLY when its `widgets/<name>.toml` exists AND is
/// enabled — so a widget is removed by deleting its file or setting
/// `enabled = false` (the Phase 8 removal switch; single source of truth, no
/// `[widgets] clock/monitor` bool). Per-widget `corner < 0` inherits
/// `default_corner` (`[widgets] corner`); `accent == 0` inherits the theme accent
/// at draw time. Called once at bring-up and again on each live config reload —
/// never per frame, so the per-widget files are not re-read every tick.
pub(super) fn resolve_desk_widgets(default_corner: i32) -> Vec<DeskWidget> {
    // The compositor's registry of IMPLEMENTED widget kinds. This is renderer
    // implementation (each kind has bespoke draw code below), not desktop policy:
    // policy — which of these are actually shown — lives in the per-widget files.
    let known: [(&str, WidgetKind); 2] = [
        ("clock", WidgetKind::Clock),
        ("monitor", WidgetKind::Monitor),
    ];
    let mut v: Vec<DeskWidget> = Vec::new();
    for (name, kind) in known {
        let wc = settings::WidgetCfg::load(name);
        if wc.from_file && wc.enabled {
            let corner = if wc.corner >= 0 {
                wc.corner.min(3)
            } else {
                default_corner
            };
            v.push(DeskWidget {
                kind,
                corner,
                accent: wc.accent,
                clock: wc,
            });
        }
    }
    v
}

/// Apply the configured `[display]` resolution via the kernel display backend.
/// `0/0` (baseline) keeps the current/boot resolution. A fixed backend (Limine/
/// GOP) reports EOPNOTSUPP (-95) — the desktop simply stays at the boot mode.
/// This is the compositor's mechanism→policy seam: the KERNEL owns the mode-set,
/// the CONFIG owns which mode (CLAUDE.md §6-10).
pub(super) fn apply_display_mode(d: &settings::Display) {
    if d.width == 0 || d.height == 0 {
        return;
    }
    let r = libdunit::set_video_mode(d.width, d.height);
    if r == 0 {
        libdunit::println(&alloc::format!(
            "gui_server: [display] resolution set to {}x{}",
            d.width,
            d.height
        ));
    } else if r == -95 {
        libdunit::println("gui_server: [display] fixed backend — boot resolution kept");
    } else {
        libdunit::println(&alloc::format!(
            "gui_server: [display] set_video_mode {}x{} rejected ({})",
            d.width,
            d.height,
            r
        ));
    }
}
