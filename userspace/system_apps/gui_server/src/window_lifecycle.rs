//! Client reaping, surface reconciliation and window creation.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Environment<'a> {
    pub apps: &'a Applications,
    pub policy: &'a settings::policy::PolicyConfig,
    pub layout: &'a Layout,
    pub skin: &'a dunit_style::shell::Visuals,
    pub insets: &'a Insets,
    pub screen: (usize, usize),
    pub now: u64,
}

pub(super) fn reap(
    server: &mut Server,
    clients: &mut Vec<ClientState>,
    mut wins: &mut Vec<Win>,
    mut z: &mut Vec<usize>,
    mut drag: &mut Option<Capture>,
    mut pressed_win: &mut Option<usize>,
    mut input_focus: &mut Option<usize>,
    switcher: &mut Switcher,
    transfers: &mut transfer_runtime::Runtime,
    apps: &Applications,
) {
    // Reap exited clients and release their protocol resources before creating more windows.
    {
        let mut ci = 0;
        while ci < clients.len() {
            let pid = clients[ci].pid;
            if !clients[ci].ready {
                // A client may fail before its first surface. Observe exit
                // without consuming a merely prepared/not-yet-run child.
                use libdunit::wait_set::{self, Event, Interest, EXITED};
                let mut event = [Event::default()];
                if wait_set::wait(&[Interest::process(pid, 0)], &mut event, 0) != Ok(1)
                    || event[0].events & EXITED == 0
                {
                    ci += 1;
                    continue;
                }
            }
            let mut st = libdunit::WaitStatus::empty();
            if libdunit::wait(pid, &mut st) != pid as isize {
                ci += 1; // still running (or not reapable yet)
                continue;
            }
            server.disconnect_peer(clients[ci].conn);
            if let Some(app) = apps.apps.get(clients[ci].app as usize) {
                libdunit::println(&alloc::format!(
                    "[GUI-EXIT] app={} kind={} code={}",
                    app.id,
                    st.kind,
                    st.code
                ));
            }
            transfers.disconnected(clients[ci].pid);
            clients.remove(ci);
            reap_windows_of(
                pid,
                &mut wins,
                &mut z,
                &mut drag,
                &mut pressed_win,
                &mut input_focus,
            );
            // The switcher's MRU snapshot holds `wins` indices; a mid-flight
            // reap would leave them stale, so drop any active overlay.
            switcher.cancel();
            libdunit::println(&alloc::format!(
                "gui_server: reaped client pid={} clients={} (spawn slot freed)",
                pid,
                clients.len(),
            ));
        }
    }
}

