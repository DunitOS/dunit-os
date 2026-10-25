//! Shared software raster, blur, clipping and TrueType primitives.
use super::*;
pub(super) fn fill_rect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = ((x + w).max(0) as usize).min(bw);
    let y1 = ((y + h).max(0) as usize).min(bh);
    let mut yy = y0;
    while yy < y1 {
        let row = yy * bw;
        let mut xx = x0;
        while xx < x1 {
            buf[row + xx] = color;
            xx += 1;
        }
        yy += 1;
    }
}

// ===========================================================================
// Compositor visual effects (concept §5): alpha blending, gradients, rounded
// corners, soft shadows, backdrop blur. All integer math (no FP in userspace),
// all gated by `settings::Effects` so the flat look is one config flag away.
// ===========================================================================

/// Blend one ARGB pixel into the back buffer (bounds-checked).
#[inline]
pub(super) fn blend_pixel(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, src: u32) {
    if x < 0 || y < 0 || x as usize >= bw || y as usize >= bh {
        return;
    }
    let idx = y as usize * bw + x as usize;
    buf[idx] = blend(buf[idx], src);
}

/// Shift every RGB channel of `c` by `delta` (positive = lighten), clamped. Used
/// to derive gradient endpoints from a single theme color.
#[inline]
pub(super) fn shade(c: u32, delta: i32) -> u32 {
    let cl = |v: i32| v.clamp(0, 255) as u32;
    let r = ((c >> 16) & 0xFF) as i32 + delta;
    let g = ((c >> 8) & 0xFF) as i32 + delta;
    let b = (c & 0xFF) as i32 + delta;
    (cl(r) << 16) | (cl(g) << 8) | cl(b)
}

/// Linear RGB interpolation between `a` and `b`; `t` in 0..=255 (0 = a, 255 = b).
#[inline]
pub(super) fn lerp_color(a: u32, b: u32, t: u32) -> u32 {
    let t = t.min(255);
    let inv = 255 - t;
    let r = (((a >> 16) & 0xFF) * inv + ((b >> 16) & 0xFF) * t) / 255;
    let g = (((a >> 8) & 0xFF) * inv + ((b >> 8) & 0xFF) * t) / 255;
    let bl = ((a & 0xFF) * inv + (b & 0xFF) * t) / 255;
    (r << 16) | (g << 8) | bl
}

/// Blend a translucent solid ARGB rect into the buffer.
pub(super) fn fill_rect_alpha(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(bw as i32);
    let y1 = (y + h).min(bh as i32);
    let mut py = y0;
    while py < y1 {
        let mut px = x0;
        while px < x1 {
            blend_pixel(buf, bw, bh, px, py, color);
            px += 1;
        }
        py += 1;
    }
}

/// No animation clock means instant transitions, never a frozen half-open UI.
pub(super) fn animation_time_ms() -> Option<u64> {
    let mut stats = libdunit::SystemStats::default();
    if libdunit::get_system_stats(&mut stats) < 0 || stats.uptime_available == 0
        || stats.uptime_hz == 0 { return None; }
    Some((stats.uptime_ticks / stats.uptime_hz).saturating_mul(1000)
        .saturating_add((stats.uptime_ticks % stats.uptime_hz) * 1000 / stats.uptime_hz))
}

// Rounded-corner selection bitmask for `fill_rrect*` (which corners get the arc).
pub(super) const RR_TL: u8 = 1;
pub(super) const RR_TR: u8 = 2;
pub(super) const RR_BL: u8 = 4;
pub(super) const RR_BR: u8 = 8;
pub(super) const RR_ALL: u8 = RR_TL | RR_TR | RR_BL | RR_BR;
pub(super) const RR_TOP: u8 = RR_TL | RR_TR;

