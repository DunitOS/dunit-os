//! All configured commands converge on the existing window/protocol operations.
use super::*;
pub struct Context<'a> {
    pub session: &'a mut Vec<settings::WinGeom>,
    pub policy: &'a settings::policy::PolicyConfig,
    pub server: &'a mut Server,
    pub clients: &'a mut Vec<ClientState>,
    pub wins: &'a mut Vec<Win>,
    pub z: &'a mut Vec<usize>,
    pub apps: &'a Applications,
    pub shell: &'a shell_runtime::Shell,
    pub effects: &'a mut window_effects::Effects,
    pub back: &'a [u32],
    pub bw: usize,
    pub bh: usize,
    pub now: u64,
    pub duration: u32,
    pub theme: &'a Theme,
    pub ly: &'a Layout,
    pub fx: &'a Effects,
    pub font: Option<&'a Font>,
    pub ws: &'a mut usize,
    pub ws_tick: &'a mut u64,
    pub menu: &'a mut bool,
    pub menu_sel: &'a mut usize,
    pub entries: &'a mut Vec<usize>,
    pub next_id: &'a mut u32,
    pub notifs: &'a mut Vec<Toast>,
    pub notifications: &'a settings::Notifications,
    pub lua_requests: &'a mut lua_policy::Requests,
}
pub fn run(action: settings::policy::Action, c: &mut Context) {
    use settings::policy::Action;
    let focus =
        c.z.iter()
            .rev()
            .copied()
            .find(|&i| c.wins[i].alive && !c.wins[i].minimized && c.wins[i].ws == *c.ws);
    let save = matches!(&action, Action::Close | Action::Maximize);
    match action {
        Action::Launcher => {
            *c.menu = true;
            *c.menu_sel = 0;
            *c.entries = c.apps.launcher.clone();
        }
        Action::Workspace(ws) => {
            if *c.ws != ws {
                *c.ws_tick = c.now;
                *c.ws = ws;
            }
        }
        Action::Launch(id) => {
            if let Some(app) = c.apps.apps.iter().position(|a| a.id == id) {
                try_launch(
                    c.server,
                    c.clients,
                    c.apps,
                    app,
                    *c.ws,
                    c.next_id,
                    c.ly.max_windows,
                    c.notifs,
                    c.notifications,
                    c.now,
                );
            }
        }
        Action::Retile => {
            if let Some(i) = focus {
                c.wins[i].lua_floating = false;
                c.lua_requests.reset();
            }
        }
        Action::Maximize => {
            if let Some(i) = focus {
                toggle_maximize(
                    c.server,
                    c.clients,
                    &mut c.wins[i],
                    c.bw,
                    c.bh,
                    c.ly,
                    &c.shell.insets,
                );
            }
        }
        Action::Close | Action::Minimize => {
            if let Some(i) = focus {
                let w = &mut c.wins[i];
                let target = c.shell.app_target(w.app as usize, c.apps).unwrap_or((
                    c.bw as i32 / 2,
                    c.bh as i32 - 1,
                    1,
                    1,
                ));
                let style = window_effects::PaintStyle {
                    theme: c.theme,
                    layout: c.ly,
                    effects: c.fx,
                    font: c.font,
                    apps: c.apps,
                };
                if action == Action::Close {
                    c.effects.start(
                        w,
                        c.back,
                        c.bw,
                        c.bh,
                        w.outer(),
                        c.now,
                        c.duration,
                        animation::Effect::Fade,
                        animation::Easing::ease_out(),
                        true,
                        &style,
                    );
                    w.alive = false;
                    send_input(c.clients, w.pid, IN_QUIT, 0, 0, 0);
                } else {
                    c.effects
                        .minimize(w, c.back, c.bw, c.bh, target, c.now, c.duration, &style);
                    if let Some(name) = c
                        .policy
                        .for_app(
                            c.apps
                                .apps
                                .get(w.app as usize)
                                .map(|a| a.id.as_str())
                                .unwrap_or(""),
                        )
                        .animation
                    {
                        let (effect, easing) = window_effects::preset(&name, w.outer(), target);
                        c.effects
                            .configure(w.pid, effect, c.duration, easing, c.now);
                    }
                    w.minimized = true;
                    libdunit::println(&alloc::format!("gui_server: minimized pid={}", w.pid));
                }
            }
        }
    }
    if save {
        if let Some(i) = focus {
            session_remember(c.session, c.apps, &c.wins[i]);
            settings::save_session(c.session);
        }
    }
}
