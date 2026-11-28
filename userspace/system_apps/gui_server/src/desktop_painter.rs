//! Full desktop painting and scanout; input and protocol service live elsewhere.
use super::*;

pub(super) struct Context<'a> {
    pub back: &'a mut Vec<u32>,
    pub bw: usize,
    pub bh: usize,
    pub fb: &'a libdunit::FbInfo,
    pub wallpaper: Option<&'a [u32]>,
    pub theme: &'a Theme,
    pub ly: &'a Layout,
    pub skin: &'a dunit_style::shell::Visuals,
    pub fx: &'a Effects,
    pub font: Option<&'a Font>,
    pub apps: &'a Applications,
    pub app_icons: &'a [Option<dunit_image::Image>],
    pub logo: Option<&'a dunit_image::Image>,
    pub wins: &'a [Win],
    pub z: &'a [usize],
    pub focused: Option<usize>,
    pub current_ws: usize,
    pub ins: &'a Insets,
    pub effects: &'a mut window_effects::Effects,
    pub shell: &'a mut shell_runtime::Shell,
    pub blur_a: &'a mut Vec<u32>,
    pub blur_b: &'a mut Vec<u32>,
    pub blur_cache: &'a mut window_painter::BlurCache,
    pub scene_epoch: u64,
    pub win_backing: &'a mut Vec<u32>,
    pub argb_blit_logged: &'a mut bool,
    pub animation_now: u64,
    pub clock_valid: bool,
    pub anim_duration_ms: u32,
    pub lua_ease_out: bool,
    pub ws_switch_tick: u64,
    pub desktop_tracks: &'a shell_motion::Tracks,
    pub workspace_layer: &'a mut desktop_motion::Layer,
    pub widgets_enabled: bool,
    pub desk_widgets: &'a [DeskWidget],
    pub desktop_widget_stats: libdunit::SystemStats,
    pub desktop_widget_stats_valid: bool,
    pub mx: i32,
    pub my: i32,
    pub wx: i32,
    pub wy: i32,
    pub cursor_visible: bool,
    pub menu_open: bool,
    pub menu_was_open: &'a mut bool,
    pub menu_anim_tick: &'a mut u64,
    pub menu_closed_at: &'a mut u64,
    pub menu_sel: usize,
    pub menu_entries: &'a [usize],
    pub menu_ly: Layout,
    pub menu_org: (i32, i32),
    pub menu_view: shell_motion::Visual,
    pub switcher: &'a Switcher,
    pub quicksettings_enabled: bool,
    pub qs_open: bool,
    pub qs_edge: Edge,
    pub qs_applet: (i32, i32, i32, i32),
    pub notifs: &'a mut Vec<Toast>,
    pub notifications: &'a settings::Notifications,
    pub notify_st_done: bool,
    pub notify_expired_logged: &'a mut bool,
    pub transfers: &'a transfer_runtime::Runtime,
    pub performance: &'a mut performance::Profiler,
    pub desktop_ready_announced: &'a mut bool,
    pub cursor_rect: &'a mut Option<(i32, i32, usize, usize)>,
    pub cursor_under: &'a mut Vec<u32>,
}

#[inline(always)]
fn fnv_mix(mut h: u64, v: u64) -> u64 {
    h ^= v;
    h.wrapping_mul(0x1000_0000_01B3)
}

pub(super) fn prune_notifs(
    notifs: &mut Vec<Toast>,
    animation_now: u64,
    clock_valid: bool,
    notify_st_done: bool,
    notify_expired_logged: &mut bool,
) {
    notifs.retain(|t| clock_valid && animation_now < t.expire);
    if notify_st_done && !*notify_expired_logged && notifs.is_empty() {
        libdunit::println("gui_server: notify expired (self-test)");
        *notify_expired_logged = true;
    }
}

fn notif_card_view(
    slot: usize,
    t: &Toast,
    bw: usize,
    bh: usize,
    notifications: &settings::Notifications,
    ly: &Layout,
    skin: &dunit_style::shell::Visuals,
    font: Option<&Font>,
    desktop_tracks: &shell_motion::Tracks,
    animation_now: u64,
    duration_ms: u32,
    lua_ease_out: bool,
) -> shell_motion::Visual {
    let margin = skin.notification_margin;
    let cardh = skin.notification_height;
    let gap = skin.notification_gap;
    let corner = notifications.corner;
    let top = corner == 0 || corner == 1;
    let leftc = corner == 0 || corner == 2;
    let px = ly.title_font_px as f32;
    let tw = font
        .map(|f| text_width_ttf(f, &t.text, px))
        .unwrap_or(t.text.len() as i32 * ly.title_font_px / 2);
    let max_width = (bw as i32 - 2 * margin).max(1);
    let cardw = (tw + 2 * skin.notification_padding)
        .clamp(skin.notification_min_width.min(max_width), max_width);
    let progress = animation::progress(animation_now, t.born, duration_ms, lua_ease_out);
    let slide =
        skin.notification_slide - (progress * skin.notification_slide as u32 / 255) as i32;
    let base_x = if leftc { margin } else { bw as i32 - margin - cardw };
    let cx = if leftc { base_x - slide } else { base_x + slide };
    let cy = if top {
        margin + slot as i32 * (cardh + gap)
    } else {
        bh as i32 - margin - cardh - slot as i32 * (cardh + gap)
    };
    desktop_tracks.sample(desktop_motion::NOTIFICATIONS, (cx, cy, cardw, cardh), animation_now)
}

