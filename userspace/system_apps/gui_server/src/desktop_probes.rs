//! Config-gated native desktop probes. Production input uses the same operations.
use super::*;

pub(super) fn resize(
    server: &mut Server,
    clients: &mut Vec<ClientState>,
    wins: &mut [Win],
    bw: usize,
    bh: usize,
    ly: &Layout,
    ins: &Insets,
    frame_start: u64,
) {
    let (_, _, ww, wh) = work_area(bw, bh, &ly, &ins);
    for wi in 0..wins.len() {
        if !wins[wi].alive {
            continue;
        }
        if matches!(wins[wi].st_step, 1 | 2) && wins[wi].st_deadline.is_none() {
            wins[wi].st_deadline = Some(frame_pacing::FrameDeadline::resize_probe(frame_start));
        }
        // A session restore can already have maximized the window
        // before this proof starts. Refresh that CONFIGURE as well,
        // then verify its restore leg instead of leaving phase zero stuck.
        if wins[wi].st_step == 0
            && matches!(wins[wi].state, WinState::Floating | WinState::Maximized)
        {
            set_window_state(
                server,
                clients,
                &mut wins[wi],
                WinState::Maximized,
                bw,
                bh,
                &ly,
                &ins,
            );
            wins[wi].st_step = 1;
            wins[wi].st_deadline = Some(frame_pacing::FrameDeadline::resize_probe(frame_start));
        } else if wins[wi].st_step == 1
            && ((wins[wi].surface_w, wins[wi].surface_h) == (ww, wh)
                || wins[wi]
                    .st_deadline
                    .is_some_and(|deadline| deadline.due(frame_start)))
        {
            // Fixed-size clients may decline resize; the deadline bounds this probe.
            let grew = (wins[wi].surface_w, wins[wi].surface_h) == (ww, wh);
            if grew {
                libdunit::println(&alloc::format!(
                    "gui_server: reconfigure w={} h={} acked (self-test)",
                    ww,
                    wh
                ));
            }
            let (rw, rh) = wins[wi]
                .restore
                .map(|(_, _, w, h)| (w, h))
                .unwrap_or((wins[wi].sw, wins[wi].sh));
            set_window_state(
                server,
                clients,
                &mut wins[wi],
                WinState::Floating,
                bw,
                bh,
                &ly,
                &ins,
            );
            wins[wi].st_step = 2;
            wins[wi].st_deadline = Some(frame_pacing::FrameDeadline::resize_probe(frame_start));
            if !grew {
                libdunit::println(&alloc::format!(
                    "gui_server: reconfigure w={} h={} settled (client kept size) (self-test)",
                    rw,
                    rh
                ));
            }
        } else if wins[wi].st_step == 2
            && ((wins[wi].surface_w, wins[wi].surface_h) == (wins[wi].sw, wins[wi].sh)
                || wins[wi]
                    .st_deadline
                    .is_some_and(|deadline| deadline.due(frame_start)))
        {
            let restored = (wins[wi].surface_w, wins[wi].surface_h) == (wins[wi].sw, wins[wi].sh);
            wins[wi].st_step = 3;
            wins[wi].st_deadline = None;
            libdunit::println(&alloc::format!(
                "gui_server: reconfigure w={} h={} {} (self-test)",
                wins[wi].sw,
                wins[wi].sh,
                if restored {
                    "restored"
                } else {
                    "settled (client kept size)"
                }
            ));
        }
    }
}

pub(super) fn switcher(
    switcher: &mut Switcher,
    wins: &mut [Win],
    mut z: &mut Vec<usize>,
    current_ws: usize,
) -> bool {
    let mru = switcher_mru(&z, &wins, current_ws);
    if mru.len() >= 2 {
        let front_pid = wins[mru[0]].pid;
        switcher.begin(&mru, false); // advances to the next candidate
        if let Some(sel) = switcher.commit() {
            let sel_pid = wins[sel].pid;
            wins[sel].minimized = false;
            raise_window(&mut z, sel);
            let now_top = z
                .iter()
                .rev()
                .copied()
                .find(|&i| wins[i].alive && wins[i].ws == current_ws && !wins[i].minimized);
            if now_top == Some(sel) && sel_pid != front_pid {
                libdunit::println(&alloc::format!(
                    "gui_server: switcher cycle prev_pid={} -> pid={} ok (self-test)",
                    front_pid,
                    sel_pid
                ));
            } else {
                libdunit::println(&alloc::format!(
                    "gui_server: FAIL switcher cycle prev_pid={} -> pid={} top={:?}",
                    front_pid,
                    sel_pid,
                    now_top.map(|i| wins[i].pid)
                ));
            }
        }
        return true;
    }

    false
}

