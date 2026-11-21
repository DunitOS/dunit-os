#![no_std]
#![no_main]

//! Userspace desktop compositor: gui-v1 transport, configured shell scenes,
//! isolated Lua policies and native animation over client shared buffers.

use core::panic::PanicInfo;

extern crate alloc;
use alloc::vec::Vec;

use gui_protocol_v1::server::Server;
use gui_protocol_v1::wire::Request;
use gui_protocol_v1::wire::{FORMAT_ARGB8888, FORMAT_XRGB8888};
use gui_protocol_v1::Opcode;

mod lua_policy;
mod animation;
mod protocol_runtime;
mod transfer_runtime;
mod input_lifecycle;
mod pointer_mode;
mod pointer_runtime;
mod frame_pacing;
mod frame_runtime;
mod performance;
mod client_buffers;
mod damage_plan;
mod damage_runtime;
use protocol_runtime::*;
mod cursor;
mod drawing;
use drawing::*;
mod composite;
use composite::blend;
mod shell_runtime;
mod shell_motion;
mod desktop_motion;
mod window_effects;
fn report_genie(pid:u32,pixels:usize){libdunit::println(&alloc::format!("gui_server: genie rendered pid={} pixels={}",pid,pixels));}
mod window_painter;
mod desktop_painter;
mod desktop_actions;

use dwm_settings as settings;
use settings::{Applications, Edge, Effects, Layout, Theme};
mod shell_theme;
mod shell_metrics;

use dunit_text::Font;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    // No unwinding in userspace: report and exit non-zero.
    libdunit::println("gui_server: PANIC");
    libdunit::exit(101)
}

mod bringup;
#[no_mangle]
pub extern "C" fn _start() -> ! { bringup::start() }

mod desktop_probes;
mod window_session;
use window_session::Session;
mod window_lifecycle;
mod window_model;
use window_model::*;
mod window_gesture;
use window_gesture::Capture;

mod desktop_navigation;
use desktop_navigation::*;

/// A toast with monotonic creation and expiry times.
struct Toast {
    text: alloc::string::String,
    born: u64,
    expire: u64,
}

/// Keep the notification queue bounded; lifetime comes from configuration.
fn notify_post(q: &mut Vec<Toast>, cfg: &settings::Notifications, now: u64, text: &str) {
    if !cfg.enabled {
        return;
    }
    const MAX_TOASTS: usize = 4;
    q.push(Toast { text: text.into(), born: now,
        expire: now.saturating_add(cfg.timeout_ms as u64) });
    if q.len() > MAX_TOASTS {
        let drop = q.len() - MAX_TOASTS;
        q.drain(0..drop);
    }
}



mod protocol_pump;
use protocol_pump::pump_clients;

mod desktop_resources;
use desktop_resources::*;

/// Input ownership survives framebuffer/session recreation.
#[derive(Default)]
struct PointerSession {
    input: u32,
    state: libdunit::MouseState,
    modifiers: u8,
    left: bool,
    right: bool,
    captured_pid: Option<u32>,
    keyboard: input_lifecycle::Lifecycle,
}