pub(super) fn notif_blockers_and_sig(
    notifs: &[Toast],
    bw: usize,
    bh: usize,
    notifications: &settings::Notifications,
    ly: &Layout,
    skin: &dunit_style::shell::Visuals,
    font: Option<&Font>,
    desktop_tracks: &shell_motion::Tracks,
    animation_now: u64,
    duration_ms: u32,
    lua_ease_out: bool,
) -> (Vec<crate::damage_plan::Rect>, u64, bool) {
    let mut sig = 0xCBF2_9CE4_8422_2325u64;
    sig = fnv_mix(sig, u64::from(notifications.enabled));
    sig = fnv_mix(sig, notifs.len() as u64);
    let mut blockers = Vec::new();
    let mut animating = false;
    if notifications.enabled && !notifs.is_empty() {
        for (slot, t) in notifs.iter().enumerate() {
            if animation::progress(animation_now, t.born, duration_ms, lua_ease_out) < 255 {
                animating = true;
            }
            sig = fnv_mix(sig, t.born);
            sig = fnv_mix(sig, t.expire);
            for &b in t.text.as_bytes() {
                sig = fnv_mix(sig, b as u64);
            }
            let view = notif_card_view(
                slot,
                t,
                bw,
                bh,
                notifications,
                ly,
                skin,
                font,
                desktop_tracks,
                animation_now,
                duration_ms,
                lua_ease_out,
            );
            let (cx, cy, cardw, cardh) = view.rect;
            let sh = if view.shadow > 0 { view.shadow } else { skin.notification_shadow }.max(0);
            let off = sh / 2 + skin.shadow_offset.max(0);
            blockers.push(crate::damage_plan::Rect {
                x: cx - sh,
                y: cy - sh,
                w: (cardw + 2 * sh).max(0) as u32,
                h: (cardh + 2 * sh + off).max(0) as u32,
            });
        }
    }
    (blockers, sig, animating)
}