/// Coverage 0..255 of pixel (px,py) inside rounded-rect [x,y,w,h] radius `r`.
/// Straight edges and non-selected corners return 255; a selected corner is
/// 4x4-supersampled in 1/8-px units for a cheap anti-aliased arc (integer only).
pub(super) fn rr_cov(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32, r: i32, corners: u8) -> u32 {
    if r <= 0 {
        return 255;
    }
    let (cx, cy, bit);
    if px < x + r && py < y + r {
        cx = x + r;
        cy = y + r;
        bit = RR_TL;
    } else if px >= x + w - r && py < y + r {
        cx = x + w - r;
        cy = y + r;
        bit = RR_TR;
    } else if px < x + r && py >= y + h - r {
        cx = x + r;
        cy = y + h - r;
        bit = RR_BL;
    } else if px >= x + w - r && py >= y + h - r {
        cx = x + w - r;
        cy = y + h - r;
        bit = RR_BR;
    } else {
        return 255;
    }
    if corners & bit == 0 {
        return 255;
    }
    let r8 = (r * 8) as i64;
    let r2 = r8 * r8;
    let mut inside = 0u32;
    let mut sy = 0;
    while sy < 4 {
        let dy = (py * 8 + (sy * 2 + 1) - cy * 8) as i64;
        let mut sx = 0;
        while sx < 4 {
            let dx = (px * 8 + (sx * 2 + 1) - cx * 8) as i64;
            if dx * dx + dy * dy <= r2 {
                inside += 1;
            }
            sx += 1;
        }
        sy += 1;
    }
    inside * 255 / 16
}

/// Fill a rounded rect with a flat (alpha-aware) `color`. `corners` selects which
/// corners get the arc (e.g. `RR_TOP` for a title bar). `r <= 0` = plain rect.
pub(super) fn fill_rrect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, r: i32, corners: u8, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let r = r.max(0).min(w / 2).min(h / 2);
    let base_a = (color >> 24) & 0xFF;
    if base_a == 0 {
        return;
    }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(bw as i32);
    let y1 = (y + h).min(bh as i32);
    let mut py = y0;
    while py < y1 {
        let mut px = x0;
        while px < x1 {
            let cov = rr_cov(px, py, x, y, w, h, r, corners);
            if cov > 0 {
                let a = base_a * cov / 255;
                blend_pixel(buf, bw, bh, px, py, (a << 24) | (color & 0x00FF_FFFF));
            }
            px += 1;
        }
        py += 1;
    }
}

/// Fill a rounded rect with a vertical gradient (`top`..`bottom` RGB) at overall
/// opacity `alpha` (0..255), corner arcs anti-aliased. Used for glass panels,
/// title bars and buttons.
pub(super) fn fill_rrect_grad(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, r: i32, corners: u8, top: u32, bottom: u32, alpha: i32) {
    if w <= 0 || h <= 0 || alpha <= 0 {
        return;
    }
    let alpha = alpha.min(255) as u32;
    let r = r.max(0).min(w / 2).min(h / 2);
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(bw as i32);
    let y1 = (y + h).min(bh as i32);
    let span = (h - 1).max(1) as u32;
    let mut py = y0;
    while py < y1 {
        let t = ((py - y).max(0) as u32 * 255) / span;
        let row = lerp_color(top, bottom, t);
        let mut px = x0;
        while px < x1 {
            let cov = rr_cov(px, py, x, y, w, h, r, corners);
            if cov > 0 {
                let a = alpha * cov / 255;
                blend_pixel(buf, bw, bh, px, py, (a << 24) | (row & 0x00FF_FFFF));
            }
            px += 1;
        }
        py += 1;
    }
}

/// Layered soft drop shadow under a rounded rect: `spread` nested black rounded
/// rects, each at `alpha/spread`, so overlap accumulates into a soft falloff.
/// Offset down-right a touch for a light-from-above look. The opaque window
/// interior [x,y,w,h] is skipped — it gets overdrawn, so shadowing it is wasted
/// work (this is the bulk of the pixels, so skipping keeps the effect cheap).
pub(super) fn draw_soft_shadow(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, r: i32, spread: i32, alpha: i32) {
    if spread <= 0 || alpha <= 0 {
        return;
    }
    let per = ((alpha / spread).max(1)) as u32;
    let mut s = spread;
    while s >= 1 {
        let off = s / 2 + 2;
        let (lx, ly2, lw, lh, lr) = (x - s, y - s + off, w + 2 * s, h + 2 * s, r + s);
        let x0 = lx.max(0);
        let y0 = ly2.max(0);
        let x1 = (lx + lw).min(bw as i32);
        let y1 = (ly2 + lh).min(bh as i32);
        let mut py = y0;
        while py < y1 {
            let in_row = py >= y && py < y + h;
            let mut px = x0;
            while px < x1 {
                if in_row && px >= x && px < x + w {
                    px = x + w; // jump past the opaque window interior
                    continue;
                }
                let cov = rr_cov(px, py, lx, ly2, lw, lh, lr, RR_ALL);
                if cov > 0 {
                    blend_pixel(buf, bw, bh, px, py, (per * cov / 255) << 24);
                }
                px += 1;
            }
            py += 1;
        }
        s -= 1;
    }
}