impl Environment<'_> {
    pub(super) fn create_session(
        &self,
        server: &mut Server,
        clients: &mut Vec<ClientState>,
        wins: &mut Vec<Win>,
        z: &mut Vec<usize>,
        session: &Session,
        scripts_enabled: bool,
    ) {
        let Environment {
            apps,
            policy,
            layout: ly,
            skin,
            insets: ins,
            screen: (bw, bh),
            now,
        } = *self;
        let animation_epoch = now;
        let mut offset = 0i32;
        for i in 0..clients.len() {
            // Rebuild runtime-ready clients as well as startup-presented clients after a modeset.
            if !(clients[i].presented || clients[i].ready) || clients[i].buf_ptr.is_null() {
                continue;
            }
            let sw = clients[i].surf_w as i32;
            let sh = clients[i].surf_h as i32;
            let mut cx = clients[i].slot_x as i32 + offset;
            let mut cy = clients[i].slot_y as i32 + ly.title_h + ly.border + offset;
            // Keep decorations outside the reserved shell strips.
            if cy - ly.title_h < ins.top + ly.border {
                cy = ins.top + ly.title_h + ly.border;
            }
            if cx < ins.left + ly.border {
                cx = ins.left + ly.border;
            }
            if cx + sw + ins.right + ly.border > bw as i32 {
                cx = (bw as i32 - ins.right - sw - ly.border).max(ins.left + ly.border);
            }
            if cy + sh + ins.bottom + ly.border > bh as i32 {
                cy =
                    (bh as i32 - ins.bottom - sh - ly.border).max(ins.top + ly.title_h + ly.border);
            }
            // Restore position here; the full geometry request follows window creation.
            if let Some(g) = session_lookup(&session, &apps, clients[i].app) {
                cx =
                    g.x.max(ins.left + ly.border)
                        .min((bw as i32 - ins.right - sw - ly.border).max(ins.left + ly.border));
                cy = g.y.max(ins.top + ly.title_h + ly.border).min(
                    (bh as i32 - ins.bottom - sh - ly.border).max(ins.top + ly.title_h + ly.border),
                );
            }
            z.push(wins.len());
            wins.push(Win::from_client(
                &clients[i],
                (cx, cy),
                &ly,
                animation_epoch,
                0,
            ));
            if clients[i].placement.is_none() {
                if let Some(geometry) = session_lookup(&session, &apps, clients[i].app) {
                    let win = wins.last_mut().unwrap();
                    let minimum = window_minimum(win, &skin, &policy, &apps);
                    restore_floating_geometry(
                        server,
                        clients,
                        win,
                        (geometry.x, geometry.y, geometry.w, geometry.h),
                        (bw, bh),
                        &ly,
                        &ins,
                        minimum,
                    );
                    if geometry.maximized {
                        set_window_state(
                            server,
                            clients,
                            win,
                            WinState::Maximized,
                            bw,
                            bh,
                            &ly,
                            &ins,
                        );
                    }
                }
            }
            if let Some(p) = clients[i].placement {
                let win = wins.last_mut().unwrap();
                let minimum = window_minimum(win, &skin, &policy, &apps);
                restore_floating_geometry(
                    server,
                    clients,
                    win,
                    (p.cx, p.cy, p.size.0, p.size.1),
                    (bw, bh),
                    &ly,
                    &ins,
                    minimum,
                );
                win.state = p.state;
                win.restore = p.restore;
                win.lua_floating = p.lua_floating;
                win.st_step = p.st_step;
                win.st_deadline = None;
                win.minimized = p.minimized;
                win.ws = p.ws.min(apps.workspaces.saturating_sub(1));
                if !scripts_enabled && win.state == WinState::Tiled {
                    set_window_state(server, clients, win, WinState::Floating, bw, bh, &ly, &ins);
                } else if win.state == WinState::Maximized {
                    // Refresh maximized geometry for the new display while preserving its restore snapshot.
                    set_window_state(server, clients, win, WinState::Maximized, bw, bh, &ly, &ins);
                }
            }
            clients[i].ready = true;
            clients[i].win_created = true;
            if let Some(app) = apps.apps.get(clients[i].app as usize) {
                libdunit::println(&alloc::format!("[GUI-PRESENT] app={}", app.id));
            }
            offset += skin.placement_startup_step;
        }
    }

    pub(super) fn update(
        &self,
        server: &mut Server,
        clients: &mut Vec<ClientState>,
        wins: &mut Vec<Win>,
        z: &mut Vec<usize>,
        mut session: &mut Session,
    ) {
        let Environment {
            apps,
            policy,
            layout: ly,
            skin,
            insets: ins,
            screen: (bw, bh),
            now,
        } = *self;
        let animation_now = now;
        // Mirror client backing and acknowledged dimensions; retain outstanding resize intent.
        for c in clients.iter_mut() {
            refresh_client(server, c);
        }
        for w in wins.iter_mut() {
            if !w.alive {
                continue;
            }
            if let Some(c) = clients.iter().find(|c| c.pid == w.pid) {
                w.buf_ptr = c.buf_ptr;
                w.buf_size = c.buf_size;
                w.surface_w = c.surf_w as i32;
                w.surface_h = c.surf_h as i32;
                if w.desired_size == Some((w.surface_w, w.surface_h)) {
                    w.desired_size = None;
                }
                if let Some((width, height)) = w.desired_size {
                    w.sw = width;
                    w.sh = height;
                } else {
                    w.sw = w.surface_w;
                    w.sh = w.surface_h;
                }
                w.format = c.format;
            }
        }

        for i in 0..clients.len() {
            if !clients[i].ready || clients[i].win_created || clients[i].buf_ptr.is_null() {
                continue;
            }
            let sw = clients[i].surf_w as i32;
            let sh = clients[i].surf_h as i32;
            let step = (wins.len() as i32 % skin.placement_columns) * skin.placement_step;
            let mut cx = skin.placement_origin_x + step + ly.border;
            let mut cy = ins.top + ly.title_h + ly.border + step;
            if cx < ins.left + ly.border {
                cx = ins.left + ly.border;
            }
            if cx + sw + ins.right + ly.border > bw as i32 {
                cx = (bw as i32 - ins.right - sw - ly.border).max(ins.left + ly.border);
            }
            if cy + sh + ins.bottom + ly.border > bh as i32 {
                cy =
                    (bh as i32 - ins.bottom - sh - ly.border).max(ins.top + ly.title_h + ly.border);
            }
            // Restore runtime windows through the same geometry path as startup windows.
            let remembered = session_lookup(&session, &apps, clients[i].app);
            if let Some(g) = remembered {
                cx =
                    g.x.max(ins.left + ly.border)
                        .min((bw as i32 - ins.right - sw - ly.border).max(ins.left + ly.border));
                cy = g.y.max(ins.top + ly.title_h + ly.border).min(
                    (bh as i32 - ins.bottom - sh - ly.border).max(ins.top + ly.title_h + ly.border),
                );
            }
            let wi = wins.len();
            wins.push(Win::from_client(
                &clients[i],
                (cx, cy),
                &ly,
                animation_now,
                clients[i].ws,
            ));
            clients[i].win_created = true;
            if let Some(app) = apps.apps.get(clients[i].app as usize) {
                libdunit::println(&alloc::format!("[GUI-PRESENT] app={}", app.id));
            }
            z.push(wi);
            if let Some(geometry) = remembered {
                let minimum = window_minimum(&wins[wi], &skin, &policy, &apps);
                restore_floating_geometry(
                    server,
                    clients,
                    &mut wins[wi],
                    (geometry.x, geometry.y, geometry.w, geometry.h),
                    (bw, bh),
                    &ly,
                    &ins,
                    minimum,
                );
            }
            // Persist the new window after applying its remembered state.
            if remembered.map_or(false, |g| g.maximized) {
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
            }
            session_remember(&mut session, &apps, &wins[wi]);
            session.persist();
        }
    }
}