/// Return true when a display reload requires fresh framebuffer resources.
fn run_desktop_session(
    server: &mut Server,
    clients: &mut Vec<ClientState>,
    pending_notifies: &mut Vec<alloc::string::String>,
    first_baseline: Option<settings::Display>,
    mut pending_reload: bool,
    pointer: &mut PointerSession,
    transfers: &mut transfer_runtime::Runtime,
    isolation_proven: &mut bool,
) -> bool {
    // Apply the configured display mode before sizing framebuffer resources.
    let cfg = settings::load_config();
    transfers.configure(cfg.settings.transfers);
    // Invalid boot configuration falls back as a whole.
    let mut cfg = if cfg.valid {
        cfg
    } else {
        libdunit::println("gui_server: boot config invalid (schema/shape) — using baseline");
        settings::load_system_config()
    };

    // Compare pending bring-up reloads against the mode captured before clients ran.
    let applied_display = match first_baseline {
        Some(boot) => {
            apply_display_mode(&boot);
            boot
        }
        None => {
            apply_display_mode(&cfg.settings.display);
            cfg.settings.display
        }
    };

    let mut fb = libdunit::FbInfo {
        addr: 0,
        width: 0,
        height: 0,
        pitch: 0,
    };
    if !libdunit::get_framebuffer(&mut fb) || fb.width == 0 || fb.height == 0 {
        libdunit::println("gui_server: no framebuffer for desktop session");
        return false;
    }
    let bw = fb.width as usize;
    let bh = fb.height as usize;
    // Keep one live application registry for all desktop consumers.
    let mut shell = match shell_runtime::Shell::prepare(&cfg, bw, bh) {
        Ok(shell) => shell,
        Err(error)=>{
            libdunit::println(&alloc::format!("gui_server: boot shell rejected {}; recovering packaged desktop",error));
            cfg=settings::load_document_from("/assets/dwm/default.toml").and_then(|text|settings::validate_document(&text)).unwrap_or_else(|_|settings::Config::defaults());
            match shell_runtime::Shell::prepare(&cfg,bw,bh){Ok(shell)=>shell,Err(error)=>{libdunit::println(&alloc::format!("gui_server: packaged shell unavailable {}",error));return false;}}
        }
    };
    let mut theme=shell.palette;let mut ly=shell.layout;let mut skin=shell.visuals;
    let mut policy = cfg.policy;
    let mut apps: Applications = cfg.apps;
    if cfg.settings.from_file {
        libdunit::println(&alloc::format!("gui_server: settings loaded from {}",settings::active_config_path()));
    } else {
        libdunit::println("gui_server: settings default (no config file)");
    }
    libdunit::println(&alloc::format!(
        "gui_server: config apps={} dock={} launcher={} startup={} ws={}",
        apps.apps.len(),
        apps.dock.len(),
        apps.launcher.len(),
        apps.startup.len(),
        apps.workspaces
    ));

    // Report accepted schema versions and exercise their boundary.
    {
        let sv = cfg.settings.meta.schema_version;
        libdunit::println(&alloc::format!(
            "gui_server: config schema_version={} supported={}",
            sv,
            settings::version_supported(sv) as u32
        ));
        let guard_ok = settings::version_supported(settings::SCHEMA_VERSION)
            && !settings::version_supported(settings::SCHEMA_VERSION + 1)
            && !settings::version_supported(0);
        libdunit::println(&alloc::format!(
            "gui_server: schema guard {} (rejects future)",
            if guard_ok { "ok" } else { "FAIL" }
        ));
    }

    // Shell insets govern window placement and the maximize work area.
    let mut panel_edge: Edge = cfg.settings.panel.pos;
    let tray_edge: Edge = cfg.settings.tray.pos;
    let tray_size: i32 = cfg.settings.tray.size;
    libdunit::println(&alloc::format!(
        "gui_server: panel edge={} tray edge={} tray_size={}",
        panel_edge.as_str(),
        tray_edge.as_str(),
        tray_size
    ));
    let mut ins = shell.insets;

    // Load only enabled widget files; resolve the set again on reload.
    let mut desk_widgets = resolve_desk_widgets(cfg.settings.widgets.corner);
    {
        let has = |k: WidgetKind| desk_widgets.iter().any(|w| w.kind == k);
        libdunit::println(&alloc::format!(
            "gui_server: widgets enabled={} clock={} monitor={}",
            cfg.settings.widgets.enabled as u32,
            has(WidgetKind::Clock) as u32,
            has(WidgetKind::Monitor) as u32,
        ));
    }

    let mut back: Vec<u32> = Vec::new();
    back.resize(bw * bh, theme.desktop);

    // Desktop wallpaper: loaded + pre-scaled once (XRGB8888, framebuffer-sized).
    // Path comes from `[desktop] wallpaper`; absent/invalid → gradient fallback.
    let mut wallpaper: Option<Vec<u32>> = load_wallpaper(cfg.settings.desktop.wallpaper.as_str(), bw, bh, theme.desktop);
    if wallpaper.is_some() {
        libdunit::println("gui_server: wallpaper loaded (config [desktop] wallpaper)");
    } else {
        libdunit::println("gui_server: wallpaper absent — gradient backdrop");
    }

    // Each registry entry owns one icon slot, shared by dock and launcher.
    let mut app_icons: Vec<Option<dunit_image::Image>> = Vec::with_capacity(apps.apps.len());
    let mut icon_count = 0usize;
    {
        let mut icon_theme = load_icon_theme(cfg.settings.desktop.icon_theme.as_str());
        for entry in apps.apps.iter() {
            let icon = resolve_app_icon(entry.icon.as_str(), &mut icon_theme, (ly.dock_w + skin.icon_raster_padding).max(skin.icon_raster_min) as u32);
            if icon.is_some() {
                icon_count += 1;
            }
            app_icons.push(icon);
        }
    }
    if icon_count > 0 {
        libdunit::println(&alloc::format!("gui_server: icon assets loaded theme={} count={} raster={}px",
            cfg.settings.desktop.icon_theme.as_str(), icon_count, (ly.dock_w + skin.icon_raster_padding).max(skin.icon_raster_min)));
    } else {
        libdunit::println("gui_server: launcher icons absent — initials fallback");
    }

    // Missing logo assets use the shell accent mark.
    let mut logo = image_result(cfg.settings.desktop.logo.as_str(),
        dunit_image::load(cfg.settings.desktop.logo.as_str(), skin.logo_raster_size as u32, skin.logo_raster_size as u32));
    if logo.is_some() {
        libdunit::println("gui_server: wordmark logo loaded (circular)");
    } else {
        libdunit::println("gui_server: wordmark logo absent — accent mark fallback");
    }

    // Cache TrueType shell glyphs before exposing the committed scene.
    // Configured and packaged fonts are loaded through the VFS.
    let mut font = load_font_path(cfg.settings.desktop.font.as_str());
    if let Err(error)=shell.prepare_text(font.as_ref(),&apps,&ly){libdunit::println(&alloc::format!("gui_server: shell font cache rejected {}",error));return false;}
    if font.is_some() {
        libdunit::println("gui_server: shell font loaded (TTF)");
    } else {
        libdunit::println("gui_server: shell font unavailable — install a valid TrueType font");
    }

    shell.describe();

    // Session restore is gated by configuration and keyed by application ID.
    let mut session = Session::new(cfg.settings.session.restore);
    libdunit::println(&alloc::format!(
        "gui_server: session restore={} entries={}",
        cfg.settings.session.restore as u32,
        session.len()
    ));

    // Ping-pong scratch for the backdrop blur, sized for the largest blurred
    // region (panel / dock strip / menu) and allocated once.
    let panel_area = bw * ly.panel_h.max(0) as usize;
    let dock_area = ly.dock_w.max(0) as usize * bw.max(bh);
    let menu_area = ly.menu_w.max(0) as usize * (apps.launcher.len() * ly.menu_item_h.max(0) as usize);
    let scratch_len = panel_area.max(dock_area).max(menu_area).max(1);
    let mut blur_a: Vec<u32> = alloc::vec![0u32; scratch_len];
    let mut blur_b: Vec<u32> = alloc::vec![0u32; scratch_len];
    // Snapshot lower layers before painting an ARGB window frame.
    let mut win_backing: Vec<u32> = Vec::new();
    let mut argb_blit_logged = false;
    // Publish no client presentation until the desktop's persistent framebuffer,
    // wallpaper, assets, font cache and blur scratch are ready. A client may draw
    // again as soon as FRAME_DONE arrives; startup must not add these allocations
    // or their latency to that next frame. Mode changes prepare fresh resources
    // but never repeat the initial isolation/resize proof.
    if first_baseline.is_some() {
        *isolation_proven = crate::bringup::present_clients(
            server, clients, &apps, &skin, pending_notifies, &mut pending_reload, transfers,
        );
    }

    // Build the window model from presented clients, tiling them if their slot
    // origins collide, and keeping the title bar on-screen.
    let animation_epoch = animation_time_ms().unwrap_or(0);
    let mut wins: Vec<Win> = Vec::new();
    let mut z: Vec<usize> = Vec::new();
    window_lifecycle::Environment {
        apps: &apps, policy: &policy, layout: &ly, skin: &skin, insets: &ins,
        screen: (bw, bh), now: animation_epoch,
    }.create_session(server, clients, &mut wins, &mut z, &session,
                     cfg.settings.scripts.enabled);

    if apps.self_test { desktop_probes::session(&mut session, &apps, &wins); }
    // Wait for a runtime window before announcing keyboard-input readiness.
    let initial_wins = wins.len();
    let mut prev_left = pointer.left;
    let mut prev_right = pointer.right;
    let mut drag: Option<Capture> = None;
    let mut pressed_win = pointer.captured_pid.and_then(|pid|
        wins.iter().position(|w| w.alive && w.pid == pid));
    let mut input_focus: Option<usize> = None; // window under the pointer
    let mut pointer_modes=pointer_runtime::Runtime::default();
    pointer_modes.state.position=(pointer.state.x as i32,pointer.state.y as i32);
    let mut keyboard_lifecycle=core::mem::take(&mut pointer.keyboard);
    let mut last_lx = i32::MIN;
    let mut last_ly = i32::MIN;
    let mut ticks = 0u32;
    // Runtime app-launch: the launcher spawns fresh gui_client windows up to
    // `ly.max_windows`; `next_id` is the tint hint handed to each new client.
    let mut next_id = clients.len() as u32 + 1;
    // Launcher dropdown state. Toggled by the launcher glyph; any click either
    // selects a menu entry (spawn) or dismisses the menu.
    let mut menu_open = false;
    // Keyboard-highlighted launcher row (`cmd_mod`+Space opens the menu, then
    // Up/Down move this selection and Enter launches it). Reset to 0 on each open.
    let mut menu_sel: usize = 0;
    // Active workspace (0-based, concept §11). Only its windows composite and
    // take input; the panel switcher and newly-spawned windows follow it.
    let mut current_ws: usize = 0;
    // One-shot: announce on serial when a window first takes keyboard focus, so
    // headless tests know the compositor is ready to accept injected keystrokes.
    let mut input_ready_announced = false;
    // One-shot: emitted right after the FIRST composited frame is presented,
    // unconditionally (empty desktop or not). This is the clean-boot acceptance
    // marker — proof the desktop reached an interactive, presenting state.
    let mut desktop_ready_announced = false;
    let mut fx = shell.effects;
    // Shortcut policy is replaced on a valid reload.
    let mut shortcuts = cfg.settings.shortcuts;
    // Live window-switcher overlay state (armed by `switch_mod`+Tab, committed on
    // modifier release). Empty/inactive until the user (or the self-test) arms it.
    let mut switcher = Switcher::new();
    // One-shot latch so the config-gated switcher self-test drives exactly one
    // cycle (test.toml `[startup] self_test`), never on the real desktop.
    let mut switcher_st_done = false;
    let mut wg = cfg.settings.widgets;
    // Quick settings changes the live effect and widget flags.
    let mut quicksettings = cfg.settings.quicksettings;
    let mut qs_open = false;
    let mut active_qs_anchor=None;
    let mut qs_st_done = false;
    let mut notifications = cfg.settings.notifications;
    let mut notifs: Vec<Toast> = Vec::new();
    let mut notify_st_done = false;
    let mut notify_expired_logged = false;
    // Deliver notifications received before the session queue existed exactly once.
    for text in pending_notifies.drain(..) {
        notify_post(&mut notifs, &notifications, animation_epoch, &text);
        libdunit::println(&alloc::format!(
            "gui_server: client notify \"{}\" queued={} (enabled={})",
            text,
            notifs.len(),
            notifications.enabled as u32,
        ));
    }
    let mut shortcut_st_done = false;
    // Recycle waits for resize probes and then verifies that reaping frees spawn slots.
    let mut recycle_phase: u8 = 0; // 0=wait 1=draining 2=done
    let mut recycle_pre: usize = 0; // clients present when we closed them
    let mut recycle_deadline: u32 = 0; // tick budget for the drain to finish
    // Durations stay in milliseconds even when rendering misses a frame.
    let mut anim_duration_ms = if fx.anim { fx.anim_ms as u32 } else { 0 };
    let mut ws_switch_tick = animation_epoch;
    let mut menu_anim_tick = animation_epoch;
    let mut previous_animation_time = animation_epoch;
    let mut menu_was_open = false;

    // Report elapsed frame periods using the system uptime clock.
    let mut prev_uptime = 0u64;
    let mut prev_tick = 0u32;
    // Control messages can request reload while idle IPC service is running.
    let mut scene_epoch=0u64;
    let mut damage_cache=damage_runtime::Cache::new();
    let mut reload_requested = pending_reload;
    let mut reload_candidate:Option<(settings::Config,lua_policy::Preflight)>=None;
    let mut menu_entries = apps.launcher.clone();
    let mut custom_menu_origin: Option<(i32,i32)> = None;
    let mut effects = window_effects::Effects::new();
    let mut desktop_tracks=shell_motion::Tracks::default();
    let mut workspace_layer=desktop_motion::Layer::default();
    let mut menu_closed_at=0u64;
    let mut event_engine = lua_policy::EventEngine::new();
    event_engine.set_context(&shell.config,&apps);
    let mut observed: Vec<(u32,bool,bool,WinState)> = Vec::new();
    let mut observed_ws = current_ws;
    let mut observed_focus = None;
    let mut observed_menu = false;
    let mut hovered_component=None;

    let mut scripts = cfg.settings.scripts;
    let mut lua_engine = lua_policy::Engine::new();
    let mut lua_requests = lua_policy::Requests::new();
    let mut lua_ease_out = false;
    let mut lua_expected: Vec<(u32, u32, u32)> = Vec::new();
    // Keep idle-service notifications until this frame applies the current policy.
    let mut notify_requests: Vec<alloc::string::String> = Vec::new();
    let mut shell_clicks: Vec<(u32,bool)> = Vec::new();
    let mut performance = performance::Profiler::new(cfg.diagnostics.performance);
    let mut damage_trace = cfg.diagnostics.damage_trace;
    let mut presentation_pacing = frame_pacing::PresentationPacing::new();
    let mut render=cfg.render;
    loop {
        let frame_start=frame_runtime::monotonic_ns().unwrap_or_else(|error|{
            libdunit::println(&alloc::format!("gui_server: FAIL frame clock {}",error));libdunit::exit(1)
        });
        let perf_work = performance.stamp();
        performance.queues();
        let animation_clock = animation_time_ms();
        let animation_now = animation_clock.unwrap_or(previous_animation_time);
        // Finish client handshakes without blocking desktop rendering.
        let perf_pump = performance.stamp();
        transfers.input(keyboard_lifecycle.current(),pressed_win.filter(|i|wins[*i].alive&&pointer.state.left()).map(|i|wins[i].pid));
        for client in clients.iter(){transfers.peer(client.pid,client.features&gui_protocol_v1::wire::FEATURE_DATA_TRANSFER!=0);}
        transfers.tick(animation_now);
        if let Err(error)=pump_clients(server, clients, &mut reload_requested, &mut notify_requests, &mut performance,transfers,None){
            libdunit::println(&alloc::format!("gui_server: FAIL protocol pump {}",error));libdunit::exit(1);
        }
        performance.finish(performance::Stage::Pump, perf_pump);

        // Validate the entire reload candidate before replacing live state.
        let mut ready_reload=None;
        if reload_requested {
            reload_requested=false;
            let candidate=settings::load_config();
            reload_candidate=None;
            if !candidate.valid {ready_reload=Some(candidate);}
            else {match lua_policy::Preflight::new(&candidate.settings.scripts,bw as u32,bh as u32,0){
                Ok(probe)=>reload_candidate=Some((candidate,probe)),
                Err(reason)=>libdunit::println(&alloc::format!("gui_server: settings reload rejected (Lua preflight {}) — keeping last good",reason)),
            }}
        }
        if let Some((_,probe))=&mut reload_candidate {
            match probe.poll(){
                lua_policy::PreflightPoll::Pending=>{},
                lua_policy::PreflightPoll::Ready=>{ready_reload=reload_candidate.take().map(|(candidate,_)|candidate);},
                lua_policy::PreflightPoll::Failed(reason)=>{libdunit::println(&alloc::format!("gui_server: settings reload rejected (Lua preflight {}) — keeping last good",reason));notify_post(&mut notifs,&notifications,animation_now,"Configuration reload rejected: Lua script failed. Previous desktop retained.");reload_candidate=None;}
            }
        }
        if let Some(ncfg)=ready_reload {
            let next_font=load_font_path(ncfg.settings.desktop.font.as_str());
            let next_shell=if ncfg.valid {
                shell_runtime::Shell::prepare(&ncfg,bw,bh).and_then(|mut shell|{
                    let layout=shell.layout;shell.prepare_text(next_font.as_ref(),&ncfg.apps,&layout)?;Ok(shell)})
            }else{Err(alloc::string::String::from("invalid config"))};
            if let Err(error) = &next_shell { libdunit::println(&alloc::format!("gui_server: shell reload rejected {}",error)); }
            let mut next_icon_theme = if ncfg.valid {
                load_icon_theme(ncfg.settings.desktop.icon_theme.as_str())
            } else { None };
            if !ncfg.valid || next_icon_theme.is_none() || next_shell.is_err() {
                // Retain the previous desktop when any resource fails validation.
                libdunit::println("gui_server: settings reload rejected (invalid config) — keeping last good");
            } else {
                // Re-enter with fresh display resources; clients retain placement below.
                if ncfg.settings.display != applied_display
                    && ncfg.settings.display.width != 0
                    && ncfg.settings.display.height != 0
                {
                    libdunit::println(
                        "gui_server: [display] changed on reload — re-entering session",
                    );
                    for win in wins.iter().filter(|w| w.alive) {
                        if let Some(client) = clients.iter_mut().find(|c| c.pid == win.pid) {
                            client.placement = Some(Placement { cx: win.cx, cy: win.cy, size: (win.sw, win.sh),
                                state: win.state, restore: win.restore, lua_floating: win.lua_floating, st_step: win.st_step,
                                minimized: win.minimized, ws: win.ws });
                        }
                    }
                    // These requests were received in the same IPC batch as the
                    // mode change. Keep them in the caller-owned queue until the
                    // next session can apply its notification policy.
                    pending_notifies.append(&mut notify_requests);
                    pointer.left = prev_left;
                    pointer.right = prev_right;
                    pointer.captured_pid = pressed_win.map(|wi| wins[wi].pid);
                    pointer_modes.revoke(clients);
                    pointer.keyboard = keyboard_lifecycle;
                    performance.flush();
                    return true;
                }
                performance.configure(ncfg.diagnostics.performance);
                damage_trace = ncfg.diagnostics.damage_trace;
                render=ncfg.render;
                lua_engine.cancel();
                event_engine.cancel();
                let _=event_engine.submit(lua_policy::Event{name:"reload",window_id:0,target:0});
                let _=event_engine.submit(lua_policy::Event{name:"theme_change",window_id:0,target:0});
                lua_requests.reset();
                lua_expected.clear();
                scripts = ncfg.settings.scripts;
                lua_ease_out = false;
                for win in wins.iter_mut() {
                    if !scripts.enabled && win.state == WinState::Tiled {
                        set_window_state(
                            server,
                            clients,
                            win,
                            WinState::Floating,
                            bw,
                            bh,
                            &ly,
                            &ins,
                        );
                    }
                }
                theme = next_shell.as_ref().unwrap().palette;
                ly = next_shell.as_ref().unwrap().layout;
                fx = next_shell.as_ref().unwrap().effects;
                shortcuts = ncfg.settings.shortcuts;
                wg = ncfg.settings.widgets;
                // Close flyouts when replacing their policy.
                transfers.configure(ncfg.settings.transfers);
                quicksettings = ncfg.settings.quicksettings;
                qs_open = false;
                active_qs_anchor=None;
                notifications = ncfg.settings.notifications;
                session.set_restore(ncfg.settings.session.restore);
                // Re-resolve the config-driven widget set so a live edit that adds/
                // removes a widgets/<name>.toml (or flips its `enabled`) takes effect.
                desk_widgets = resolve_desk_widgets(ncfg.settings.widgets.corner);
                shell = next_shell.unwrap();
                skin=shell.visuals;
                scene_epoch=scene_epoch.wrapping_add(1);
                shell.describe();
                effects.clear();
                desktop_tracks=shell_motion::Tracks::default();

                hovered_component=None;
                font = next_font;
                policy = ncfg.policy;
                for win in &mut wins {win.app=apps.apps.get(win.app as usize).and_then(|old|ncfg.apps.apps.iter().position(|new|new.id==old.id)).map(|index|index as u8).unwrap_or(u8::MAX);}
                for client in clients.iter_mut(){client.app=apps.apps.get(client.app as usize).and_then(|old|ncfg.apps.apps.iter().position(|new|new.id==old.id)).map(|index|index as u8).unwrap_or(u8::MAX);}
                apps = ncfg.apps;
                event_engine.set_context(&shell.config,&apps);
                menu_entries = apps.launcher.clone();
                custom_menu_origin = None;
                // Refresh work-area insets with the shell configuration.
                panel_edge = ncfg.settings.panel.pos;
                ins = shell.insets;
                anim_duration_ms = if fx.anim { fx.anim_ms as u32 } else { 0 };
                // Every Win caches its own title_h/border (used by outer/contains):
                // re-seed them so existing windows adopt the new geometry.
                for w in wins.iter_mut() {
                    w.traffic_lights = ly.traffic_lights;
                    w.title_h = ly.title_h;
                    w.border = ly.border;
                }
                // Grow scratch space when the new layout needs it.
                let panel_area = bw * ly.panel_h.max(0) as usize;
                let dock_area = ly.dock_w.max(0) as usize * bw.max(bh);
                let menu_area =
                    ly.menu_w.max(0) as usize * (menu_entries.len() * ly.menu_item_h.max(0) as usize);
                let need = panel_area.max(dock_area).max(menu_area).max(1);
                if blur_a.len() < need {
                    blur_a.resize(need, 0);
                    blur_b.resize(need, 0);
                }
                wallpaper = load_wallpaper(ncfg.settings.desktop.wallpaper.as_str(), bw, bh, theme.desktop);

                logo=image_result(ncfg.settings.desktop.logo.as_str(),dunit_image::load(ncfg.settings.desktop.logo.as_str(),skin.logo_raster_size as u32,skin.logo_raster_size as u32));
                app_icons.clear();
                for entry in apps.apps.iter() {
                    app_icons.push(resolve_app_icon(entry.icon.as_str(), &mut next_icon_theme, (ly.dock_w + skin.icon_raster_padding).max(skin.icon_raster_min) as u32));
                }

                if current_ws >= apps.workspaces {
                    current_ws = apps.workspaces.saturating_sub(1);
                }
                // A live shortcut-policy change (switcher toggled off, or the
                // arming modifier swapped) must not leave a stale overlay armed.
                switcher.cancel();
                libdunit::println("gui_server: settings reloaded (config changed)");
            }
        }


        // Apply notification requests after reload so they use the new policy.
        for text in notify_requests.drain(..) {
            notify_post(&mut notifs, &notifications, animation_now, &text);
            libdunit::println(&alloc::format!(
                "gui_server: client notify \"{}\" queued={} (enabled={})",
                text,
                notifs.len(),
                notifications.enabled as u32,
            ));
        }

        window_lifecycle::reap(server, clients, &mut wins, &mut z, &mut drag,
            &mut pressed_win, &mut input_focus, &mut switcher, transfers, &apps);

        window_lifecycle::Environment {
            apps: &apps, policy: &policy, layout: &ly, skin: &skin, insets: &ins,
            screen: (bw, bh), now: animation_now,
        }.update(server, clients, &mut wins, &mut z, &mut session);

        if apps.self_test {
            desktop_probes::resize(server, clients, &mut wins, bw, bh, &ly, &ins, frame_start);
        }

        if apps.self_test && !switcher_st_done && shortcuts.switcher {
            switcher_st_done = desktop_probes::switcher(&mut switcher, &mut wins, &mut z, current_ws);
        }

        if apps.self_test && !notify_st_done && notifications.enabled {
            desktop_probes::notification(&mut notifs, &notifications, animation_now);
            notify_st_done = true;
        }

        if apps.self_test && !qs_st_done && quicksettings.enabled {
            desktop_probes::quicksettings(&mut fx, &mut wg);
            qs_st_done = true;
        }

        if apps.self_test && !shortcut_st_done {
            desktop_probes::shortcuts(&shortcuts, &apps, &mut current_ws);
            shortcut_st_done = true;
        }

        // Recycle only after every startup window has completed its resize probe.
        if apps.self_test {
            match recycle_phase {
                0 => {
                    let live: Vec<usize> =
                        (0..wins.len()).filter(|&i| wins[i].alive).collect();
                    let all_settled =
                        !live.is_empty() && live.iter().all(|&i| wins[i].st_step == 3);
                    // Let the ARGB composition proof finish before closing all windows.
                    if all_settled && argb_blit_logged && !clients.is_empty() {
                        recycle_pre = clients.len();
                        for &wi in &live {
                            effects.start(&wins[wi],&back,bw,bh,wins[wi].outer(),animation_now,anim_duration_ms,
                        animation::Effect::Fade,animation::Easing::ease_out(),true,
                        &window_effects::PaintStyle{visuals:&skin,theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});
                    wins[wi].alive = false;
                            send_input(clients, wins[wi].pid, IN_QUIT, 0, 0, 0);
                        }
                        recycle_deadline = ticks.saturating_add(600); // ~10s budget
                        recycle_phase = 1;
                        libdunit::println(&alloc::format!(
                            "gui_server: recycle self-test closing {} windows (clients={})",
                            live.len(),
                            recycle_pre,
                        ));
                    }
                }
                1 => {
                    // A freed client slot must allow another launch.
                    if clients.len() < recycle_pre {
                        let free_before = clients.len();
                        let mut reopened = 0usize;
                        while try_launch(
                            server,
                            clients,
                            &apps,
                            0, // first registered app (config order, not a hardcoded id)
                            current_ws,
                            &mut next_id,
                            ly.max_windows,
                            &mut notifs,
                            &notifications,
                            animation_now,
                        ) {
                            reopened += 1;
                            if reopened >= ly.max_windows {
                                break; // safety: never loop past the cap
                            }
                        }
                        libdunit::println(&alloc::format!(
                            "gui_server: recycle self-test reaped_to={} reopened={} {} (ceiling lifted)",
                            free_before,
                            reopened,
                            if reopened > 0 { "ok" } else { "FAIL" },
                        ));
                        recycle_phase = 2;
                    } else if ticks >= recycle_deadline {
                        libdunit::println(&alloc::format!(
                            "gui_server: FAIL recycle self-test drain stalled clients={} pre={}",
                            clients.len(),
                            recycle_pre,
                        ));
                        recycle_phase = 2;
                    }
                }
                _ => {}
            }
        }

        for commit in effects.take_geometry_commits(&wins,animation_now) {
            if let Some(w)=wins.iter_mut().find(|w|w.pid==commit.pid) {
                w.cx=commit.cx;w.cy=commit.cy;
                if w.sw!=commit.width||w.sh!=commit.height {
                    w.sw=commit.width;w.sh=commit.height;w.desired_size=Some((commit.width,commit.height));
                    reconfigure_client(server,clients,w.pid,commit.width as u32,commit.height as u32,1);}
            }
        }
        shell.advance(animation_now);
        ins=shell.insets;
        let perf_input = performance.stamp();
        // Route every queued transition in order, bounded independently of
        // queue refill. A stationary pass still reconciles moved window geometry.
        shell_clicks.clear();
        sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,menu_open||qs_open||switcher.active);
        if pointer_modes.take_cancelled(){drag=None;pressed_win=None;input_focus=None;last_lx=i32::MIN;}
        pointer_modes.service(clients,if transfers.active().is_some()||drag.is_some(){None}else{keyboard_lifecycle.current()});
        if pointer_modes.take_cancelled(){drag=None;pressed_win=None;input_focus=None;last_lx=i32::MIN;}
        pointer_modes.state.bound(pointer_modes.bounds(&wins,(bw,bh)));
        let mut batch=frame_pacing::InputBatch::new();
        let mut pointer_sample=batch.next(||libdunit::pointer::read_with_motion(pointer.input)).unwrap_or_else(|error|{
            libdunit::println(&alloc::format!("gui_server: FAIL pointer queue read {}",error));libdunit::exit(1)
        });
        let (mx,my,wx,wy,menu_view,menu_org,menu_ly,qs_applet,qs_edge)=loop {
        let mut m = pointer.state;
        m.wheel = 0;
        match pointer_sample.take().expect("input batch always starts with a record or stationary pass") {
            frame_pacing::PointerSample::Queued(sample) => {
                pointer.modifiers = sample.modifiers as u8;
                let event = sample.event;
                performance.pointer(event.wheel, event.flags & libdunit::pointer::LOST_BEFORE != 0);
                m = libdunit::MouseState { x: event.x, y: event.y,
                    buttons: event.buttons, wheel: event.wheel };
                if event.flags & libdunit::pointer::LOST_BEFORE != 0 {
                    pointer_modes.revoke(clients);
                    // A new owner or an overrun has no trustworthy press history.
                    // Leave cancels client presses/selection without activation.
                    for wi in [pressed_win, input_focus.filter(|wi| Some(*wi) != pressed_win)].into_iter().flatten() {
                        if wins[wi].alive { send_pointer(clients,wins[wi].pid,IN_LEAVE,0,0, 0, pointer.modifiers); }
                    }
                    transfers.cancel();
                    drag = None;
                    pressed_win = None;
                    input_focus = None;
                    last_lx = i32::MIN;
                    prev_left = m.left();
                    prev_right = m.right();
                }
                if pointer_modes.hidden() && !sample.relative { pointer_modes.revoke(clients); }
                let bounds=pointer_modes.bounds(&wins,(bw,bh));
                let (x,y)=pointer_modes.state.sample((event.x as i32,event.y as i32),(sample.dx,sample.dy),sample.relative,bounds);
                m.x=x.max(0) as u32;m.y=y.max(0) as u32;
                if pointer_modes.hidden() && (sample.dx!=0||sample.dy!=0) {
                    if let Some(pid)=pointer_modes.owner(){send_pointer(clients,pid,gui_protocol_v1::control::IN_RELATIVE,sample.dx,sample.dy,m.buttons as u8,pointer.modifiers);}
                }
            }
            frame_pacing::PointerSample::Stationary => {
                if pointer_modes.owner().is_some(){m.x=pointer_modes.state.position.0.max(0) as u32;m.y=pointer_modes.state.position.1.max(0) as u32;}
            },
        }
        // A live modeset can shrink bounds while old events remain queued.
        m.x = m.x.min(bw.saturating_sub(1) as u32);
        m.y = m.y.min(bh.saturating_sub(1) as u32);
        pointer.state = m;
        let mx = m.x as i32;
        let my = m.y as i32;
        transfers.position(mx,my);
        let workspace_input=desktop_tracks.sample(desktop_motion::WORKSPACE,(0,0,bw as i32,bh as i32),animation_now);
        let (wx,wy)=desktop_motion::logical_point(workspace_input.rect,mx,my,bw,bh);
        let left = m.left();
        let press = left && !prev_left;
        if press {if let Some(target)=shell.event_target(mx,my){shell_clicks.push(target);}}
        let release = !left && prev_left;
        let right = m.right();
        let rpress = right && !prev_right;

        // Recompute shell anchors after a live layout change.
        let panel_rect = panel_strip(&ly, &ins, panel_edge, bw, bh);

        let raw_menu_org=custom_menu_origin.unwrap_or_else(||menu_origin(&ly,panel_edge,panel_rect,menu_entries.len()as i32));
        let menu_view=desktop_tracks.sample(desktop_motion::LAUNCHER,(raw_menu_org.0,raw_menu_org.1,ly.menu_w,(menu_entries.len().max(1)as i32)*ly.menu_item_h),animation_now);
        let menu_org=(menu_view.rect.0,menu_view.rect.1);
        let mut menu_ly=ly;menu_ly.menu_w=menu_view.rect.2;menu_ly.menu_item_h=(menu_view.rect.3/menu_entries.len().max(1)as i32).max(1);
        // Anchor the quick-settings flyout to its configured panel cell.
        let (qs_applet,qs_edge)=active_qs_anchor.or_else(||shell.first_quicksettings_anchor()).unwrap_or(((0,0,0,0),panel_edge));
        let in_shell = |x: i32, y: i32| shell.contains(x,y);

        sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,menu_open||qs_open||switcher.active);
        if pointer_modes.take_cancelled(){drag=None;pressed_win=None;input_focus=None;last_lx=i32::MIN;}

        // --- Right-button press: forward to the content window under the cursor
        // as IN_DOWN with button=1 (context-menu trigger). Panel/dock/title are
        // shell-owned and ignore the right button; a right-click there is a no-op.
        if rpress && drag.is_none() && !menu_open && !qs_open && !in_shell(mx, my) {
            let mut zi = z.len();
            while zi > 0 {
                zi -= 1;
                let wi = z[zi];
                if wins[wi].alive && wins[wi].ws == current_ws && !wins[wi].minimized && !effects.covers(wins[wi].pid,animation_now) && effects.input_geometry(&wins[wi],animation_now).in_content(wx,wy) {
                    z.retain(|&i| i != wi);
                    z.push(wi);
                    sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,false);
                    if pointer_modes.take_cancelled(){drag=None;pressed_win=None;input_focus=None;last_lx=i32::MIN;}
                    let (x,y)=effects.input_geometry(&wins[wi],animation_now).content_position(wx,wy);send_pointer(clients, wins[wi].pid,IN_DOWN,x,y, 1, pointer.modifiers);
                    break;
                }
            }
        }

        // --- Button press edge: menu first, then panel, then windows ---
        if press && menu_open {
            // The dropdown is open: a click on an entry spawns that app; any
            // other click just dismisses the menu. Either way the menu closes
            // and the click is consumed (never reaches a window).
            let mut chosen: Option<usize> = None;
            for i in 0..menu_entries.len() {
                let r = menu_item_rect(&menu_ly, menu_org, i as i32);
                if in_rect(r, mx, my) {
                    chosen = Some(i);
                    break;
                }
            }
            menu_open = false;
            if let Some(i) = chosen {
                if let Some(ri) = menu_entries.get(i).copied() {
                    try_launch(
                        server, clients, &apps, ri, current_ws, &mut next_id,
                        ly.max_windows, &mut notifs, &notifications, animation_now,
                    );
                }
            }
        } else if press && qs_open {
            // Toggle the same effect and widget flags used by painting.
            let flyout = qs_flyout_rect(&ly, &skin, qs_edge, qs_applet, 2);
            if in_rect(qs_row_rect(&ly, &skin, flyout, 0), mx, my) {
                fx.blur = !fx.blur;
                libdunit::println(&alloc::format!("gui_server: quicksettings blur={}", fx.blur as u32));
            } else if in_rect(qs_row_rect(&ly, &skin, flyout, 1), mx, my) {
                wg.enabled = !wg.enabled;
                libdunit::println(&alloc::format!("gui_server: quicksettings widgets={}", wg.enabled as u32));
            }
            qs_open = false;
        } else if press && shell.contains(mx,my) {
            if let Some(action) = shell.click(mx,my,&apps) {
                match action {
                    shell_runtime::Action::Launcher(ids,anchor,edge) => {
                        menu_entries = ids;
                        custom_menu_origin = Some(menu_origin(&ly,edge,anchor,menu_entries.len() as i32));
                        menu_open = true; menu_sel = 0; qs_open = false;
                    }
                    shell_runtime::Action::Workspace(ws) => {
                        if current_ws != ws { ws_switch_tick = animation_now; } current_ws = ws;
                    }
                    shell_runtime::Action::QuickSettings(anchor,edge)=>{active_qs_anchor=Some((anchor,edge));qs_open=!qs_open;menu_open=false;},
                    shell_runtime::Action::Group(index,anchor) => shell.open_group(index,anchor,bw,bh),
                    shell_runtime::Action::Command(command) => {
                        if let Ok(action)=settings::policy::action(&command,&apps){desktop_actions::run(action,&mut desktop_actions::Context{session:&mut session,policy:&policy,server,clients,wins:&mut wins,z:&mut z,apps:&apps,shell:&shell,effects:&mut effects,back:&back,bw,bh,now:animation_now,duration:if animation_clock.is_some(){anim_duration_ms}else{0},theme:&theme,ly:&ly,fx:&fx,font:font.as_ref(),ws:&mut current_ws,ws_tick:&mut ws_switch_tick,menu:&mut menu_open,menu_sel:&mut menu_sel,entries:&mut menu_entries,next_id:&mut next_id,notifs:&mut notifs,notifications:&notifications,lua_requests:&mut lua_requests});}
                    }
                    shell_runtime::Action::App(ri) => {
                        let running=(0..wins.len()).rev().find(|&wi|wins[wi].alive&&wins[wi].app as usize==ri);
                        if let Some(wi)=running {
                            if current_ws!=wins[wi].ws {ws_switch_tick=animation_now;current_ws=wins[wi].ws;}
                            if wins[wi].minimized {
                                effects.restore(&wins[wi],animation_now,anim_duration_ms);
                                wins[wi].minimized=false;
                                libdunit::println(&alloc::format!("gui_server: restored pid={}",wins[wi].pid));
                            }
                            raise_window(&mut z,wi);
                        } else {try_launch(server,clients,&apps,ri,current_ws,&mut next_id,ly.max_windows,&mut notifs,&notifications,animation_now);}
                    }
                }
            }
        } else if press {
            let mut hit: Option<usize> = None;
            let mut zi = z.len();
            while zi > 0 {
                zi -= 1;
                let wi = z[zi];
                if wins[wi].alive && wins[wi].ws == current_ws && !wins[wi].minimized && !effects.covers(wins[wi].pid,animation_now) && effects.input_geometry(&wins[wi],animation_now).contains(wx,wy) {
                    hit = Some(wi);
                    break;
                }
            }
            if let Some(wi)=hit {
                let visual=effects.input_geometry(&wins[wi],animation_now);
                effects.cancel_geometry(wins[wi].pid);
                if (visual.sw,visual.sh)!=(wins[wi].sw,wins[wi].sh){wins[wi].desired_size=Some((visual.sw,visual.sh));reconfigure_client(server,clients,wins[wi].pid,visual.sw as u32,visual.sh as u32,1);}
                wins[wi].cx=visual.cx;wins[wi].cy=visual.cy;wins[wi].sw=visual.sw;wins[wi].sh=visual.sh;
                // Raise: move wi to the top of the z-order.
                z.retain(|&i| i != wi);
                z.push(wi);
                if wins[wi].in_close(wx, wy, &skin) {
                    effects.start(&wins[wi],&back,bw,bh,wins[wi].outer(),animation_now,anim_duration_ms,
                        animation::Effect::Fade,animation::Easing::ease_out(),true,
                        &window_effects::PaintStyle{visuals:&skin,theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});
                    wins[wi].alive = false;
                    send_input(clients, wins[wi].pid, IN_QUIT, 0, 0, 0);
                    drag = None;
                    // Remember where it was before it goes away, so reopening the
                    // app restores its last geometry.
                    session_remember(&mut session, &apps, &wins[wi]);
                    session.persist();
                } else if wins[wi].in_min(wx, wy, &skin) {
                    // Minimize: drop it from compositing/input; the dock task
                    // switcher restores it on click.
                    let target=shell.app_target(wins[wi].app as usize,&apps).unwrap_or((bw as i32/2,bh as i32-1,1,1));
                    effects.minimize(&wins[wi],&back,bw,bh,target,animation_now,anim_duration_ms,
                        &window_effects::PaintStyle{visuals:&skin,theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});
                    if let Some(name)=policy.for_app(apps.apps.get(wins[wi].app as usize).map(|a|a.id.as_str()).unwrap_or("")).animation {
                        let (effect,easing)=window_effects::preset(&name,wins[wi].outer(),target);effects.configure(wins[wi].pid,effect,anim_duration_ms,easing,animation_now);
                    }
                    wins[wi].minimized = true;
                    drag = None;
                } else if wins[wi].in_max(wx, wy, &skin) {
                    // Maximize and restore share the client reconfiguration path.
                    toggle_maximize(server, clients, &mut wins[wi], bw, bh, &ly, &ins);
                    drag = None;
                    // Persist the new maximize state (with the preserved floating
                    // rect) so a reopen restores it in the same state.
                    session_remember(&mut session, &apps, &wins[wi]);
                    session.persist();
                } else if let Some(edges) = wins[wi].resize_edges(wx, wy, &skin) {
                    wins[wi].lua_floating = true;
                    wins[wi].motion = None;
                    drag = Some(Capture::Resize {
                        window: wi, origin: wins[wi].outer(), pointer: (wx, wy), edges,
                    });
                } else if wins[wi].in_title(wx, wy) {
                    if scripts.enabled { wins[wi].lua_floating = true; }
                    drag = Some(begin_window_move(server, clients, &mut wins[wi], wi,
                                                  (wx, wy), (bw, bh), &ly, &ins));
                } else if wins[wi].in_content(wx, wy) {
                    // Click landed on client content: forward it to the client.
                    sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,false);
                    if pointer_modes.take_cancelled(){drag=None;input_focus=None;last_lx=i32::MIN;}
                    let (x,y)=effects.input_geometry(&wins[wi],animation_now).content_position(wx,wy);send_pointer(clients, wins[wi].pid,IN_DOWN,x,y, 0, pointer.modifiers);
                    transfers.begin_press(wins[wi].pid,mx,my);
                    pressed_win = Some(wi);
                }
            }
        }
        if let Some(capture) = drag {
            let wi = capture.window();
            if !wins[wi].alive || wins[wi].minimized || wins[wi].ws != current_ws {
                drag = None;
            } else {
                // Apply the release packet's position before persisting the result.
                let minimum = window_minimum(&wins[wi], &skin, &policy, &apps);
                advance_window_capture(server, clients, &mut wins[wi], capture,
                                       (wx, wy), (bw, bh), &ins, minimum);
                if !left {
                    session_remember(&mut session, &apps, &wins[wi]);
                    session.persist();
                    drag = None;
                }
            }
        }

        sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,menu_open||qs_open||switcher.active);
        if pointer_modes.take_cancelled(){drag=None;pressed_win=None;input_focus=None;last_lx=i32::MIN;}

        // --- Pointer focus + motion routed to the topmost content window ---
        let cur = if drag.is_some() || shell.contains(mx,my) || menu_open || qs_open {
            None
        } else {
            let mut c = None;
            let mut zi = z.len();
            while zi > 0 {
                zi -= 1;
                let wi = z[zi];
                if wins[wi].alive && wins[wi].ws == current_ws && !wins[wi].minimized && !effects.covers(wins[wi].pid,animation_now) && effects.input_geometry(&wins[wi],animation_now).in_content(wx,wy) {
                    c = Some(wi);
                    break;
                }
            }
            c
        };
        // Preserve the existing client's implicit button grab until a real
        // transfer takes it over, including a first movement outside its window.
        let cur=if left&&!transfers.dragging(){pressed_win.filter(|wi|wins[*wi].alive).or(cur)}else{cur};
        let cur=pointer_modes.owner().and_then(|pid|wins.iter().position(|w|w.pid==pid&&w.alive)).or(cur);
        if cur != input_focus {
            if let Some(old) = input_focus {
                if wins[old].alive {
                    send_pointer(clients, wins[old].pid, IN_LEAVE, 0, 0, 0, pointer.modifiers);
                }
            }
            input_focus = cur;
            last_lx = i32::MIN; // force the next motion to be delivered
        }
        if transfers.dragging() {
            let hit=cur.map(|wi|{let (x,y)=effects.input_geometry(&wins[wi],animation_now).content_position(wx,wy);(wins[wi].pid,x,y)});
            let opener=shell.app_at(mx,my).filter(|index|apps.apps.get(*index).is_some_and(|entry|!entry.open_formats.is_empty()));
            if let Some(index)=opener{transfers.pointer_opener(index,&apps.apps[index].open_formats,transfers.action(pointer.modifiers),animation_now);}
            else{transfers.pointer(hit,transfers.action(pointer.modifiers),animation_now);}
        }
        if let Some(wi) = cur.filter(|_|!transfers.dragging()) {
            let (lx,ly)=effects.input_geometry(&wins[wi],animation_now).content_position(wx,wy);
            if !pointer_modes.hidden() && (lx != last_lx || ly != last_ly) {
                send_pointer(clients, wins[wi].pid, IN_MOVE, lx, ly, 0, pointer.modifiers);
                last_lx = lx;
                last_ly = ly;
            }
        }

        // --- Button release: deliver UP to the window that captured the press ---
        if release {
            if transfers.dragging(){
                if let Some(wi)=transfers.active().and_then(|(_,_,target,_)|target).and_then(|pid|wins.iter().position(|w|w.pid==pid&&w.alive)){
                    raise_window(&mut z,wi);
                    sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,false);
                    if pointer_modes.take_cancelled(){drag=None;input_focus=None;last_lx=i32::MIN;}
                }
                transfers.drop_pointer(animation_now);
                while let Some((index,id))=transfers.take_launch(){
                    let launched=try_launch(server,clients,&apps,index,current_ws,&mut next_id,ly.max_windows,&mut notifs,&notifications,animation_now);
                    let pid=if launched{clients.last().map(|client|client.pid)}else{None};
                    transfers.launch_result(id,pid,animation_now);
                }
            }
            else if let Some(wi) = pressed_win {
                if wins[wi].alive {
                    let (x,y)=effects.input_geometry(&wins[wi],animation_now).content_position(wx,wy);send_pointer(clients, wins[wi].pid,IN_UP,x,y, 0, pointer.modifiers);
                }
            }
            pressed_win = None;
            transfers.end_press();
        }
        prev_left = left;
        prev_right = right;


        let focused = keyboard_window(&wins,&z,current_ws);

        // --- Mouse wheel: forward the tick's scroll delta to the focused window
        // as IN_SCROLL (delta in `lx`). Clients that don't scroll ignore it; the
        // terminal uses it to move through its scrollback history.
        if m.wheel != 0 {
            if let Some(wi) = focused {
                send_pointer(clients, wins[wi].pid, IN_SCROLL, m.wheel, 0, 0, pointer.modifiers);
            }
        }

        pointer_sample=batch.next(||libdunit::pointer::read_with_motion(pointer.input)).unwrap_or_else(|error|{
            libdunit::println(&alloc::format!("gui_server: FAIL pointer queue read {}",error));libdunit::exit(1)
        });
        if pointer_sample.is_none(){break (mx,my,wx,wy,menu_view,menu_org,menu_ly,qs_applet,qs_edge);}
        };
        let focused=keyboard_window(&wins,&z,current_ws);

        // Old clients receive cooked makes; negotiated clients also receive paired releases.
        if focused.is_some() && wins.len() > initial_wins && !input_ready_announced {
            libdunit::println("gui_server: desktop input ready");
            input_ready_announced = true;
        }
        while let Some(ev) = libdunit::get_key_event() {
            performance.key();
            // cmd_mod is configured desktop policy, Escape is its release action.
            if ev.pressed && ev.scancode==SC_ESC && ev.mods&shortcuts.cmd_mod.mask()!=0 && pointer_modes.owner().is_some(){
                pointer_modes.revoke(clients);continue;
            }
            if ev.pressed&&ev.scancode==SC_ESC&&transfers.active().is_some(){transfers.cancel();continue;}
            // Synchronize before every key, not once per frame: a preceding
            // shortcut or switcher commit may already have raised another window.
            sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,menu_open||qs_open||switcher.active);
            if pointer_modes.take_cancelled(){drag=None;pressed_win=None;input_focus=None;last_lx=i32::MIN;}
            let key_observation=keyboard_lifecycle.observe(ev.scancode,ev.pressed);
            if let Some(pid)=key_observation.released_to {
                send_input(clients,pid,IN_KEY_UP,ev.scancode as i32,ev.mods as i32,0);
            }
            if qs_open && ev.pressed && ev.scancode == SC_ESC {
                qs_open = false;
                active_qs_anchor = None;
                continue;
            }
            // The switcher consumes its chord so it cannot leak into the client key stream.
            if shortcuts.switcher {
                let mask = shortcuts.switch_mod.mask();
                let mod_held = (ev.mods & mask) != 0;
                if ev.pressed && ev.scancode == SC_TAB && mod_held {
                    if switcher.active {
                        switcher.advance(ev.shift());
                    } else {
                        let mru = switcher_mru(&z, &wins, current_ws);
                        switcher.begin(&mru, ev.shift());
                    }
                    continue; // swallow Tab — it never reaches a client
                }
                if switcher.active {
                    if ev.pressed && ev.scancode == SC_ESC {
                        switcher.cancel();
                        continue;
                    }
                    if !mod_held {
                        // Arming modifier released → commit the highlighted window.
                        if let Some(sel) = switcher.commit() {
                            if wins.get(sel).map_or(false, |w| w.alive) {
                                if wins[sel].minimized{effects.restore(&wins[sel],animation_now,anim_duration_ms);}
                                wins[sel].minimized = false;
                                raise_window(&mut z, sel);
                                libdunit::println(&alloc::format!(
                                    "gui_server: switcher raised pid={}",
                                    wins[sel].pid
                                ));
                            }
                        }
                        // Fall through: this event is the modifier release itself,
                        // dropped by the release/modifier filter just below.
                    }
                }
            }

            // Modal launcher navigation consumes keys until dismissal or launch.
            if menu_open {
                if ev.pressed {
                    match ev.scancode {
                        SC_ESC => menu_open = false,
                        SC_UP => {
                            if menu_sel > 0 {
                                menu_sel -= 1;
                            }
                        }
                        SC_DOWN => {
                            if menu_sel + 1 < menu_entries.len() {
                                menu_sel += 1;
                            }
                        }
                        SC_ENTER => {
                            menu_open = false;
                            if let Some(ri) = menu_entries.get(menu_sel).copied() {
                                try_launch(
                                    server, clients, &apps, ri, current_ws, &mut next_id,
                                    ly.max_windows, &mut notifs, &notifications, animation_now,
                                );
                            }
                        }
                        _ => {}
                    }
                }
                continue; // menu is modal — no key reaches a client while it is open
            }

            // Apply configured global shortcuts before resolving client keyboard focus.
            let custom=if ev.pressed {policy.bindings.iter().find(|b|b.scancode==ev.scancode&&b.modifiers==ev.mods).map(|b|b.action.clone())}else{None};
            if let Some(action)=custom {desktop_actions::run(action,&mut desktop_actions::Context{session:&mut session,policy:&policy,server,clients,wins:&mut wins,z:&mut z,apps:&apps,shell:&shell,effects:&mut effects,back:&back,bw,bh,now:animation_now,duration:if animation_clock.is_some(){anim_duration_ms}else{0},theme:&theme,ly:&ly,fx:&fx,font:font.as_ref(),ws:&mut current_ws,ws_tick:&mut ws_switch_tick,menu:&mut menu_open,menu_sel:&mut menu_sel,entries:&mut menu_entries,next_id:&mut next_id,notifs:&mut notifs,notifications:&notifications,lua_requests:&mut lua_requests});continue;}
            let action=match match_shortcut(ev.scancode,ev.mods,ev.pressed,&shortcuts,apps.workspaces){
                ShortcutAction::Workspace(ws)=>Some(settings::policy::Action::Workspace(ws)),ShortcutAction::Launcher=>Some(settings::policy::Action::Launcher),
                ShortcutAction::CloseWindow=>Some(settings::policy::Action::Close),ShortcutAction::MaximizeWindow=>Some(settings::policy::Action::Maximize),ShortcutAction::None=>None};
            if let Some(action)=action {desktop_actions::run(action,&mut desktop_actions::Context{session:&mut session,policy:&policy,server,clients,wins:&mut wins,z:&mut z,apps:&apps,shell:&shell,effects:&mut effects,back:&back,bw,bh,now:animation_now,duration:if animation_clock.is_some(){anim_duration_ms}else{0},theme:&theme,ly:&ly,fx:&fx,font:font.as_ref(),ws:&mut current_ws,ws_tick:&mut ws_switch_tick,menu:&mut menu_open,menu_sel:&mut menu_sel,entries:&mut menu_entries,next_id:&mut next_id,notifs:&mut notifs,notifications:&notifications,lua_requests:&mut lua_requests});continue;}

            if !ev.pressed || is_modifier_scancode(ev.scancode) {
                continue;
            }
            // Re-resolve after shortcuts have changed z/workspace. Negotiated
            // clients suppress held repeats that began in an earlier focus
            // scope; the legacy make-only stream remains unchanged.
            sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,menu_open||qs_open||switcher.active);
            if pointer_modes.take_cancelled(){drag=None;pressed_win=None;input_focus=None;last_lx=i32::MIN;}
            if let Some(wi) = keyboard_window(&wins,&z,current_ws) {
                let pid=wins[wi].pid;
                let lifecycle=clients.iter().find(|c|c.pid==pid).is_some_and(|c|c.features&gui_protocol_v1::wire::FEATURE_INPUT_LIFECYCLE!=0);
                if !lifecycle || keyboard_lifecycle.deliver_make(ev.scancode,pid,key_observation.fresh) {
                    send_input(clients,pid,IN_KEY,ev.scancode as i32,ev.mods as i32,ev.ascii as u32);
                }
            }
        }
        sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,menu_open||qs_open||switcher.active);
        if pointer_modes.take_cancelled(){drag=None;pressed_win=None;input_focus=None;last_lx=i32::MIN;}

        performance.finish(performance::Stage::Input, perf_input);
        for win in wins.iter_mut().filter(|w|w.alive&&!w.rule_applied) {
            let app=apps.apps.get(win.app as usize).map(|a|a.id.as_str()).unwrap_or("");
            let rule=policy.for_app(app);
            if let Some(ws)=rule.workspace {win.ws=ws;}
            if let Some(x)=rule.x {win.cx=x.clamp(ins.left,(bw as i32-ins.right-win.sw).max(ins.left));}
            if let Some(y)=rule.y {win.cy=y.clamp(ins.top+ly.title_h,(bh as i32-ins.bottom-win.sh).max(ins.top+ly.title_h));}
            if rule.width.is_some()||rule.height.is_some() {reconfigure_client(server,clients,win.pid,
                rule.width.unwrap_or(win.sw).min(bw as i32)as u32,rule.height.unwrap_or(win.sh).min(bh as i32)as u32,1);}
            if let Some(floating)=rule.floating {win.lua_floating=floating;}
            if let Some(opacity)=rule.opacity {win.opacity=opacity;}
            if let Some(name)=rule.animation.as_deref().filter(|_|animation_clock.is_some()){let target=shell.app_target(win.app as usize,&apps).unwrap_or((bw as i32/2,bh as i32-1,1,1));
                let (effect,easing)=window_effects::preset(name,win.outer(),target);
                effects.start(win,&back,bw,bh,target,animation_now,anim_duration_ms,effect,easing,false,
                    &window_effects::PaintStyle{visuals:&skin,theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});}
            win.rule_applied=true;
        }
        if scripts.enabled && !scripts.events.as_str().is_empty() {
            for win in &wins {
                let old=observed.iter().find(|old|old.0==win.pid);
                let name=match old {
                    None if win.alive=>Some("window_open"),
                    Some(old) if old.1&&!win.alive=>Some("window_close"),
                    Some(old) if old.2!=win.minimized=>Some(if win.minimized{"window_minimize"}else{"window_restore"}),
                    Some(old) if old.3!=win.state=>Some("window_maximize"),_=>None,
                };
                if let Some(name)=name {let _=event_engine.submit(lua_policy::Event{name,window_id:win.pid,target:win.app as u32+1});}
            }
            for old in &observed {if old.1&&!wins.iter().any(|w|w.pid==old.0){let _=event_engine.submit(lua_policy::Event{name:"window_close",window_id:old.0,target:0});}}
            let focus=focused.map(|i|wins[i].pid);
            if focus!=observed_focus {if let Some(pid)=focus {let _=event_engine.submit(lua_policy::Event{name:"window_focus",window_id:pid,target:wins.iter().find(|w|w.pid==pid).map_or(0,|w|w.app as u32+1)});}}
            if current_ws!=observed_ws {let _=event_engine.submit(lua_policy::Event{name:"workspace_change",window_id:0,target:current_ws as u32});}
            if menu_open!=observed_menu {let _=event_engine.submit(lua_policy::Event{name:if menu_open{"launcher_open"}else{"launcher_close"},window_id:0,target:0});}
            let hover=shell.event_target(mx,my);
            if hover!=hovered_component {if let Some((target,_))=hover {let _=event_engine.submit(lua_policy::Event{name:"panel_hover",window_id:0,target});}hovered_component=hover;}
            for (target,widget) in shell_clicks.drain(..) {let _=event_engine.submit(lua_policy::Event{name:if widget{"widget_click"}else{"panel_click"},window_id:0,target});}
            for target in shell.due_widget_updates(animation_now){let _=event_engine.submit(lua_policy::Event{name:"widget_update",window_id:0,target});}
            observed.clear();observed.extend(wins.iter().map(|w|(w.pid,w.alive,w.minimized,w.state)));
            observed_ws=current_ws;observed_focus=focus;observed_menu=menu_open;
        }
        match event_engine.poll(&scripts,scripts.events.as_str()) {
            lua_policy::EventPoll::Pending=>{},
            lua_policy::EventPoll::Failed(reason)=>libdunit::println(&alloc::format!("gui_server: Lua event rejected {}; native animation retained",reason)),
            lua_policy::EventPoll::Ready(actions,commands,widgets)=>'batch:{
                let commands:Result<Vec<_>,_>=commands.iter().map(|command|settings::policy::action(command,&apps)).collect();
                let Ok(commands)=commands else {libdunit::println("gui_server: Lua command rejected; native state retained");break 'batch;};
                let mut actions=actions;
                if !fx.anim||animation_clock.is_none(){for action in &mut actions{action.duration_ms=0;}}
                let mut next_desktop_tracks=desktop_tracks.clone();
                for action in actions.iter().filter(|a|a.window_id==0&&(513..=515).contains(&a.target)){if next_desktop_tracks.submit(action,animation_now).is_err(){libdunit::println("gui_server: desktop animation budget rejected");break 'batch;}}
                if let Err(error)=shell.apply_response(&actions,&widgets,animation_now,font.as_ref(),&apps,&ly){libdunit::println(&alloc::format!("gui_server: Lua scene batch rejected {}",error));break 'batch;}
                desktop_tracks=next_desktop_tracks;
                for update in &widgets{if let Some(component)=shell.config.components.get(update.target as usize-1){libdunit::println(&alloc::format!("gui_server: widget updated component={} text={}",component.id,component.label));}}
                for action in actions.iter().filter(|a|a.window_id==0){libdunit::println(&alloc::format!("gui_server: component animation committed target={}",action.target));}
                for command in commands {desktop_actions::run(command,&mut desktop_actions::Context{session:&mut session,policy:&policy,server,clients,wins:&mut wins,z:&mut z,apps:&apps,shell:&shell,effects:&mut effects,back:&back,bw,bh,now:animation_now,duration:if animation_clock.is_some(){anim_duration_ms}else{0},theme:&theme,ly:&ly,fx:&fx,font:font.as_ref(),ws:&mut current_ws,ws_tick:&mut ws_switch_tick,menu:&mut menu_open,menu_sel:&mut menu_sel,entries:&mut menu_entries,next_id:&mut next_id,notifs:&mut notifs,notifications:&notifications,lua_requests:&mut lua_requests});}
                for mut action in actions {
                if !fx.anim||animation_clock.is_none(){action.duration_ms=0;}
                if action.window_id==0{continue;}

                let easing=animation::Easing::new(action.curve).unwrap_or_else(|_|animation::Easing::ease_out());
                if action.property.is_none(){if let Some((source,target))=effects.snapshot_geometry(action.window_id){
                    let effect=match action.effect {lua_policy::Effect::Fade=>animation::Effect::Fade,lua_policy::Effect::Slide=>animation::Effect::Slide,
                        lua_policy::Effect::Scale=>animation::Effect::Scale,lua_policy::Effect::Spring=>animation::Effect::Spring,
                        lua_policy::Effect::Genie=>animation::Effect::Genie(window_effects::target_edge(source,target))};
                    effects.configure(action.window_id,effect,action.duration_ms,easing,animation_now);
                }}
                if let Some(win)=wins.iter_mut().find(|w|w.pid==action.window_id) {
                    if let Some(property)=action.property{if property==lua_policy::Property::Opacity{win.born=animation_now.saturating_sub(anim_duration_ms as u64);}effects.scalar(win.pid,property,animation_now,action.duration_ms,action.from,action.to,easing);}
                    else {
                        let target=shell.app_target(win.app as usize,&apps).unwrap_or((bw as i32/2,bh as i32-1,1,1));
                        let effect=match action.effect {lua_policy::Effect::Fade=>animation::Effect::Fade,lua_policy::Effect::Slide=>animation::Effect::Slide,
                            lua_policy::Effect::Scale=>animation::Effect::Scale,lua_policy::Effect::Spring=>animation::Effect::Spring,
                            lua_policy::Effect::Genie=>animation::Effect::Genie(window_effects::target_edge(win.outer(),target))};
                        if !effects.configure(win.pid,effect,action.duration_ms,easing,animation_now) {
                            effects.start(win,&back,bw,bh,target,animation_now,action.duration_ms,effect,easing,false,
                                &window_effects::PaintStyle{visuals:&skin,theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});
                        }
                    }
                    libdunit::println(&alloc::format!("gui_server: Lua animation committed pid={} duration={}",win.pid,action.duration_ms));
                }
            }
            }
        }

        // Lua policy is evaluated outside the compositor process. Only topology,
        // workspace or configuration changes start a new request; frame rendering
        // never waits for the VM. Dragging releases a window from Lua tiling
        // until an explicit retile action, so policy cannot fight the pointer.
        let mut lua_failure = None;
        if scripts.enabled {
            let ids: Vec<u32> = wins
                .iter()
                .filter(|w| {
                    w.alive
                        && !w.minimized
                        && w.ws == current_ws
                        && !w.lua_floating
                        && w.state != WinState::Maximized
                })
                .map(|w| w.pid)
                .collect();
            let width = (bw as i32 - ins.left - ins.right).max(1) as u32;
            let height = (bh as i32 - ins.top - ins.bottom).max(1) as u32;
            let changed = lua_requests.begin(&ids, width, height, current_ws);
            if changed {
                lua_expected.clear();
                lua_engine.cancel();
                if !ids.is_empty() {
                    if let Err(reason) = lua_engine.start(&scripts, width, height, ids.len()) {
                        lua_failure = Some(reason);
                    }
                }
            }
            match lua_engine.poll() {
                lua_policy::Poll::Ready(layout_policy) => {
                    let ids = lua_requests.ids();
                    // Reject the whole layout if an app cannot fit its minimum size.
                    let fits = layout_policy
                        .rects
                        .iter()
                        .zip(ids)
                        .all(|(r, pid)| wins.iter().find(|w| w.pid == *pid && w.alive)
                            .is_some_and(|w| {
                                let minimum = window_minimum(w, &skin, &policy, &apps);
                                r.w >= minimum.0 + ly.border * 2
                                    && r.h >= minimum.1 + ly.title_h + ly.border * 2
                            }));
                    if fits {
                        lua_expected.clear();
                        for (pid, rect) in ids.iter().zip(&layout_policy.rects) {
                            if let Some(win) = wins.iter_mut().find(|w| w.pid == *pid && w.alive) {
                                if win.state == WinState::Floating {
                                    win.restore = Some((win.cx, win.cy, win.sw, win.sh));
                                }
                                lua_expected.push((win.pid, (rect.w - ly.border * 2) as u32,
                                    (rect.h - ly.title_h - ly.border * 2) as u32));
                                win.state = WinState::Tiled;
                                let target = (ins.left + rect.x + ly.border,
                                    ins.top + rect.y + ly.title_h + ly.border);
                                let duration_ms = if fx.anim { layout_policy.duration_ms } else { 0 };
                                if duration_ms > 0 && target != (win.cx, win.cy) {
                                    win.motion = Some(Motion { from: (win.cx, win.cy), to: target,
                                        start: animation_now, duration_ms, ease_out: layout_policy.ease_out });
                                } else {
                                    win.motion = None;
                                    (win.cx, win.cy) = target;
                                }
                                reconfigure_client(
                                    server,
                                    clients,
                                    win.pid,
                                    (rect.w - ly.border * 2) as u32,
                                    (rect.h - ly.title_h - ly.border * 2) as u32,
                                    1,
                                );
                            }
                        }
                        anim_duration_ms = if fx.anim { layout_policy.duration_ms } else { 0 };
                        lua_ease_out = layout_policy.ease_out;
                        libdunit::println(&alloc::format!(
                            "gui_server: Lua policy applied windows={} duration_ms={} easing={}",
                            ids.len(),
                            layout_policy.duration_ms,
                            if lua_ease_out { "ease_out" } else { "linear" }
                        ));
                    } else {
                        lua_failure = Some("cell smaller than window minimum");
                    }
                }
                lua_policy::Poll::Failed(reason) => {
                    lua_failure = Some(reason);
                }
                lua_policy::Poll::Pending => {}
            }
        }

        if let Some(reason) = lua_failure {
            lua_engine.cancel();
            lua_expected.clear();
            lua_ease_out = false;
            let mut restored = 0usize;
            for win in wins.iter_mut().filter(|w| w.alive) {
                if win.state == WinState::Tiled {
                    win.motion = None;
                    set_window_state(server, clients, win, WinState::Floating,
                        bw, bh, &ly, &ins);
                    restored += 1;
                }
            }
            // Keep lua_requests: this topology has already failed. Re-attempt
            // only on topology/workspace/geometry change or explicit reload,
            // so a persistent syntax/quota error cannot create a toast storm.
            libdunit::println(&alloc::format!(
                "gui_server: Lua policy rejected: {}; floating fallback restored={}", reason, restored));
            let message = alloc::format!("Lua layout failed: {}. Floating layout restored.", reason);
            notify_post(&mut notifs, &notifications, animation_now, &message);
            libdunit::println(&alloc::format!("gui_server: Lua fallback notification queued={}",
                u8::from(notifs.iter().any(|toast| toast.text == message))));
        }

        for win in wins.iter_mut() {
            if let Some(motion) = win.motion {
                let progress = animation::progress(animation_now, motion.start, if animation_clock.is_some() { motion.duration_ms } else { 0 }, motion.ease_out) as i64;
                win.cx = (motion.from.0 as i64 + (motion.to.0 - motion.from.0) as i64 * progress / 255) as i32;
                win.cy = (motion.from.1 as i64 + (motion.to.1 - motion.from.1) as i64 * progress / 255) as i32;
                if progress == 255 { win.motion = None; }
            }
        }

        if !lua_expected.is_empty() && lua_expected.iter().all(|(pid, width, height)|
            clients.iter().any(|c| c.pid == *pid && c.surf_w == *width && c.surf_h == *height)) {
            libdunit::println(&alloc::format!("gui_server: Lua geometry committed windows={}", lua_expected.len()));
            lua_expected.clear();
        }

        // Policy/Lua commands may have changed focus after keyboard dispatch.
        sync_keyboard_focus(&mut pointer_modes,&mut keyboard_lifecycle,clients,&wins,&z,current_ws,menu_open||qs_open||switcher.active);
        if pointer_modes.take_cancelled(){drag=None;pressed_win=None;input_focus=None;last_lx=i32::MIN;}

        let damage_duration=if animation_clock.is_some(){anim_duration_ms}else{0};
        let mut desktop_widget_stats=libdunit::SystemStats::default();
        let desktop_widget_stats_valid=wg.enabled&&!desk_widgets.is_empty()
            &&libdunit::get_system_stats(&mut desktop_widget_stats)>=0;
        let widget_seconds=if desktop_widget_stats_valid&&desktop_widget_stats.uptime_hz>0 {
            desktop_widget_stats.uptime_ticks/desktop_widget_stats.uptime_hz
        }else{0};
        let workspace_damage_view=desktop_tracks.sample(desktop_motion::WORKSPACE,(0,0,bw as i32,bh as i32),animation_now);
        let damage_blocked=transfers.active().is_some()||menu_open||menu_was_open||qs_open||switcher.active||!notifs.is_empty()
            ||(menu_closed_at!=0&&animation_now.saturating_sub(menu_closed_at)<damage_duration as u64)
            ||animation::progress(animation_now,ws_switch_tick,damage_duration,lua_ease_out)<255
            ||desktop_tracks.animating(animation_now)||desktop_motion::changed(&workspace_damage_view,bw,bh);
        let perf_compose = performance.begin_compose();
        let composition=damage_cache.compose(damage_runtime::Context{
            wins:&wins,z:&z,focused,workspace:current_ws,mouse:(mx,my),cursor_visible:!pointer_modes.hidden(),now:animation_now,
            duration:damage_duration,epoch:scene_epoch,theme:&theme,layout:&ly,effects:&fx,
            shell:&shell,window_effects:&effects,blocked:damage_blocked,
            trace: damage_trace,
            widgets:(wg.enabled,widget_seconds,desktop_widget_stats.memory_used_percent(),desktop_widget_stats.live_processes()),
        },clients,&mut back,bw,bh,&mut performance);
        let full = matches!(composition, damage_runtime::Composition::Full);
        performance.composition(!full,damage_blocked);
        if full {
            desktop_painter::paint(desktop_painter::Context {
                back: &mut back,
                bw,
                bh,
                fb: &fb,
                wallpaper: wallpaper.as_deref(),
                theme: &theme,
                ly: &ly,
                skin: &skin,
                fx: &fx,
                font: font.as_ref(),
                apps: &apps,
                app_icons: &app_icons,
                logo: logo.as_ref(),
                wins: &wins,
                z: &z,
                focused,
                current_ws,
                ins: &ins,
                effects: &mut effects,
                shell: &mut shell,
                blur_a: &mut blur_a,
                blur_b: &mut blur_b,
                win_backing: &mut win_backing,
                argb_blit_logged: &mut argb_blit_logged,
                animation_now,
                clock_valid: animation_clock.is_some(),
                anim_duration_ms,
                lua_ease_out,
                ws_switch_tick,
                desktop_tracks: &desktop_tracks,
                workspace_layer: &mut workspace_layer,
                widgets_enabled: wg.enabled,
                desk_widgets: &desk_widgets,
                desktop_widget_stats,
                desktop_widget_stats_valid,
                mx,
                my,
                wx,
                wy,
                cursor_visible: !pointer_modes.hidden(),
                menu_open,
                menu_was_open: &mut menu_was_open,
                menu_anim_tick: &mut menu_anim_tick,
                menu_closed_at: &mut menu_closed_at,
                menu_sel,
                menu_entries: &menu_entries,
                menu_ly,
                menu_org,
                menu_view,
                switcher: &switcher,
                quicksettings_enabled: quicksettings.enabled,
                qs_open,
                qs_edge,
                qs_applet,
                notifs: &mut notifs,
                notifications: &notifications,
                notify_st_done,
                notify_expired_logged: &mut notify_expired_logged,
                transfers,
                performance: &mut performance,
                desktop_ready_announced: &mut desktop_ready_announced,
            });
        }
        performance.finish_compose(perf_compose);
        for c in clients.iter_mut(){c.dirty=None;c.buffers.copied=0;}
        let perf_done = performance.stamp();
        complete_desktop_frames(server,clients);
        if transfers.tracing(){for client in clients.iter_mut().filter(|client|client.ready&&client.presented){
            let size=(client.surf_w,client.surf_h);
            if client.reported_size!=Some(size){
                client.reported_size=Some(size);
                if let Some(app)=apps.apps.get(client.app as usize){libdunit::println(&alloc::format!("[GUI-SIZE] app={} width={} height={}",app.id,size.0,size.1));}
            }
        }}
        performance.finish(performance::Stage::FrameDone,perf_done);
        previous_animation_time = animation_now;
        performance.finish(performance::Stage::Work,perf_work);
        transfers.input(keyboard_lifecycle.current(),pressed_win.filter(|i|wins[*i].alive&&pointer.state.left()).map(|i|wins[i].pid));
        transfers.flush();
        let deadline=frame_pacing::FrameDeadline::new(frame_start,render.frame_interval_ms);
        let commit_wake_after = presentation_pacing.commit_wake_after(
            frame_start, render.frame_interval_ms,
            !matches!(composition, damage_runtime::Composition::Idle),
        );
        let visible = |pid| wins.iter().any(|w| w.pid == pid && w.alive && !w.minimized && w.ws == current_ws);
        if let Err(error)=frame_runtime::wait_for_frame(deadline,commit_wake_after,visible,server,clients,&mut reload_requested,&mut notify_requests,&mut performance,transfers){
            libdunit::println(&alloc::format!("gui_server: FAIL frame wait {}",error));libdunit::exit(1);
        }
        performance.frame();
        ticks += 1;
        // Report the average whole loop period (work plus deadline wait), not
        // pure compositing cost. The PIT-backed clock has 10 ms resolution.
        // Optional PERF-SERVER diagnostics separate coarse elapsed stages.
        if ticks == 20 {
            let mut st = libdunit::SystemStats::default();
            if libdunit::get_system_stats(&mut st) >= 0 {
                prev_uptime = st.uptime_ticks;
                prev_tick = ticks;
            }
        } else if ticks == 80 && prev_tick != 0 {
            let mut st = libdunit::SystemStats::default();
            if libdunit::get_system_stats(&mut st) >= 0 {
                let win = (ticks - prev_tick) as u64;
                let dt = st.uptime_ticks.saturating_sub(prev_uptime); // 100 Hz PIT
                let ms = (dt * 10 / win).min(999);
                let mut line = *b"gui_server: frame ~000 ms";
                line[19] = b'0' + ((ms / 100) % 10) as u8;
                line[20] = b'0' + ((ms / 10) % 10) as u8;
                line[21] = b'0' + (ms % 10) as u8;
                if let Ok(s) = core::str::from_utf8(&line) {
                    libdunit::println(s);
                }
            }
        }
    }
}