/// One horizontal box-blur pass: `src` -> `dst` (both `w*h` packed), radius `r`,
/// sliding-window average of RGB with edge clamp. Alpha forced opaque.
pub(super) fn box_h(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize) {
    for y in 0..h {
        let row = y * w;
        let mut sr = 0u32;
        let mut sg = 0u32;
        let mut sb = 0u32;
        let first_hi = r.min(w - 1);
        for i in 0..=first_hi {
            let p = src[row + i];
            sr += (p >> 16) & 0xFF;
            sg += (p >> 8) & 0xFF;
            sb += p & 0xFF;
        }
        let mut lo = 0i32;
        let mut hi = first_hi as i32;
        for x in 0..w {
            let cnt = (hi - lo + 1) as u32;
            dst[row + x] = 0xFF00_0000 | ((sr / cnt) << 16) | ((sg / cnt) << 8) | (sb / cnt);
            let add = x as i32 + 1 + r as i32;
            if add < w as i32 {
                let p = src[row + add as usize];
                sr += (p >> 16) & 0xFF;
                sg += (p >> 8) & 0xFF;
                sb += p & 0xFF;
                hi = add;
            }
            let rem = x as i32 - r as i32;
            if rem >= 0 {
                let p = src[row + rem as usize];
                sr -= (p >> 16) & 0xFF;
                sg -= (p >> 8) & 0xFF;
                sb -= p & 0xFF;
                lo = rem + 1;
            }
        }
    }
}

/// One vertical box-blur pass: `src` -> `dst` (both `w*h` packed), radius `r`.
pub(super) fn box_v(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize) {
    for x in 0..w {
        let mut sr = 0u32;
        let mut sg = 0u32;
        let mut sb = 0u32;
        let first_hi = r.min(h - 1);
        for i in 0..=first_hi {
            let p = src[i * w + x];
            sr += (p >> 16) & 0xFF;
            sg += (p >> 8) & 0xFF;
            sb += p & 0xFF;
        }
        let mut lo = 0i32;
        let mut hi = first_hi as i32;
        for y in 0..h {
            let cnt = (hi - lo + 1) as u32;
            dst[y * w + x] = 0xFF00_0000 | ((sr / cnt) << 16) | ((sg / cnt) << 8) | (sb / cnt);
            let add = y as i32 + 1 + r as i32;
            if add < h as i32 {
                let p = src[add as usize * w + x];
                sr += (p >> 16) & 0xFF;
                sg += (p >> 8) & 0xFF;
                sb += p & 0xFF;
                hi = add;
            }
            let rem = y as i32 - r as i32;
            if rem >= 0 {
                let p = src[rem as usize * w + x];
                sr -= (p >> 16) & 0xFF;
                sg -= (p >> 8) & 0xFF;
                sb -= p & 0xFF;
                lo = rem + 1;
            }
        }
    }
}