pub(super) fn paint(c: Context<'_>) {
    let Context {
        mut back,
        bw,
        bh,
        fb,
        wallpaper,
        theme,
        ly,
        skin,
        fx,
        font,
        apps,
        app_icons,
        logo,
        wins,
        z,
        focused,
        current_ws,
        ins,
        effects,
        shell,
        mut blur_a,
        mut blur_b,
        blur_cache,
        scene_epoch,
        mut win_backing,
        argb_blit_logged,
        animation_now,
        clock_valid,
        anim_duration_ms,
        lua_ease_out,
        ws_switch_tick,
        desktop_tracks,
        workspace_layer,
        widgets_enabled,
        desk_widgets,
        desktop_widget_stats,
        desktop_widget_stats_valid,
        mx,
        my,
        wx,
        wy,
        cursor_visible,
        menu_open,
        menu_was_open,
        menu_anim_tick,
        menu_closed_at,
        menu_sel,
        menu_entries,
        menu_ly,
        menu_org,
        menu_view,
        switcher,
        quicksettings_enabled,
        qs_open,
        qs_edge,
        qs_applet,
        notifs,
        notifications,
        notify_st_done,
        notify_expired_logged,
        transfers,
        performance,
        desktop_ready_announced,
        cursor_rect,
        cursor_under,
    } = c;
    let duration_ms = if clock_valid { anim_duration_ms } else { 0 };
    let workspace_view = desktop_tracks.sample(
        desktop_motion::WORKSPACE,
        (0, 0, bw as i32, bh as i32),
        animation_now,
    );
    let workspace_transform =
        desktop_motion::changed(&workspace_view, bw, bh) && workspace_layer.begin(&back);
    let ws_fade = animation::progress(animation_now, ws_switch_tick, duration_ms, lua_ease_out);
    let ws_opacity_track =
        desktop_tracks.has(desktop_motion::WORKSPACE, lua_policy::Property::Opacity);
    let win_alpha = |w: &Win| -> i32 {
        (animation::progress(animation_now, w.born, duration_ms, lua_ease_out).min(
            if ws_opacity_track {
                255
            } else {
                ws_fade
            },
        ) * effects.alpha(w.pid, animation_now)
            / 255
            * w.opacity as u32
            / 255) as i32
    };
    let mut opaque_cover: Option<(usize, i32, i32, i32, i32)> = None;
    if !workspace_transform {
        for (pos, &wi) in z.iter().enumerate().rev() {
            let w = &wins[wi];
            if !w.alive
                || w.ws != current_ws
                || w.minimized
                || effects.covers(w.pid, animation_now)
                || w.format != FORMAT_XRGB8888
                || w.buf_ptr.is_null()
            {
                continue;
            }
            let want = (w.surface_w as usize).saturating_mul(w.surface_h as usize);
            if want == 0 || want * 4 > w.buf_size || win_alpha(w) != 255 {
                continue;
            }
            let (visual, _, _) = effects.visual(w, animation_now, &theme, &fx);
            let (ox, oy, ow, oh) = clip_to_screen(visual.cx, visual.cy, visual.sw, visual.sh, bw, bh);
            if ow > 0 && oh > 0 {
                opaque_cover = Some((pos, ox as i32, oy as i32, ow as i32, oh as i32));
                break;
            }
        }
    }

    let mut blur_target: Option<(usize, (usize, usize, usize, usize), u64, bool)> = None;
    if !workspace_transform
        && !effects.animating(animation_now)
        && !desktop_tracks.animating(animation_now)
        && ws_fade == 255
    {
        for (pos, &wi) in z.iter().enumerate().rev() {
            let w = &wins[wi];
            if !w.alive
                || w.ws != current_ws
                || w.minimized
                || effects.covers(w.pid, animation_now)
                || w.format != FORMAT_ARGB8888
                || w.buf_ptr.is_null()
            {
                continue;
            }
            let want = (w.surface_w as usize).saturating_mul(w.surface_h as usize);
            if want == 0 || want * 4 > w.buf_size || win_alpha(w) <= 0 {
                continue;
            }
            let (visual, _, visual_fx) = effects.visual(w, animation_now, &theme, &fx);
            if !visual_fx.blur || visual_fx.blur_radius <= 0 || visual_fx.blur_iters <= 0 {
                break;
            }
            let rect = clip_to_screen(visual.cx, visual.cy, visual.sw, visual.sh, bw, bh);
            if rect.2 == 0 || rect.3 == 0 {
                break;
            }
            if let Some((cover_pos, ocx, ocy, ocw, och)) = opaque_cover {
                if cover_pos > pos
                    && ocx <= rect.0 as i32
                    && ocy <= rect.1 as i32
                    && ocx + ocw >= (rect.0 + rect.2) as i32
                    && ocy + och >= (rect.1 + rect.3) as i32
                {
                    break;
                }
            }
            let mut sig = 0xCBF2_9CE4_8422_2325u64;
            for v in [
                scene_epoch,
                bw as u64,
                bh as u64,
                current_ws as u64,
                u64::from(wallpaper.is_some()),
                pos as u64,
                rect.0 as u64,
                rect.1 as u64,
                rect.2 as u64,
                rect.3 as u64,
                visual_fx.blur_radius as u64,
                visual_fx.blur_iters as u64,
                theme.desktop as u64,
                theme.title_focused as u64,
                theme.title_unfocused as u64,
                theme.border_focused as u64,
                theme.border_unfocused as u64,
                theme.close as u64,
                theme.minimize as u64,
                theme.maximize as u64,
                theme.title_text_focused as u64,
                theme.title_text_unfocused as u64,
                theme.menu as u64,
                theme.launcher as u64,
                theme.panel_text as u64,
                u64::from(fx.blur),
                fx.blur_radius as u64,
                fx.blur_iters as u64,
                u64::from(fx.gradient),
                fx.corner_radius as u64,
                fx.shadow as u64,
                fx.shadow_alpha as u64,
                fx.menu_alpha as u64,
                ly.border as u64,
                ly.title_h as u64,
                ly.title_font_px as u64,
                u64::from(widgets_enabled),
            ] {
                sig = fnv_mix(sig, v);
            }
            if widgets_enabled && !desk_widgets.is_empty() {
                let secs = if desktop_widget_stats_valid && desktop_widget_stats.uptime_hz > 0 {
                    desktop_widget_stats.uptime_ticks / desktop_widget_stats.uptime_hz
                } else {
                    0
                };
                sig = fnv_mix(sig, secs);
                sig = fnv_mix(sig, desktop_widget_stats.memory_used_percent() as u64);
                sig = fnv_mix(sig, desktop_widget_stats.live_processes());
            }
            for (bpos, &bwi) in z.iter().enumerate() {
                if bpos >= pos {
                    break;
                }
                let bw_win = &wins[bwi];
                if !bw_win.alive
                    || bw_win.ws != current_ws
                    || bw_win.minimized
                    || effects.covers(bw_win.pid, animation_now)
                {
                    continue;
                }
                let ba = win_alpha(bw_win);
                if ba <= 0 {
                    continue;
                }
                for v in [
                    bpos as u64,
                    bw_win.pid as u64,
                    bw_win.app as u64,
                    bw_win.buf_seq,
                    bw_win.cx as u64,
                    bw_win.cy as u64,
                    bw_win.sw as u64,
                    bw_win.sh as u64,
                    bw_win.surface_w as u64,
                    bw_win.surface_h as u64,
                    bw_win.format as u64,
                    u64::from(focused == Some(bwi)),
                    ba as u64,
                    bw_win.title_h as u64,
                    bw_win.border as u64,
                    u64::from(bw_win.traffic_lights),
                ] {
                    sig = fnv_mix(sig, v);
                }
            }
            let hit = blur_cache.matches(sig, rect);
            blur_target = Some((pos, rect, sig, hit));
            break;
        }
    }

    let blur_skip = match blur_target {
        Some((pos, (rx, ry, rw, rh), _, true)) => Some((pos, rx as i32, ry as i32, rw as i32, rh as i32)),
        _ => None,
    };
    let backdrop_cover = match (opaque_cover, blur_skip) {
        (Some(oc), Some(bs)) => {
            if oc.3 as i64 * oc.4 as i64 >= bs.3 as i64 * bs.4 as i64 {
                Some(oc)
            } else {
                Some(bs)
            }
        }
        (Some(oc), None) => Some(oc),
        (None, Some(bs)) => Some(bs),
        (None, None) => None,
    };

    // Desktop backdrop: paint only strips outside any full-opacity top cover
    // or cached blurred window backdrop.
    let strips: [(usize, usize, usize, usize); 4] = if let Some((_, cx, cy, cw, ch)) = backdrop_cover {
        let (cx, cy, cw, ch) = (cx as usize, cy as usize, cw as usize, ch as usize);
        [
            (0, 0, bw, cy),
            (0, cy + ch, bw, bh - (cy + ch)),
            (0, cy, cx, ch),
            (cx + cw, cy, bw - (cx + cw), ch),
        ]
    } else {
        [(0, 0, bw, bh), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)]
    };
    if let Some(wp) = &wallpaper {
        for &(sx, sy, sw, sh) in &strips {
            if sw == 0 || sh == 0 {
                continue;
            }
            if sw == bw && sx == 0 {
                let start = sy * bw;
                let end = (sy + sh) * bw;
                back[start..end].copy_from_slice(&wp[start..end]);
            } else {
                for y in sy..sy + sh {
                    let base = y * bw + sx;
                    back[base..base + sw].copy_from_slice(&wp[base..base + sw]);
                }
            }
        }
    } else if fx.gradient {
        let top = shade(theme.desktop, skin.desktop_gradient_top);
        let bot = shade(theme.desktop, skin.desktop_gradient_bottom);
        let span = (bh as u32 - 1).max(1);
        for &(sx, sy, sw, sh) in &strips {
            if sw == 0 || sh == 0 {
                continue;
            }
            for y in sy..sy + sh {
                let row = 0xFF00_0000 | lerp_color(top, bot, (y as u32 * 255) / span);
                let base = y * bw + sx;
                back[base..base + sw].fill(row);
            }
        }
    } else {
        for &(sx, sy, sw, sh) in &strips {
            if sw == 0 || sh == 0 {
                continue;
            }
            for y in sy..sy + sh {
                let base = y * bw + sx;
                back[base..base + sw].fill(theme.desktop);
            }
        }
    }

    // Paint configured desktop widgets behind windows.
    if widgets_enabled && !desk_widgets.is_empty() {
        if let Some(f) = font {
            let wst = desktop_widget_stats;
            let whave = desktop_widget_stats_valid;
            for corner in 0..4i32 {
                let clock_w = desk_widgets
                    .iter()
                    .find(|w| w.corner == corner && w.kind == WidgetKind::Clock);
                let mon_w = desk_widgets
                    .iter()
                    .find(|w| w.corner == corner && w.kind == WidgetKind::Monitor);
                if clock_w.is_none() && mon_w.is_none() {
                    continue;
                }
                let card_w = skin.widget_width;
                let pad = skin.widget_padding;
                let clock_px = clock_w
                    .map(|w| w.clock.font_px as f32)
                    .unwrap_or(ly.title_font_px as f32);
                let clock_h = clock_w
                    .map(|w| {
                        if w.clock.style == settings::ClockStyle::Analog {
                            w.clock.size as i32
                        } else {
                            clock_px as i32 + skin.widget_clock_gap
                        }
                    })
                    .unwrap_or(0);
                let mon_h = if mon_w.is_some() {
                    skin.widget_monitor_height
                } else {
                    0
                };
                let card_h = pad * 2 + clock_h + mon_h;
                let margin = skin.widget_margin;
                let cx = if corner == 1 || corner == 3 {
                    bw as i32 - ins.right - card_w - margin
                } else {
                    ins.left + margin
                };
                let cy = if corner == 2 || corner == 3 {
                    bh as i32 - ins.bottom - card_h - margin
                } else {
                    ins.top + margin
                };
                if let Some((_, ocx, ocy, ocw, och)) = backdrop_cover {
                    if cx >= ocx && cy >= ocy && cx + card_w <= ocx + ocw && cy + card_h <= ocy + och {
                        continue;
                    }
                }
                if fx.blur {
                    blur_region(
                        &mut back,
                        bw,
                        bh,
                        cx,
                        cy,
                        card_w,
                        card_h,
                        fx.blur_radius,
                        fx.blur_iters,
                        &mut blur_a,
                        &mut blur_b,
                    );
                }
                fill_rrect_grad(
                    &mut back,
                    bw,
                    bh,
                    cx,
                    cy,
                    card_w,
                    card_h,
                    fx.corner_radius,
                    RR_ALL,
                    shade(theme.menu, skin.widget_gradient_top),
                    shade(theme.menu, skin.widget_gradient_bottom),
                    fx.menu_alpha,
                );
                let secs = if whave && wst.uptime_hz > 0 {
                    wst.uptime_ticks / wst.uptime_hz
                } else {
                    0
                };
                let mut ty = cy + pad;
                if let Some(clock) = clock_w {
                    if clock.clock.style == settings::ClockStyle::Analog {
                        shell_runtime::analog(
                            &skin,
                            &mut back,
                            bw,
                            bh,
                            (cx + pad, ty, card_w - 2 * pad, clock_h),
                            secs,
                            theme.panel_text,
                            theme.launcher,
                        );
                    } else {
                        let clk = alloc::format!(
                            "{:02}:{:02}:{:02}",
                            (secs / 3600) % 100,
                            (secs / 60) % 60,
                            secs % 60
                        );
                        let tw = text_width_ttf(f, &clk, clock_px);
                        let baseline = ty + clock_px as i32 - skin.widget_clock_baseline_inset;
                        draw_text_ttf(
                            &mut back,
                            bw,
                            bh,
                            f,
                            cx + (card_w - tw) / 2,
                            baseline,
                            clock_px,
                            &clk,
                            theme.panel_text & 0x00FF_FFFF,
                        );
                    }
                    ty += clock_h;
                }
                if let Some(mw) = mon_w {
                    let px = ly.title_font_px as f32;
                    let ram_pct = if whave && wst.pmm_total_bytes > 0 {
                        wst.memory_used_percent()
                    } else {
                        0
                    };
                    let line = alloc::format!("RAM {}%    {} proc", ram_pct, wst.live_processes());
                    draw_text_ttf(
                        &mut back,
                        bw,
                        bh,
                        f,
                        cx + pad,
                        ty + px as i32,
                        px,
                        &line,
                        theme.panel_text & 0x00FF_FFFF,
                    );
                    // RAM usage bar under the text row.
                    let bar_y = ty + px as i32 + skin.widget_bar_gap;
                    let bar_w = card_w - 2 * pad;
                    fill_rrect(
                        &mut back,
                        bw,
                        bh,
                        cx + pad,
                        bar_y,
                        bar_w,
                        skin.widget_bar_height,
                        skin.widget_bar_radius,
                        RR_ALL,
                        ((fx.menu_alpha.min(255) as u32) << 24)
                            | (shade(theme.menu, skin.widget_bar_shade) & 0x00FF_FFFF),
                    );
                    let fill_w = (bar_w * ram_pct.min(100) as i32) / 100;
                    if fill_w > 0 {
                        // Per-widget accent (widgets/monitor.toml `accent`) overrides
                        // the theme launcher color for the fill; 0 = inherit theme.
                        let bar_col = if mw.accent != 0 {
                            mw.accent
                        } else {
                            theme.launcher
                        };
                        fill_rrect(
                            &mut back,
                            bw,
                            bh,
                            cx + pad,
                            bar_y,
                            fill_w,
                            skin.widget_bar_height,
                            skin.widget_bar_radius,
                            RR_ALL,
                            0xFF00_0000 | (bar_col & 0x00FF_FFFF),
                        );
                    }
                }
            }
        }
    }

    // Paint windows bottom to top with window and workspace opacity.
    for (pos, &wi) in z.iter().enumerate() {
        let w = &wins[wi];
        if !w.alive || w.ws != current_ws || w.minimized || effects.covers(w.pid, animation_now) {
            continue;
        }
        let is_focused = focused == Some(wi);
        let a = win_alpha(w);
        if a <= 0 {
            continue;
        }
        let (visual, visual_theme, visual_fx) = effects.visual(w, animation_now, &theme, &fx);
        if let Some((cover_pos, ocx, ocy, ocw, och)) = opaque_cover {
            if pos < cover_pos {
                let (ox, oy, ow, oh) = visual.outer();
                let sh = visual_fx.shadow.max(0);
                if ox - sh >= ocx
                    && oy - sh >= ocy
                    && ox + ow + sh <= ocx + ocw
                    && oy + oh + sh <= ocy + och
                {
                    continue;
                }
            }
        }
        if let Some((cover_pos, ocx, ocy, ocw, och)) = blur_skip {
            if pos < cover_pos {
                let (ox, oy, ow, oh) = visual.outer();
                let sh = visual_fx.shadow.max(0);
                if ox - sh >= ocx
                    && oy - sh >= ocy
                    && ox + ow + sh <= ocx + ocw
                    && oy + oh + sh <= ocy + och
                {
                    continue;
                }
            }
        }
        let target_cache = match blur_target {
            Some((tpos, _, sig, _)) if pos == tpos => Some((&mut *blur_cache, sig)),
            _ => None,
        };
        window_painter::paint_with_blur_cache(
            &mut back,
            bw,
            bh,
            &visual,
            is_focused,
            a,
            &visual_theme,
            &ly,
            &skin,
            &visual_fx,
            font,
            &apps,
            wx,
            wy,
            &mut win_backing,
            argb_blit_logged,
            &mut blur_a,
            &mut blur_b,
            target_cache,
        );
    }

    effects.prune(&wins, animation_now);
    effects.paint(&mut back, bw, bh, animation_now, current_ws);
    if workspace_transform {
        workspace_layer.finish(
            &mut back,
            bw,
            bh,
            &workspace_view,
            &skin,
            &mut blur_a,
            &mut blur_b,
        );
    }
    let frame_fx = Effects {
        anim: fx.anim && clock_valid,
        ..*fx
    };
    shell.paint(
        &mut back,
        bw,
        bh,
        font,
        &app_icons,
        logo,
        &apps,
        &wins,
        focused,
        current_ws,
        mx,
        my,
        animation_now,
        &theme,
        &ly,
        &frame_fx,
        &mut blur_a,
        &mut blur_b,
    );

    // Launcher dropdown: acrylic rounded menu with a drop-in reveal.
    if menu_open && !*menu_was_open {
        *menu_anim_tick = animation_now; // stamp the open so the menu drops in
    }
    if !menu_open && *menu_was_open {
        *menu_closed_at = animation_now;
    }
    *menu_was_open = menu_open;
    if menu_open
        || (*menu_closed_at > 0
            && animation_now.saturating_sub(*menu_closed_at) < duration_ms as u64)
        || desktop_tracks.active(desktop_motion::LAUNCHER, animation_now)
    {
        let menu_opacity = if desktop_tracks
            .has(desktop_motion::LAUNCHER, lua_policy::Property::Opacity)
            && (menu_open
                || desktop_tracks.since(
                    desktop_motion::LAUNCHER,
                    lua_policy::Property::Opacity,
                    *menu_closed_at,
                )) {
            menu_view.opacity
        } else if menu_open {
            255
        } else {
            255 - animation::progress(animation_now, *menu_closed_at, duration_ms, lua_ease_out)
        };
        let menu_blend = menu_opacity < 255 && workspace_layer.begin(&back);
        let mprog =
            animation::progress(animation_now, *menu_anim_tick, duration_ms, lua_ease_out) as i32; // 0..255
        let (mx0, my0, mw0, _) = menu_item_rect(&menu_ly, menu_org, 0);
        let full_h = menu_entries.len() as i32 * menu_ly.menu_item_h;
        let reveal_h = full_h * mprog / 255; // grow the card downward
        if reveal_h > 0 {
            if fx.blur || menu_view.blur > 0 {
                blur_region(
                    &mut back,
                    bw,
                    bh,
                    mx0,
                    my0,
                    mw0,
                    reveal_h,
                    if menu_view.blur > 0 {
                        menu_view.blur
                    } else {
                        fx.blur_radius
                    },
                    fx.blur_iters,
                    &mut blur_a,
                    &mut blur_b,
                );
            }
            if menu_view.shadow > 0 {
                draw_soft_shadow(
                    &skin,
                    &mut back,
                    bw,
                    bh,
                    mx0,
                    my0,
                    mw0,
                    reveal_h,
                    menu_view.radius.unwrap_or(fx.corner_radius),
                    menu_view.shadow,
                    fx.shadow_alpha,
                );
            }
            fill_rrect_grad(
                &mut back,
                bw,
                bh,
                mx0,
                my0,
                mw0,
                reveal_h,
                menu_view.radius.unwrap_or(fx.corner_radius),
                RR_ALL,
                shade(theme.menu, skin.overlay_gradient_top),
                theme.menu,
                fx.menu_alpha,
            );
        }
        for i in 0..menu_entries.len() {
            let ri = menu_entries[i];
            let (ix, iy, iw, ih) = menu_item_rect(&menu_ly, menu_org, i as i32);
            if iy + ih > my0 + reveal_h {
                break; // below the revealed edge — not shown yet
            }
            // Highlight the row under the pointer OR the keyboard selection,
            // so mouse and `cmd_mod`+Space→Up/Down navigation share one cue.
            let hover = (mx >= ix && mx < ix + iw && my >= iy && my < iy + ih) || i == menu_sel;
            if hover {
                fill_rrect(
                    &mut back,
                    bw,
                    bh,
                    ix + skin.hover_inset_x,
                    iy + skin.hover_inset_y,
                    iw - 2 * skin.hover_inset_x,
                    ih - 2 * skin.hover_inset_y,
                    skin.hover_radius,
                    RR_ALL,
                    ((skin.hover_alpha as u32) << 24) | (theme.menu_hover & 0x00FF_FFFF),
                );
            }
            // Registry icon at the row's left, then the app label after it.
            let mut text_x = ix + skin.menu_text_inset;
            if let Some(icon) = app_icons.get(ri).and_then(|s| s.as_ref()) {
                let isz =
                    (ih - skin.menu_icon_padding).clamp(skin.menu_icon_min, skin.menu_icon_max);
                let icy = iy + (ih - isz) / 2;
                blit_icon(
                    &mut back,
                    bw,
                    bh,
                    &icon.pixels,
                    icon.width,
                    icon.height,
                    ix + skin.menu_icon_inset,
                    icy,
                    isz,
                    isz,
                );
                text_x = ix + skin.menu_icon_inset + isz + skin.menu_icon_inset;
            }
            let label = apps.apps.get(ri).map(|a| a.label.as_str()).unwrap_or("");
            draw_shell_text(
                &mut back,
                bw,
                bh,
                font,
                text_x,
                iy + (ih - ly.panel_font_px) / 2,
                ly.panel_font_px,
                0xff000000 | (menu_view.color.unwrap_or(theme.panel_text) & 0x00ff_ffff),
                label.as_bytes(),
            );
        }
        if menu_blend {
            let mut opacity = desktop_tracks.sample(0, (0, 0, bw as i32, bh as i32), animation_now);
            opacity.opacity = menu_opacity;
            workspace_layer.finish(&mut back, bw, bh, &opacity, &skin, &mut blur_a, &mut blur_b);
        }
    }

    // Window switcher overlay.
    if switcher.active && !switcher.order.is_empty() {
        let n = switcher.order.len() as i32;
        let tile = skin.switcher_tile;
        let gap = skin.switcher_gap;
        let pad = skin.switcher_padding;
        let label_h = skin.switcher_label_height;
        let panw = n * tile + (n - 1) * gap + 2 * pad;
        let panh = tile + label_h + 2 * pad;
        let px0 = (bw as i32 - panw) / 2;
        let py0 = (bh as i32 - panh) / 2;
        draw_soft_shadow(
            &skin,
            &mut back,
            bw,
            bh,
            px0,
            py0,
            panw,
            panh,
            skin.switcher_radius,
            skin.switcher_shadow,
            skin.switcher_shadow_alpha,
        );
        fill_rrect(
            &mut back,
            bw,
            bh,
            px0,
            py0,
            panw,
            panh,
            skin.switcher_radius,
            RR_ALL,
            ((skin.switcher_alpha as u32) << 24) | (theme.menu & 0x00FF_FFFF),
        );
        for (k, &wi) in switcher.order.iter().enumerate() {
            let tx = px0 + pad + k as i32 * (tile + gap);
            let ty = py0 + pad;
            if k == switcher.idx {
                fill_rrect(
                    &mut back,
                    bw,
                    bh,
                    tx - skin.switcher_highlight_inset,
                    ty - skin.switcher_highlight_inset,
                    tile + 2 * skin.switcher_highlight_inset,
                    tile + 2 * skin.switcher_highlight_inset,
                    skin.switcher_highlight_radius,
                    RR_ALL,
                    0xFF00_0000 | (theme.menu_hover & 0x00FF_FFFF),
                );
            }
            let app = wins[wi].app as usize;
            let isz = tile - 2 * skin.switcher_icon_inset;
            if let Some(ic) = app_icons.get(app).and_then(|s| s.as_ref()) {
                blit_icon(
                    &mut back,
                    bw,
                    bh,
                    &ic.pixels,
                    ic.width,
                    ic.height,
                    tx + skin.switcher_icon_inset,
                    ty + skin.switcher_icon_inset,
                    isz,
                    isz,
                );
            } else {
                let lbl = apps.apps.get(app).map(|a| a.label.as_str()).unwrap_or("?");
                draw_shell_text(
                    &mut back,
                    bw,
                    bh,
                    font,
                    tx + skin.switcher_text_inset,
                    ty + (isz - ly.panel_font_px) / 2,
                    ly.panel_font_px,
                    theme.panel_text,
                    lbl.as_bytes(),
                );
            }
        }
        if let Some(&wi) = switcher.order.get(switcher.idx) {
            if let Some(f) = font {
                let title = win_title(&apps, wins[wi].app);
                let fpx = skin.switcher_font_size as f32;
                let tw = text_width_ttf(f, title, fpx);
                let sx = px0 + (panw - tw) / 2;
                let baseline = py0 + pad + tile + label_h - skin.switcher_baseline_inset;
                draw_text_ttf(
                    &mut back,
                    bw,
                    bh,
                    f,
                    sx,
                    baseline,
                    fpx,
                    title,
                    theme.panel_text & 0x00FF_FFFF,
                );
            }
        }
    }

    // Quick-settings flyout.
    if quicksettings_enabled && qs_open {
        let flyout = qs_flyout_rect(&ly, &skin, qs_edge, qs_applet, 2);
        let (fx0, fy0, fw0, fh0) = flyout;
        if fx.blur {
            blur_region(
                &mut back,
                bw,
                bh,
                fx0,
                fy0,
                fw0,
                fh0,
                fx.blur_radius,
                fx.blur_iters,
                &mut blur_a,
                &mut blur_b,
            );
        }
        draw_soft_shadow(
            &skin,
            &mut back,
            bw,
            bh,
            fx0,
            fy0,
            fw0,
            fh0,
            skin.qs_radius,
            skin.qs_shadow,
            skin.qs_shadow_alpha,
        );
        fill_rrect_grad(
            &mut back,
            bw,
            bh,
            fx0,
            fy0,
            fw0,
            fh0,
            fx.corner_radius,
            RR_ALL,
            shade(theme.menu, skin.overlay_gradient_top),
            theme.menu,
            fx.menu_alpha,
        );
        let rows = [("Blur", fx.blur), ("Widgets", widgets_enabled)];
        for (i, (lbl, on)) in rows.iter().enumerate() {
            let (rx, ry, rw, rh) = qs_row_rect(&ly, &skin, flyout, i as i32);
            let hover = mx >= rx && mx < rx + rw && my >= ry && my < ry + rh;
            if hover {
                fill_rrect(
                    &mut back,
                    bw,
                    bh,
                    rx + skin.qs_hover_inset,
                    ry + skin.qs_hover_inset,
                    rw - 2 * skin.qs_hover_inset,
                    rh - 2 * skin.qs_hover_inset,
                    skin.hover_radius,
                    RR_ALL,
                    ((skin.hover_alpha as u32) << 24) | (theme.menu_hover & 0x00FF_FFFF),
                );
            }
            if let Some(f) = font {
                let px = ly.panel_font_px as f32;
                let baseline = ry + rh / 2 + skin.qs_baseline_offset;
                draw_text_ttf(
                    &mut back,
                    bw,
                    bh,
                    f,
                    rx + skin.qs_label_inset,
                    baseline,
                    px,
                    lbl,
                    theme.panel_text & 0x00FF_FFFF,
                );
            }
            // On/off pill on the row's trailing edge, knob sliding to reflect state.
            let (pw, ph) = (skin.qs_toggle_width, skin.qs_toggle_height);
            let px0 = rx + rw - pw - skin.qs_toggle_margin;
            let py0 = ry + (rh - ph) / 2;
            let pill = if *on { theme.launcher } else { theme.taskbtn };
            fill_rrect(
                &mut back,
                bw,
                bh,
                px0,
                py0,
                pw,
                ph,
                ph / 2,
                RR_ALL,
                0xFF00_0000 | (pill & 0x00FF_FFFF),
            );
            let knob = ph - 2 * skin.qs_knob_inset;
            let kx = if *on {
                px0 + pw - knob - skin.qs_knob_inset
            } else {
                px0 + skin.qs_knob_inset
            };
            fill_rrect(
                &mut back,
                bw,
                bh,
                kx,
                py0 + skin.qs_knob_inset,
                knob,
                knob,
                knob / 2,
                RR_ALL,
                skin.toggle_knob,
            );
        }
    }

    // Expire toasts before painting the remaining queue.
    prune_notifs(notifs, animation_now, clock_valid, notify_st_done, notify_expired_logged);
    if notifications.enabled && !notifs.is_empty() {
        for (slot, t) in notifs.iter().enumerate() {
            let px = ly.title_font_px as f32;
            let view = notif_card_view(
                slot,
                t,
                bw,
                bh,
                notifications,
                ly,
                skin,
                font,
                desktop_tracks,
                animation_now,
                duration_ms,
                lua_ease_out,
            );
            let (cx, cy, cardw, cardh) = view.rect;
            let radius = view.radius.unwrap_or(skin.notification_radius);
            if view.blur > 0 {
                let need = (cardw.max(0) as usize)
                    .saturating_mul(cardh.max(0) as usize)
                    .min(bw * bh);
                if blur_a.len() < need {
                    blur_a.resize(need, 0);
                    blur_b.resize(need, 0);
                }
                blur_region(
                    &mut back,
                    bw,
                    bh,
                    cx,
                    cy,
                    cardw,
                    cardh,
                    view.blur,
                    1,
                    &mut blur_a,
                    &mut blur_b,
                );
            }
            draw_soft_shadow(
                &skin,
                &mut back,
                bw,
                bh,
                cx,
                cy,
                cardw,
                cardh,
                radius,
                if view.shadow > 0 {
                    view.shadow
                } else {
                    skin.notification_shadow
                },
                skin.notification_shadow_alpha * view.opacity as i32 / 255,
            );
            fill_rrect_grad(
                &mut back,
                bw,
                bh,
                cx,
                cy,
                cardw,
                cardh,
                radius,
                RR_ALL,
                shade(theme.menu, skin.overlay_gradient_top),
                theme.menu,
                fx.menu_alpha * view.opacity as i32 / 255,
            );
            // Accent bar on the leading edge marks it as a toast.
            fill_rrect(
                &mut back,
                bw,
                bh,
                cx,
                cy + skin.notification_accent_inset,
                skin.notification_accent_width,
                cardh - 2 * skin.notification_accent_inset,
                skin.notification_accent_radius,
                RR_ALL,
                (view.opacity << 24) | (theme.launcher & 0x00ffffff),
            );
            if let Some(f) = font {
                let baseline = cy + cardh / 2 + skin.notification_baseline_offset;
                draw_text_ttf_alpha(
                    &mut back,
                    bw,
                    bh,
                    f,
                    cx + skin.notification_text_inset,
                    baseline,
                    px,
                    &t.text,
                    view.color.unwrap_or(theme.panel_text),
                    view.opacity,
                );
            }
        }
    }

    if transfers.dragging() {
        if let Some((_, source, target, action)) = transfers.active() {
            if let Some(w) = target.and_then(|pid| wins.iter().find(|w| w.pid == pid && w.alive)) {
                let g = effects.input_geometry(w, animation_now);
                let (x, y, ww, hh) = g.outer();
                let b = ly.border.max(1);
                let color = theme.menu_hover;
                fill_rect(&mut back, bw, bh, x, y, ww, b, color);
                fill_rect(&mut back, bw, bh, x, y + hh - b, ww, b, color);
                fill_rect(&mut back, bw, bh, x, y, b, hh, color);
                fill_rect(&mut back, bw, bh, x + ww - b, y, b, hh, color);
            }
            if let Some(rect) = transfers
                .opener()
                .and_then(|index| shell.app_target(index, &apps))
            {
                let (x, y, w, h) = rect;
                let b = ly.border.max(1);
                let color = theme.menu_hover;
                fill_rect(&mut back, bw, bh, x, y, w, b, color);
                fill_rect(&mut back, bw, bh, x, y + h - b, w, b, color);
                fill_rect(&mut back, bw, bh, x, y, b, h, color);
                fill_rect(&mut back, bw, bh, x + w - b, y, b, h, color);
            }
            let side = ly.panel_font_px.max(1);
            let gap = ly.border.max(1);
            let x = mx + side + gap;
            let y = my + side + gap;
            if let Some(icon) = wins
                .iter()
                .find(|w| w.pid == source)
                .and_then(|w| app_icons.get(w.app as usize))
                .and_then(|i| i.as_ref())
            {
                blit_icon(
                    &mut back,
                    bw,
                    bh,
                    &icon.pixels,
                    icon.width,
                    icon.height,
                    x,
                    y,
                    side,
                    side,
                );
            }
            let label = match action {
                Some(dunit_transfer::Action::Copy) => "+",
                Some(dunit_transfer::Action::Move) => ">",
                Some(dunit_transfer::Action::Link) => "@",
                None => "x",
            };
            draw_shell_text(
                &mut back,
                bw,
                bh,
                font,
                x + side + gap,
                y,
                ly.panel_font_px,
                theme.panel_text,
                label.as_bytes(),
            );
        }
    }
    if cursor_visible {
        *cursor_rect = crate::cursor::bounds(mx, my, &shell.cursor, bw, bh);
        if let Some(rect) = *cursor_rect {
            crate::cursor::save_under(&back, bw, rect, cursor_under);
        } else {
            cursor_under.clear();
        }
        draw_cursor(
            &mut back,
            bw,
            bh,
            mx,
            my,
            &shell.cursor,
            skin.cursor_colors(),
        );
    } else {
        *cursor_rect = None;
        cursor_under.clear();
    }

    let bytes = unsafe { core::slice::from_raw_parts(back.as_ptr() as *const u8, back.len() * 4) };
    let perf_present = performance.stamp();
    let present_result = libdunit::fb_present(bytes, fb.width, fb.height, 0, 0);
    performance.present(perf_present, (fb.width as u64) * (fb.height as u64));
    if present_result != 0 {
        libdunit::println("gui_server: FAIL framebuffer present");
        libdunit::exit(1);
    }
    if !*desktop_ready_announced {
        libdunit::println("gui_server: desktop ready");
        *desktop_ready_announced = true;
    }
}
