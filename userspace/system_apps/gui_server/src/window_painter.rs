//! Shared decoration and client-surface painter used for live windows and isolated animation snapshots.
use super::*;
pub fn paint(
    back: &mut [u32],
    bw: usize,
    bh: usize,
    w: &Win,
    is_focused: bool,
    a: i32,
    theme: &Theme,
    ly: &Layout,
    fx: &Effects,
    font: Option<&Font>,
    apps: &Applications,
    mx: i32,
    my: i32,
    win_backing: &mut Vec<u32>,
    argb_blit_logged: &mut bool,
    blur_a: &mut Vec<u32>,
    blur_b: &mut Vec<u32>,
) {
    let (ox, oy, ow, oh) = w.outer();
    let bcol = if is_focused {
        theme.border_focused
    } else {
        theme.border_unfocused
    };
    let tcol = if is_focused {
        theme.title_focused
    } else {
        theme.title_unfocused
    };
    if fx.shadow > 0 {
        draw_soft_shadow(
            back,
            bw,
            bh,
            ox,
            oy,
            ow,
            oh,
            fx.corner_radius,
            fx.shadow,
            fx.shadow_alpha * a / 255,
        );
    }
    // Translucent (ARGB) window: snapshot the real backing (wallpaper +
    // lower-z windows already composited into `back`) under the content
    // rect NOW, before the opaque frame gradient below overwrites it. It's
    // restored right before the client blit so the client's semi-transparent
    // background blends over what's genuinely behind the window. Opaque
    // (XRGB) windows skip this — they overwrite the content rect wholesale.
    let translucent = w.format == FORMAT_ARGB8888 || a < 255;
    let mut backing_rect: Option<(usize, usize, usize, usize)> = None;
    if translucent {
        let (bx, by, cw, ch) = clip_to_screen(w.cx, w.cy, w.sw, w.sh, bw, bh);
        if cw > 0 && ch > 0 {
            win_backing.clear();
            for ry in 0..ch {
                let srow = (by + ry) * bw + bx;
                win_backing.extend_from_slice(&back[srow..srow + cw]);
            }
            backing_rect = Some((bx, by, cw, ch));
        }
    }
    // Rounded frame: top corners only, so the square-bottomed client
    // surface tucks flush against the bottom edge (no corner poke-through).
    fill_rrect_grad(
        back,
        bw,
        bh,
        ox,
        oy,
        ow,
        oh,
        fx.corner_radius,
        RR_TOP,
        shade(bcol, 18),
        shade(bcol, -12),
        a,
    );
    fill_rrect_grad(
        back,
        bw,
        bh,
        w.cx,
        w.cy - w.title_h,
        w.sw,
        w.title_h,
        fx.corner_radius,
        RR_TOP,
        shade(tcol, 16),
        shade(tcol, -10),
        a,
    );
    for (role, color) in [(0, theme.close), (1, theme.minimize), (2, theme.maximize)] {
        let (x, y, size, _) = w.control_rect(role);
        let radius = if w.traffic_lights {
            size / 2
        } else {
            fx.corner_radius.min(size / 2)
        };
        fill_rrect(
            back,
            bw,
            bh,
            x,
            y,
            size,
            size,
            radius,
            RR_ALL,
            ((a as u32) << 24) | (color & 0x00FF_FFFF),
        );
        // Traffic lights reveal action glyphs on hover, like desktop controls.
        if !w.traffic_lights || in_rect(w.control_rect(role), mx, my) {
            let glyph = if is_focused {
                theme.title_text_focused
            } else {
                theme.title_text_unfocused
            };
            let col = ((a as u32) << 24) | (glyph & 0x00FF_FFFF);
            let half = (size / 2).max(3);
            let gx = x + (size - half) / 2;
            let gy = y + size / 2;
            if role == 1 {
                fill_rect_alpha(back, bw, bh, gx, gy, half, 1, col);
            }
            if role == 2 {
                let top = y + (size - half) / 2;
                fill_rect_alpha(back, bw, bh, gx, top, half, 1, col);
                fill_rect_alpha(back, bw, bh, gx, top + half - 1, half, 1, col);
                fill_rect_alpha(back, bw, bh, gx, top, 1, half, col);
                fill_rect_alpha(back, bw, bh, gx + half - 1, top, 1, half, col);
            }
            if role == 0 && w.traffic_lights {
                for j in 0..half {
                    fill_rect_alpha(back, bw, bh, gx + j, y + (size - half) / 2 + j, 1, 1, col);
                    fill_rect_alpha(
                        back,
                        bw,
                        bh,
                        gx + half - 1 - j,
                        y + (size - half) / 2 + j,
                        1,
                        1,
                        col,
                    );
                }
            }
        }
    }
    // Window title text (TTF), left-aligned in the title bar, clear of the
    // close chip. Drawn only once the card is nearly solid so it appears
    // with the surface rather than through the grow-in blend.
    if a > 0 {
        if let Some(f) = font {
            let px = ly.title_font_px as f32;
            let baseline = w.cy - w.title_h / 2 + 5;
            let txtcol = if is_focused {
                theme.title_text_focused & 0x00FF_FFFF
            } else {
                theme.title_text_unfocused & 0x00FF_FFFF
            };
            let title = win_title(&apps, w.app);
            let tx = if w.traffic_lights {
                w.cx + (w.sw - text_width_ttf(f, title, px)) / 2
            } else {
                w.cx + 12
            };
            draw_text_ttf_alpha(back, bw, bh, f, tx, baseline, px, title, txtcol, a as u32);
        }
    }
    // Client surface. Held back until the frame is nearly solid so the
    // grow-in shows a clean card, not a half-blended image. XRGB surfaces
    // are an opaque copy (fast path); ARGB surfaces blend per-pixel over
    // the restored backing so a translucent client reveals the desktop.
    if a > 0 {
        let want = (w.surface_w as usize) * (w.surface_h as usize);
        if want > 0 && want * 4 <= w.buf_size {
            // Restore the pre-frame backing under the content rect so the
            // ARGB blend composites over wallpaper/lower windows, not frame.
            if let Some((bx, by, cw, ch)) = backing_rect {
                for ry in 0..ch {
                    let drow = (by + ry) * bw + bx;
                    let srow = ry * cw;
                    back[drow..drow + cw].copy_from_slice(&win_backing[srow..srow + cw]);
                }
            }
            if w.format == FORMAT_ARGB8888 && fx.blur && fx.blur_radius > 0 {
                let size = (w.sw.max(1) as usize)
                    .saturating_mul(w.sh.max(1) as usize)
                    .min(bw * bh);
                if blur_a.len() < size {
                    blur_a.resize(size, 0);
                    blur_b.resize(size, 0);
                }
                blur_region(
                    back,
                    bw,
                    bh,
                    w.cx,
                    w.cy,
                    w.sw,
                    w.sh,
                    fx.blur_radius,
                    fx.blur_iters,
                    blur_a,
                    blur_b,
                );
            }
            let src = unsafe { core::slice::from_raw_parts(w.buf_ptr as *const u32, want) };
            let argb = w.format == FORMAT_ARGB8888;
            if argb && !*argb_blit_logged {
                libdunit::println(&alloc::format!(
                    "gui_server: argb blit (client format={}, per-pixel src-over)",
                    w.format
                ));
                *argb_blit_logged = true;
            }
            if a == 255 && w.sw == w.surface_w && w.sh == w.surface_h {
                blit_surface(
                    back,
                    bw,
                    bh,
                    w.cx,
                    w.cy,
                    src,
                    w.sw as usize,
                    w.sh as usize,
                    argb,
                );
            } else {
                for y in (-w.cy).max(0)..w.sh.min(bh as i32 - w.cy) {
                    for x in (-w.cx).max(0)..w.sw.min(bw as i32 - w.cx) {
                        let pixel = src[(y as i64 * w.surface_h as i64 / w.sh as i64) as usize
                            * w.surface_w as usize
                            + (x as i64 * w.surface_w as i64 / w.sw as i64) as usize];
                        let alpha = if argb { pixel >> 24 } else { 255 };
                        blend_pixel(
                            back,
                            bw,
                            bh,
                            w.cx + x,
                            w.cy + y,
                            ((alpha * a as u32 / 255) << 24) | (pixel & 0x00ff_ffff),
                        );
                    }
                }
            }
        }
    }
}