/// Separable box blur of a sub-region of the back buffer (the acrylic backdrop
/// under the panel/menu/dock). `tmp_a`/`tmp_b` are ping-pong scratch buffers
/// allocated once by the caller; a region larger than the scratch is skipped.
/// `iters` box passes ≈ a Gaussian. Integer-only, O(region) per pass.
pub(super) fn blur_region(buf: &mut [u32], bw: usize, bh: usize, rx: i32, ry: i32, rw: i32, rh: i32, radius: i32, iters: i32, tmp_a: &mut [u32], tmp_b: &mut [u32]) {
    if radius <= 0 || iters <= 0 {
        return;
    }
    let x0 = rx.max(0) as usize;
    let y0 = ry.max(0) as usize;
    let x1 = ((rx + rw).max(0) as usize).min(bw);
    let y1 = ((ry + rh).max(0) as usize).min(bh);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let w = x1 - x0;
    let h = y1 - y0;
    if w * h > tmp_a.len() || w * h > tmp_b.len() {
        return;
    }
    for yy in 0..h {
        let dr = yy * w;
        let sr = (y0 + yy) * bw + x0;
        tmp_a[dr..dr + w].copy_from_slice(&buf[sr..sr + w]);
    }
    let r = radius as usize;
    for _ in 0..iters {
        box_h(tmp_a, tmp_b, w, h, r);
        box_v(tmp_b, tmp_a, w, h, r);
    }
    for yy in 0..h {
        let sr = yy * w;
        let dr = (y0 + yy) * bw + x0;
        buf[dr..dr + w].copy_from_slice(&tmp_a[sr..sr + w]);
    }
}

/// Clip the rect `(x, y, w, h)` to the `bw*bh` framebuffer, returning
/// `(x0, y0, cw, ch)` in buffer pixels (`cw == 0` or `ch == 0` = fully off-screen).
pub(super) fn clip_to_screen(x: i32, y: i32, w: i32, h: i32, bw: usize, bh: usize) -> (usize, usize, usize, usize) {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(bw as i32);
    let y1 = (y + h).min(bh as i32);
    if x1 <= x0 || y1 <= y0 {
        return (0, 0, 0, 0);
    }
    (x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize)
}

/// Blit a client's surface into the back buffer at (x, y), clipped. `argb`
/// selects the compositing rule per the format the client imported its buffer
/// with (captured in `ClientState.format`, mirrored into `Win.format`):
///   * XRGB8888 (`argb == false`): the surface is opaque — a straight copy, the
///     fast path; the top 8 bits are "don't care" so we never read them.
///   * ARGB8888 (`argb == true`):  the surface carries a straight-alpha channel
///     (e.g. a terminal with a translucent background) — src-over blend each
///     pixel over the already-composited desktop so the wallpaper/windows behind
///     show through, while opaque pixels (text, icons: alpha 255) overwrite.
pub(super) fn blit_surface(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    x: i32,
    y: i32,
    src: &[u32],
    sw: usize,
    sh: usize,
    argb: bool,
) {
    let mut sy = 0usize;
    while sy < sh {
        let dy = y + sy as i32;
        if dy >= 0 && (dy as usize) < bh {
            let drow = dy as usize * bw;
            let srow = sy * sw;
            let mut sx = 0usize;
            while sx < sw {
                let dx = x + sx as i32;
                if dx >= 0 && (dx as usize) < bw {
                    if argb {
                        // src-over: opaque (a==255) pixels overwrite, translucent
                        // ones blend, fully transparent (a==0) leave the desktop.
                        blend_pixel(buf, bw, bh, dx, dy, src[srow + sx]);
                    } else {
                        buf[drow + dx as usize] = 0xff00_0000 | src[srow + sx];
                    }
                }
                sx += 1;
            }
        }
        sy += 1;
    }
}
/// A classic top-left arrow cursor. 'X' = black outline, '.' = white fill,
/// ' ' = transparent. Hotspot is the top-left corner (0,0).
pub(super) const CURSOR: [&str; 17] = [
    "X                ",
    "XX               ",
    "X.X              ",
    "X..X             ",
    "X...X            ",
    "X....X           ",
    "X.....X          ",
    "X......X         ",
    "X.......X        ",
    "X........X       ",
    "X.....XXXXX      ",
    "X..X..X          ",
    "X.X X..X         ",
    "XX  X..X         ",
    "X    X..X        ",
    "     X..X        ",
    "      XX         ",
];