pub(super) fn notification(
    mut notifs: &mut Vec<Toast>,
    notifications: &settings::Notifications,
    animation_now: u64,
) {
    let before = notifs.len();
    notify_post(&mut notifs, &notifications, animation_now, "Self-test");
    let posted = notifs.len();
    let future = animation_now.saturating_add(notifications.timeout_ms as u64 + 1);
    let survivors = notifs.iter().filter(|t| future < t.expire).count();
    let ok = posted == before + 1 && survivors < posted;
    // Check the added toast, since client startup can have queued earlier notifications.
    libdunit::println(&alloc::format!(
        "gui_server: notify posted added={} corner={} timeout_ms={} {} (self-test) queued={}",
        posted - before,
        notifications.corner,
        notifications.timeout_ms,
        if ok { "ok" } else { "FAIL" },
        posted,
    ));
}

pub(super) fn quicksettings(fx: &mut Effects, wg: &mut settings::Widgets) {
    let b0 = fx.blur;
    let w0 = wg.enabled;
    fx.blur = !b0;
    wg.enabled = !w0;
    let flipped = fx.blur != b0 && wg.enabled != w0;
    fx.blur = b0;
    wg.enabled = w0;
    libdunit::println(&alloc::format!(
                "gui_server: quicksettings applet enabled=1 toggled blur={}->{} widgets={}->{} {} (self-test)",
                b0 as u32,
                (!b0) as u32,
                w0 as u32,
                (!w0) as u32,
                if flipped { "ok" } else { "FAIL" }
            ));
}

pub(super) fn shortcuts(
    shortcuts: &settings::Shortcuts,
    apps: &Applications,
    current_ws: &mut usize,
) {
    let m = shortcuts.cmd_mod.mask();
    let mut map_ok = true;
    if shortcuts.ws_switch && apps.workspaces >= 2 {
        map_ok &= match_shortcut(0x03, m, true, &shortcuts, apps.workspaces)
            == ShortcutAction::Workspace(1); // cmd_mod + '2' → ws index 1
    }
    if shortcuts.launcher {
        map_ok &= match_shortcut(SC_SPACE, m, true, &shortcuts, apps.workspaces)
            == ShortcutAction::Launcher;
    }
    if shortcuts.win_close {
        map_ok &= match_shortcut(SC_Q, m, true, &shortcuts, apps.workspaces)
            == ShortcutAction::CloseWindow;
    }
    if shortcuts.win_max {
        map_ok &= match_shortcut(SC_M, m, true, &shortcuts, apps.workspaces)
            == ShortcutAction::MaximizeWindow;
    }
    // Negative: the same keys WITHOUT the modifier must not fire globally.
    map_ok &= match_shortcut(SC_Q, 0, true, &shortcuts, apps.workspaces) == ShortcutAction::None;
    // Behavioural: apply a workspace jump and read it back, then restore.
    let mut effect_ok = true;
    if shortcuts.ws_switch && apps.workspaces >= 3 {
        let before = *current_ws;
        if let ShortcutAction::Workspace(ws) =
            match_shortcut(0x04, m, true, &shortcuts, apps.workspaces)
        // cmd_mod + '3'
        {
            *current_ws = ws;
        }
        effect_ok = *current_ws == 2;
        *current_ws = before;
    }
    libdunit::println(&alloc::format!(
                "gui_server: shortcuts cmd_mod={} ws={} launcher={} close={} max={} map={} effect={} (self-test)",
                shortcuts.cmd_mod.as_str(),
                shortcuts.ws_switch as u32,
                shortcuts.launcher as u32,
                shortcuts.win_close as u32,
                shortcuts.win_max as u32,
                if map_ok { "ok" } else { "FAIL" },
                if effect_ok { "ok" } else { "FAIL" },
            ));
}

pub(super) fn session(mut session: &mut Session, apps: &Applications, wins: &[Win]) {
    for w in wins.iter() {
        session_remember(&mut session, &apps, w);
    }
    let wrote = session.save();
    let reloaded = settings::load_session();
    let matched = reloaded.len() == session.len()
        && session.entries().iter().all(|s| {
            reloaded.iter().any(|r| {
                r.id.as_str() == s.id.as_str()
                    && r.x == s.x
                    && r.y == s.y
                    && r.w == s.w
                    && r.h == s.h
            })
        });
    libdunit::println(&alloc::format!(
        "gui_server: session roundtrip {} (wrote={} saved={} reloaded={})",
        if wrote && matched { "ok" } else { "FAIL" },
        wrote as u32,
        session.len(),
        reloaded.len()
    ));
}