/// Draw the arrow cursor into the back buffer with its hotspot at (px, py).
pub(super) fn draw_cursor(buf: &mut [u32], bw: usize, bh: usize, px: i32, py: i32) {
    for (ry, row) in CURSOR.iter().enumerate() {
        let dy = py + ry as i32;
        if dy < 0 || dy as usize >= bh {
            continue;
        }
        let drow = dy as usize * bw;
        for (rx, ch) in row.bytes().enumerate() {
            let color = match ch {
                b'X' => 0xFF000000,
                b'.' => 0xFFFFFFFF,
                _ => continue,
            };
            let dx = px + rx as i32;
            if dx >= 0 && (dx as usize) < bw {
                buf[drow + dx as usize] = color;
            }
        }
    }
}
// ===========================================================================
// DWM shell — top panel / taskbar (M4 slice 5)
// ===========================================================================
//
// A compositor-owned panel across the top of the screen: a launcher glyph on
// the left, one taskbar button per live window (click to raise + focus) in the
// middle, and an uptime clock on the right. The panel is drawn on top of every
// window each tick and its band is reserved — windows are kept below it and
// pointer input over the panel is consumed by the shell, never forwarded to a
// client.

// Panel/launcher/taskbar/menu geometry now lives in `settings::Layout` (loaded
// from the `[layout]` TOML table); `max_windows` there also caps launcher spawns
// so a stuck loop cannot fork the machine to death.

// The application registry (ids, names, exec paths, icons, labels) and the
// dock / launcher / autostart lists are no longer hardcoded here: they live in
// `settings::Applications`, built from the `[application.*]` / `[dock]` /
// `[launcher]` / `[startup]` TOML tables (baseline in `Applications::baseline`).
// A window's `app` field is an index into that live registry (0xFF = unknown).

// Shell-content geometry is laid out along the panel's MAIN axis (X for a
// top/bottom panel, Y for a left/right panel) so one set of helpers serves all
// four edges. `panel_slot` maps a 1-D span on the main axis to a screen rect
// spanning the full cross-axis thickness; `inset_cross` trims the cross axis.
// For the default top panel these reproduce the old fixed geometry byte-for-byte.

/// The panel strip itself: `ly.panel_h` thick, flush against `panel_edge`, owning
/// its corners (offset 0). For the default top panel this is `(0, 0, bw, panel_h)`.
pub(super) fn panel_strip(ly: &Layout, ins: &Insets, edge: Edge, bw: usize, bh: usize) -> (i32, i32, i32, i32) {
    strip_rect(edge, ly.panel_h, 0, ins, bw, bh)
}

/// Top-left origin `(x, y)` of the launcher dropdown, flying out PERPENDICULAR to
/// the panel edge: below a top panel, above a bottom panel, right of a left panel,
/// left of a right panel. `item_count` sizes the "above" case. The menu itself is
/// always a vertical list of `menu_item_h` rows.
pub(super) fn menu_origin(ly: &Layout, edge: Edge, panel: (i32, i32, i32, i32), item_count: i32) -> (i32, i32) {
    let (px, py, pw, ph) = panel;
    let full_h = item_count * ly.menu_item_h;
    match edge {
        Edge::Top => (px, py + ph),
        Edge::Bottom => (px, py - full_h),
        Edge::Left => (px + pw, py),
        Edge::Right => (px - ly.menu_w, py),
    }
}

/// Rect of the i-th launcher-menu entry, stacked vertically from `origin`.
pub(super) fn menu_item_rect(ly: &Layout, origin: (i32, i32), i: i32) -> (i32, i32, i32, i32) {
    (origin.0, origin.1 + i * ly.menu_item_h, ly.menu_w, ly.menu_item_h)
}

/// Quick-settings flyout card: `rows` toggle rows flying out PERPENDICULAR to the
/// panel edge from the applet cell (same convention as the launcher `menu_origin`).
pub(super) fn qs_flyout_rect(ly: &Layout, edge: Edge, applet: (i32, i32, i32, i32), rows: i32) -> (i32, i32, i32, i32) {
    let (ax, ay, aw, _ah) = applet;
    let w = 150i32;
    let h = rows * ly.menu_item_h + 8;
    match edge {
        Edge::Top => (ax, ay + applet.3, w, h),
        Edge::Bottom => (ax, ay - h, w, h),
        Edge::Left => (ax + aw, ay, w, h),
        Edge::Right => (ax - w, ay, w, h),
    }
}

/// Rect of the i-th quick-settings flyout row (a toggle line).
pub(super) fn qs_row_rect(ly: &Layout, flyout: (i32, i32, i32, i32), i: i32) -> (i32, i32, i32, i32) {
    (flyout.0 + 4, flyout.1 + 4 + i * ly.menu_item_h, flyout.2 - 8, ly.menu_item_h)
}

/// True when the point is inside `rect`.
pub(super) fn in_rect(rect: (i32, i32, i32, i32), x: i32, y: i32) -> bool {
    x >= rect.0 && x < rect.0 + rect.2 && y >= rect.1 && y < rect.1 + rect.3
}

/// Legacy numeric call sites use the same configured TrueType font as shell text.
pub(super) fn draw_shell_text(buf:&mut[u32],bw:usize,bh:usize,font:Option<&Font>,x:i32,y:i32,px:i32,color:u32,s:&[u8])->i32 {
    match (font,core::str::from_utf8(s)) {
        (Some(font),Ok(text))=>draw_text_ttf_alpha(buf,bw,bh,font,x,y+px,px as f32,text,color,color>>24),
        _=>x,
    }
}

/// Write a zero-padded two-digit value into `out[off..off+2]`.


// --- Crisp TTF text for the shell (panel titles, window titles, tray) --------
// All user-visible shell text uses the configured TrueType font.

/// Load the configured TTF from the VFS, falling back to the shipped default
/// font in the initrd (/assets/fonts) — assets live on the filesystem, not baked
/// into the ELF. The live font path is a config knob (`[desktop] font`). `None`
/// only if even the default font fails to parse; the caller reports the error.
pub(super) fn load_font_path(path:&str)->Option<Font>{
    if let Some(bytes) = libdunit::read_binary(path, 4 * 1024 * 1024) {
        if let Ok(f) = Font::parse(bytes) {
            return Some(f);
        }
    }
    libdunit::read_binary("/assets/fonts/DejaVuSans.ttf", 4 * 1024 * 1024).and_then(|b| Font::parse(b).ok())
}

pub(super) fn round_i32(v: f32) -> i32 {
    if v <= 0.0 {
        0
    } else {
        (v + 0.5) as i32
    }
}

/// Total advance width of `text` at `px`, rounded to whole pixels.
pub(super) fn text_width_ttf(font: &Font, text: &str, px: f32) -> i32 {
    round_i32(font.layout_line(text, px).1)
}

/// Draw one line of TTF `text` into the back buffer with its left edge at `x`
/// and baseline at `baseline`, blending each glyph's 8-bit coverage as alpha
/// over packed ARGB `color`. Returns the x just past the last glyph.
pub(super) fn draw_text_ttf(
    back: &mut [u32],
    bw: usize,
    bh: usize,
    font: &Font,
    x: i32,
    baseline: i32,
    px: f32,
    text: &str,
    color: u32,
) -> i32 {

    draw_text_ttf_alpha(back,bw,bh,font,x,baseline,px,text,color,255)
}
pub(super) fn draw_text_ttf_alpha(
    back: &mut [u32],
    bw: usize,
    bh: usize,
    font: &Font,
    x: i32,
    baseline: i32,
    px: f32,
    text: &str,
    color: u32,
    opacity:u32,
) -> i32 {
    let rgb = color & 0x00FF_FFFF;
    let (glyphs, adv) = font.layout_line(text, px);
    for g in glyphs {
        if let Some(bmp) = font.rasterize(g.glyph, px) {
            let ox = x + round_i32(g.x) + bmp.left;
            let oy = baseline - bmp.top;
            for row in 0..bmp.height {
                for col in 0..bmp.width {
                    let cov = bmp.coverage[row * bmp.width + col] as u32*opacity.min(255)/255;
                    if cov == 0 {
                        continue;
                    }
                    blend_pixel(back, bw, bh, ox + col as i32, oy + row as i32, (cov << 24) | rgb);
                }
            }
        }
    }
    x + round_i32(adv)
}
