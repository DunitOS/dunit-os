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
use protocol_runtime::*;
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
mod desktop_actions;

use dwm_settings as settings;
use settings::{Applications, Edge, Effects, Layout, Theme};

use dunit_text::Font;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    // No unwinding in userspace: report and exit non-zero.
    libdunit::println("gui_server: PANIC");
    libdunit::exit(101)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    libdunit::println("gui_server: start");

    // 1) The M2 protocol reference server links and runs here (allocator from
    //    libdunit, no_std+alloc core). Instantiate it and admit one client so a
    //    genuine object is allocated — this exercises the crate end to end.
    let mut server = Server::new();
    let conn = match server.connect() {
        Some(conn) => {
            libdunit::println("gui_server: protocol server up (client admitted)");
            conn
        }
        None => {
            libdunit::println("gui_server: FAIL protocol server rejected first client");
            libdunit::exit(1);
        }
    };

    // 2) Acquire the exclusive display-master capability (kernel syscall 53).
    //    A single GUI server owns the display; a second acquire would fail.
    let display = libdunit::handle_display_acquire();
    if display > 0 {
        libdunit::println("gui_server: display master acquired");
    } else {
        libdunit::println("gui_server: display master unavailable");
    }

    // 3) Acquire the exclusive input-master capability (kernel syscall 55).
    //    The compositor is the sole reader of keyboard/mouse input; it fans
    //    events out to clients over the protocol. A second acquire would fail.
    let input = libdunit::handle_input_acquire();
    if input > 0 {
        libdunit::println("gui_server: input master acquired");
    } else {
        libdunit::println("gui_server: input master unavailable");
    }

    // 4) Shared-buffer capability smoke (M3 item 1): allocate a zero-copy shared
    //    frame buffer, map it writable via the handle's WRITE right, round-trip a
    //    byte pattern through the aliased physical frames, then release. This
    //    exercises the syscall path that clients will use to hand pixel buffers
    //    to the compositor.
    let buf = libdunit::handle_create_shared(4096);
    if buf > 0 {
        let mapped = libdunit::handle_map(buf as u32, 0, 4096);
        if mapped > 0 {
            let ptr = mapped as usize as *mut u8;
            let mut ok = true;
            unsafe {
                for i in 0..4096usize {
                    core::ptr::write_volatile(ptr.add(i), (i & 0xff) as u8);
                }
                for i in 0..4096usize {
                    if core::ptr::read_volatile(ptr.add(i)) != (i & 0xff) as u8 {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                libdunit::println("gui_server: shared buffer round-trip OK");
            } else {
                libdunit::println("gui_server: FAIL shared buffer mismatch");
            }
        } else {
            libdunit::println("gui_server: FAIL shared buffer map");
        }
        libdunit::shared_buffer::release(buf as u32, if mapped > 0 { mapped as usize } else { 0 }, 4096);
    } else {
        libdunit::println("gui_server: FAIL shared buffer create");
    }

    // 5) Cross-process capability transfer (M3 item 1; groundwork for item 4's
    //    untrusted clients): create a shared buffer, write a marker, spawn an
    //    untrusted child, transfer a handle to it, and prove both sides see the
    //    same physical frames (bidirectional zero-copy across a process bound).
    let xbuf = libdunit::handle_create_shared(4096);
    if xbuf > 0 {
        let mapped = libdunit::handle_map(xbuf as u32, 0, 4096);
        if mapped > 0 {
            let ptr = mapped as usize as *mut u8;
            unsafe {
                core::ptr::write_volatile(ptr, 0xA5);
                core::ptr::write_volatile(ptr.add(1), 0x5A);
            }
            let peer = libdunit::spawn("gui_shbuf_peer");
            if peer > 0 {
                // Transfer a duplicate so we keep our own handle + mapping.
                let dup = libdunit::handle_dup(
                    xbuf as u32,
                    libdunit::RIGHT_READ
                        | libdunit::RIGHT_WRITE
                        | libdunit::RIGHT_MAP
                        | libdunit::RIGHT_TRANSFER,
                );
                let child_handle = if dup > 0 {
                    libdunit::handle_transfer(dup as u32, peer as u32)
                } else {
                    -1
                };
                if child_handle > 0 {
                    let mut msg = [0u8; 8];
                    msg[..4].copy_from_slice(&libdunit::get_pid().to_le_bytes());
                    msg[4..].copy_from_slice(&(child_handle as u32).to_le_bytes());
                    libdunit::ipc_send(peer as u32, &msg);
                    let mut ack = [0u8; 4];
                    if libdunit::ipc_recv_blocking(&mut ack, 0) == 4 && &ack == b"done" {
                        let seen = unsafe {
                            core::ptr::read_volatile(ptr.add(2)) == 0xC3
                                && core::ptr::read_volatile(ptr.add(3)) == 0x3C
                        };
                        if seen {
                            libdunit::println("gui_server: cross-process shared buffer OK");
                        } else {
                            libdunit::println("gui_server: FAIL child writes not visible");
                        }
                    } else {
                        libdunit::println("gui_server: FAIL child ack");
                    }
                } else {
                    libdunit::println("gui_server: FAIL capability transfer");
                }
                // Reap the child so it does not linger as a zombie.
                let mut status = libdunit::WaitStatus::empty();
                for _ in 0..50 {
                    if libdunit::wait(peer as u32, &mut status) == peer as isize {
                        break;
                    }
                    libdunit::sleep_ms(10);
                }
            } else {
                libdunit::println("gui_server: FAIL peer spawn");
            }
        }
        libdunit::shared_buffer::release(xbuf as u32, if mapped > 0 { mapped as usize } else { 0 }, 4096);
    }

    // 6) Framebuffer output path (M3 item 3, first slice): as the display master,
    //    blit a small pixel block into the system framebuffer via syscall 57. The
    //    kernel gates fb_present on the display-master owner, so a non-compositor
    //    process is denied (see gui_shbuf_peer). This is the mechanism the software
    //    compositor will drive to present composited surfaces.
    let mut block = [0u8; 16 * 16 * 4];
    for px in block.chunks_exact_mut(4) {
        // Solid opaque blue (little-endian XRGB/BGRA: B,G,R,A).
        px[0] = 0xC0;
        px[1] = 0x40;
        px[2] = 0x20;
        px[3] = 0xff;
    }
    if libdunit::fb_present(&block, 16, 16, 0, 0) == 0 {
        libdunit::println("gui_server: framebuffer present OK");
    } else {
        libdunit::println("gui_server: FAIL framebuffer present");
    }

    // 7) Software compositor cycle (M3 item 3): drive the gui_protocol_v1
    //    reference server with REAL wire packets through one client's full
    //    map-and-present lifecycle, back the surface with a REAL kernel shared
    //    buffer, and composite its pixels to the framebuffer.
    //
    //    `Server` is a pure protocol state machine — it never touches pixels and
    //    exposes no surface geometry — so the compositor keeps its own
    //    buffer-id -> shared-frame mapping and its own placement policy, feeds
    //    the client's requests through `deliver`, and on the committed frame
    //    blits the mapped pixels to the framebuffer via `fb_present`. This is the
    //    exact mechanism the untrusted cross-process clients (item 4) will drive;
    //    here the request stream is synthesized in-process to keep it serially
    //    verifiable without a second protocol-speaking process.
    if run_compositor_cycle(&mut server, conn) {
        libdunit::println("gui_server: compositor cycle OK");
    } else {
        libdunit::println("gui_server: FAIL compositor cycle");
    }

    // 8) Cross-process compositing (M3 item 4): serve TWO real, untrusted
    //    client ELFs concurrently over IPC. Each renders into its own shared
    //    buffer, transfers the buffer capability to us, and drives its surface
    //    through the gui-v1 wire protocol (client→compositor messages carry a
    //    4-byte client-id envelope so we route them to the right connection).
    //    We map each transferred buffer read-only, run its requests through a
    //    per-client Server connection, and composite both surfaces to the
    //    framebuffer at separate slots. Neither client holds display/input.
    // serve_two_clients() emits "served two untrusted clients OK" itself (right
    // before it holds the composited frame on screen); only report failure here.
    if !serve_two_clients() {
        libdunit::println("gui_server: FAIL serving clients");
    }

    libdunit::println("gui_server: OK");
    libdunit::exit(0)
}

/// Opcode of an outbound protocol packet (offset 8, little-endian u16).
/// Reserved screen margins (px) that the shell strips subtract from the usable
/// desktop, one accumulator per edge. `reserved_insets` fills it from the
/// configured panel/tray/dock edges, and every geometry
/// computation (work area, spawn/drag clamps, dock strip origin) reads from here
/// so "what real estate is free" has a single source of truth.
#[derive(Clone, Copy)]
struct Insets {
    top: i32,
    bottom: i32,
    left: i32,
    right: i32,
}

/// Add `size` px to the margin on `edge`.
fn add_edge(ins: &mut Insets, edge: Edge, size: i32) {
    match edge {
        Edge::Top => ins.top += size,
        Edge::Bottom => ins.bottom += size,
        Edge::Left => ins.left += size,
        Edge::Right => ins.right += size,
    }
}

/// Resolve the configured shell layout into reserved per-edge margins:
/// - the panel reserves `ly.panel_h` on `panel_edge`;
/// - the dock reserves its thickness and outer margin on `ly.dock_edge`;
/// - the tray reserves `tray_size` on `tray_edge`, but ONLY when it does not
///   share the panel's edge — a same-edge tray rides *inside* the panel strip
///   (the classic top-panel readout) and reserves nothing extra.
///
/// Margins accumulate, so the default (panel=top, tray=top, dock=left) yields
/// `{top: panel_h, left: dock_w, bottom: 0, right: 0}` — byte-identical to the
/// pre-Phase-5 fixed layout.


/// The screen rect `(x, y, w, h)` of a shell strip `thickness` px thick on `edge`,
/// offset `off` px inward from that edge. Horizontal strips (top/bottom) span the
/// full width and own the corners; vertical strips (left/right) span only the gap
/// *between* the top/bottom insets, so a left/right strip never overlaps a
/// top/bottom one at a corner. `off` lets several strips stack on one edge (e.g.
/// panel outermost at `off=0`, dock innermost at `off = ins.left - dock_w`).
fn strip_rect(edge: Edge, thickness: i32, off: i32, ins: &Insets, bw: usize, bh: usize) -> (i32, i32, i32, i32) {
    let bw = bw as i32;
    let bh = bh as i32;
    match edge {
        Edge::Top => (0, off, bw, thickness),
        Edge::Bottom => (0, bh - off - thickness, bw, thickness),
        Edge::Left => (off, ins.top, thickness, (bh - ins.top - ins.bottom).max(1)),
        Edge::Right => (bw - off - thickness, ins.top, thickness, (bh - ins.top - ins.bottom).max(1)),
    }
}

/// The maximize target rect `(cx, cy, w, h)` = the framebuffer minus the reserved
/// shell margins (`reserved_insets`) and the window's own title bar + border. One
/// helper so the interactive chip, the drag-restore path and the headless
/// self-test all compute the same geometry, and so it honours whatever edges the
/// panel/tray are configured on.
fn work_area(bw: usize, bh: usize, ly: &Layout, ins: &Insets) -> (i32, i32, i32, i32) {
    let x = ins.left + ly.border;
    let y = ins.top + ly.title_h + ly.border;
    let w = (bw as i32 - ins.left - ins.right - 2 * ly.border).max(1);
    let h = (bh as i32 - ins.top - ins.bottom - ly.title_h - 2 * ly.border).max(1);
    (x, y, w, h)
}

/// Move `win` into `new_state`, driving the client re-import needed for the new
/// size. This is the single window-state transition used by the title-bar chip,
/// the macOS-style drag-to-restore, and the headless self-test. Leaving `Floating`
/// snapshots the current `(cx,cy,sw,sh)` into `restore`; returning to `Floating`
/// replays that snapshot. The *position* is applied here immediately (so the
/// decoration follows at once); the *size* only actually changes once the owning
/// client re-imports at the CONFIGURE'd geometry (mirrored back by the per-frame
/// Win<->ClientState sync), so a client that ignores CONFIGURE keeps its size while
/// its position still updates — harmless. Reapplying Maximized refreshes the
/// work area after a display change; other unchanged states are a no-op.
fn set_window_state(
    server: &mut Server,
    clients: &[ClientState],
    win: &mut Win,
    new_state: WinState,
    bw: usize,
    bh: usize,
    ly: &Layout,
    ins: &Insets,
) {
    // STATE_ACTIVATED — the surface stays focused across the reconfigure.
    const STATE_ACTIVATED: u32 = 1;
    if win.state == new_state && new_state != WinState::Maximized {
        return;
    }
    // Snapshot floating geometry once, when first leaving the floating state.
    if win.state == WinState::Floating {
        win.restore = Some((win.cx, win.cy, win.sw, win.sh));
    }
    let target = match new_state {
        WinState::Tiled => None,
        WinState::Maximized => Some(work_area(bw, bh, ly, ins)),
        WinState::Floating => win.restore.take(),
    };
    win.motion = None;
    win.desired_size=None;
    win.state = new_state;
    if let Some((tx, ty, tw, th)) = target {
        win.cx = tx;
        win.cy = ty;
        reconfigure_client(server, clients, win.pid, tw as u32, th as u32, STATE_ACTIVATED);
    }
}

/// Toggle the title-bar maximize chip: `Maximized` <-> `Floating`.
fn toggle_maximize(
    server: &mut Server,
    clients: &[ClientState],
    win: &mut Win,
    bw: usize,
    bh: usize,
    ly: &Layout,
    ins: &Insets,
) {
    let next = if win.state == WinState::Maximized {
        WinState::Floating
    } else {
        WinState::Maximized
    };
    set_window_state(server, clients, win, next, bw, bh, ly, ins);
}

// ---------------------------------------------------------------------------
// Window/session persistence (M4). Geometry is remembered per APP id (not per
// pid, which is ephemeral), so reopening an app restores where its window last
// sat. The store is `settings::SESSION_PATH`, pre-created writable in the kernel
// VFS; it is RAM-only until DunitFS v2, then the same path survives reboots.
// ---------------------------------------------------------------------------

/// The registered app id for a window (its join key into the session store), or
/// `None` when the window's app is unregistered / id-less (nothing to remember).
fn win_app_id<'a>(apps: &'a Applications, app: u8) -> Option<&'a str> {
    let id = apps.apps.get(app as usize)?.id.as_str();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// Upsert a window's FLOATING geometry into the in-RAM session mirror, keyed by
/// its app id. A maximized window records its pre-maximize floating rect (from
/// `restore`) plus the `maximized` flag, so a later restore reopens it maximized
/// yet with a sane un-maximize size. No-op for id-less windows.
fn session_remember(session: &mut Vec<settings::WinGeom>, apps: &Applications, w: &Win) {
    let Some(id) = win_app_id(apps, w.app) else {
        return;
    };
    let (gx, gy, gw, gh) = match w.restore {
        Some((rx, ry, rw, rh)) if w.state != WinState::Floating => (rx, ry, rw, rh),
        _ => (w.cx, w.cy, w.sw, w.sh),
    };
    let maximized = w.state == WinState::Maximized;
    let ge = settings::WinGeom::new(id, gx, gy, gw, gh, maximized);
    if let Some(e) = session.iter_mut().find(|e| e.id.as_str() == id) {
        *e = ge;
    } else if session.len() < settings::MAX_SESSION {
        session.push(ge);
    }
}

/// Find a remembered geometry for the app `app` (registry index) in the mirror.
fn session_lookup(session: &[settings::WinGeom], apps: &Applications, app: u8) -> Option<settings::WinGeom> {
    let id = win_app_id(apps, app)?;
    session.iter().find(|e| e.id.as_str() == id).copied()
}

// Hardware make-codes the switcher watches. The trigger key (Tab) and the cancel
// key (Esc) are keyboard-protocol invariants, not desktop policy — only which
// MODIFIER arms the switcher is config (`[shortcuts] switch_mod`).
const SC_TAB: u8 = 0x0F;
const SC_ESC: u8 = 0x01;
// Make-codes the global desktop shortcuts watch. Like Tab/Esc above these are
// keyboard-protocol invariants (the physical key positions), NOT desktop policy —
// only WHICH MODIFIER arms them and WHETHER each action is enabled is config
// (`[shortcuts] cmd_mod` + the per-action bools). The digit row 1..9/0 is the
// PC set-1 make sequence 0x02..0x0B.
const SC_SPACE: u8 = 0x39;
const SC_Q: u8 = 0x10;
const SC_M: u8 = 0x32;
const SC_UP: u8 = 0x48;
const SC_DOWN: u8 = 0x50;
const SC_ENTER: u8 = 0x1C;

/// Map a top-row digit make-code to its value 1..=9 (0 maps to 10, so it can
/// index workspace 10 when there are that many). Non-digit scancodes yield None.
fn digit_from_scancode(sc: u8) -> Option<u32> {
    match sc {
        0x02..=0x0A => Some((sc - 0x01) as u32), // '1'..'9'
        0x0B => Some(10),                        // '0' → the 10th workspace
        _ => None,
    }
}

/// A global desktop shortcut resolved from a key event. The DECISION (key+mods →
/// action, gated by `[shortcuts]`) is pure and lives here so it can be unit-proven
/// headless; the EFFECT (mutating compositor state) stays in the key-drain loop,
/// reusing the exact same paths the mouse chips drive (single source of truth).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ShortcutAction {
    None,
    /// Jump to this 0-based virtual workspace.
    Workspace(usize),
    /// Open the application launcher menu.
    Launcher,
    /// Close the focused window.
    CloseWindow,
    /// Toggle maximize on the focused window.
    MaximizeWindow,
}

/// Resolve a key press against the config-driven global-shortcut policy. Returns
/// the action a matching `cmd_mod`+key chord maps to, or `None`. A bare key (no
/// `cmd_mod` held) never fires a global action — it falls through to the focused
/// client — so app typing is never stolen. Pure: no compositor state touched,
/// which is exactly what lets the self-test prove the mapping without a keyboard.
fn match_shortcut(scancode: u8, mods: u8, pressed: bool, sc: &settings::Shortcuts, ws_count: usize) -> ShortcutAction {
    if !pressed || (mods & sc.cmd_mod.mask()) == 0 {
        return ShortcutAction::None;
    }
    if sc.ws_switch {
        if let Some(n) = digit_from_scancode(scancode) {
            if n >= 1 && (n as usize) <= ws_count {
                return ShortcutAction::Workspace(n as usize - 1);
            }
        }
    }
    if sc.launcher && scancode == SC_SPACE {
        return ShortcutAction::Launcher;
    }
    if sc.win_close && scancode == SC_Q {
        return ShortcutAction::CloseWindow;
    }
    if sc.win_max && scancode == SC_M {
        return ShortcutAction::MaximizeWindow;
    }
    ShortcutAction::None
}

/// Alt/Super-Tab window switcher (M4, gated by `[shortcuts]`). While armed it
/// holds a frozen most-recently-used snapshot of the candidate windows and the
/// highlighted index; releasing the arming modifier commits the highlight (the
/// compositor then raises + un-minimizes that window). Tab advances, Shift-Tab
/// steps back, Esc cancels — all UI invariants.
struct Switcher {
    active: bool,
    order: Vec<usize>,
    idx: usize,
}

impl Switcher {
    fn new() -> Self {
        Switcher { active: false, order: Vec::new(), idx: 0 }
    }

    /// Arm over a fresh MRU snapshot. The current front window is index 0, so the
    /// first Tab moves to the next candidate (classic Alt-Tab "previous window").
    fn begin(&mut self, mru: &[usize], backward: bool) {
        self.order.clear();
        self.order.extend_from_slice(mru);
        self.active = !self.order.is_empty();
        self.idx = 0;
        if self.order.len() >= 2 {
            self.advance(backward);
        }
    }

    fn advance(&mut self, backward: bool) {
        let n = self.order.len();
        if n == 0 {
            return;
        }
        self.idx = if backward { (self.idx + n - 1) % n } else { (self.idx + 1) % n };
    }

    fn selected(&self) -> Option<usize> {
        self.order.get(self.idx).copied()
    }

    fn cancel(&mut self) {
        self.active = false;
    }

    /// Close the overlay and yield the highlighted window index to raise.
    fn commit(&mut self) -> Option<usize> {
        let sel = self.selected();
        self.active = false;
        sel
    }
}

/// Candidate windows for the switcher in most-recently-used order (topmost of the
/// z-order first). Only live windows on the active workspace are offered; a
/// minimized one is still listed — committing to it un-minimizes it.
fn switcher_mru(z: &[usize], wins: &[Win], ws: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for &i in z.iter().rev() {
        if wins.get(i).map_or(false, |w| w.alive && w.ws == ws) {
            out.push(i);
        }
    }
    out
}

/// Move `wi` to the top of the z-order (mirrors the click-to-raise path).
fn raise_window(z: &mut Vec<usize>, wi: usize) {
    z.retain(|&i| i != wi);
    z.push(wi);
}

/// Repair the three `Option<wins index>` cursors after the `Win` at `wi` was
/// removed from `wins`. An index equal to the removed slot is cleared; any index
/// above it shifts down by one to track the `Vec::remove` compaction. `drag`
/// carries a `(win, off_x, off_y)` tuple; the other two are bare indices.
fn adjust_indices_after_removal(
    drag: &mut Option<(usize, i32, i32)>,
    pressed_win: &mut Option<usize>,
    input_focus: &mut Option<usize>,
    wi: usize,
) {
    match *drag {
        Some((i, ..)) if i == wi => *drag = None,
        Some((i, ox, oy)) if i > wi => *drag = Some((i - 1, ox, oy)),
        _ => {}
    }
    for opt in [pressed_win, input_focus] {
        match *opt {
            Some(i) if i == wi => *opt = None,
            Some(i) if i > wi => *opt = Some(i - 1),
            _ => {}
        }
    }
}

/// Remove every `Win` owned by `pid` and repair ALL stored `wins` indices — the
/// z-order (`z`), the drag capture, and the pressed/hover cursors. Iterates the
/// victims high→low so an earlier `Vec::remove` never shifts an index still to be
/// visited. Called once a client process has been reaped, so leaving its window
/// model behind (as the old close path did) would both leak and desync indices.
fn reap_windows_of(
    pid: u32,
    wins: &mut Vec<Win>,
    z: &mut Vec<usize>,
    drag: &mut Option<(usize, i32, i32)>,
    pressed_win: &mut Option<usize>,
    input_focus: &mut Option<usize>,
) {
    let mut victims: Vec<usize> = (0..wins.len()).filter(|&i| wins[i].pid == pid).collect();
    victims.sort_unstable();
    for &wi in victims.iter().rev() {
        wins.remove(wi);
        z.retain(|&i| i != wi);
        for i in z.iter_mut() {
            if *i > wi {
                *i -= 1;
            }
        }
        adjust_indices_after_removal(drag, pressed_win, input_focus, wi);
    }
}

/// A transient desktop notification (M4, `[notifications]`). `born`/`expire` are
/// monotonic milliseconds; the compositor prunes a toast once `expire` passes.
/// Purely runtime UI feedback — the policy (on/off, timeout, corner) lives in the
/// config, never a constant.
struct Toast {
    text: alloc::string::String,
    born: u64,
    expire: u64,
}

/// Post a notification toast, gated by `[notifications] enabled` (single source of
/// truth — the flag is the resolved config's, never a hardcoded default). The
/// per-toast lifetime comes from `timeout_ms` (clamped 500..30000 at parse time). The queue is bounded so a burst of launches can't
/// grow it without limit; the oldest toasts are dropped first.
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

// ===========================================================================
// M4 userspace DWM — interactive desktop session
// ===========================================================================
//
// A persistent compositor loop that owns a full-screen back buffer. Each client
// surface is decorated with a draggable title bar; the real mouse (syscall 60,
// `get_mouse_state`) drives focus/raise/drag/close. Every tick the whole desktop
// is recomposited and presented, so it stays correct as windows move and as the
// terminal underneath (when launched via `exec gui_server`) would otherwise show
// through. Client *content* is still static here — reacting to input inside a
// window (button hover/press) is the next slice.

// Window decoration geometry (title-bar height, border) now lives in
// `settings::Layout`, loaded from the `[layout]` TOML table at desktop start.
// Each `Win` carries its own `title_h`/`border` so its methods stay parameter-free.

/// Fill an axis-aligned rectangle in the back buffer, clipped to its bounds.
/// Human-readable window title for a window's registry index (`Win.app`), looked
/// up in the live application registry. An unknown index (0xFF, e.g. a client
/// spawned for an app not in the registry) falls back to a generic "Window".
fn win_title<'a>(apps: &'a Applications, app: u8) -> &'a str {
    apps.apps.get(app as usize).map(|a| a.name.as_str()).unwrap_or("Window")
}


/// Layout state of a top-level window. `Floating` is the free-form default every
/// client is created in; `Maximized` fills the work area (screen minus the
/// reserved panel + dock). Modelled as an enum, not a `maximized: bool`, so the
/// same state machine (`set_window_state`) extends to `Fullscreen` / tiled layouts
/// later without touching every call site — a window is always in exactly one
/// state, and `Win::restore` holds the floating geometry to return to. This also
/// keeps "maximize" (a window *state*) cleanly separate from "resize" (the
/// protocol CONFIGURE mechanism that *any* state change drives).
#[derive(Clone, Copy, PartialEq, Eq)]
enum WinState {
    Tiled,
    Floating,
    Maximized,
}

/// Window policy retained across display-session re-entry. Client buffers stay
/// owned by ClientState; this records only the compositor's placement decisions.
#[derive(Clone, Copy)]
struct Placement {
    cx: i32,
    cy: i32,
    state: WinState,
    restore: Option<(i32, i32, i32, i32)>,
    lua_floating: bool,
    st_step: u8,
    minimized: bool,
    ws: usize,
}

/// Per-window compositor state, derived from a `ClientState` but with a live
/// content origin the user can drag around.
#[derive(Clone, Copy)]
struct Motion { from: (i32, i32), to: (i32, i32), start: u64, duration_ms: u32, ease_out: bool }

#[derive(Clone)]
struct Win {
    motion: Option<Motion>,
    pid: u32,
    lua_floating: bool,
    cx: i32,
    cy: i32,
    sw: i32,
    sh: i32,
    surface_w: i32,
    surface_h: i32,
    desired_size: Option<(i32,i32)>,
    buf_ptr: *const u8,
    buf_size: usize,
    alive: bool,
    /// Decoration geometry copied from the resolved `[layout]` config so each
    /// window's hit-tests and framing stay self-contained.
    title_h: i32,
    border: i32,
    traffic_lights: bool,
    /// Index into the live application registry (0xFF = unknown) — drives the
    /// dock running marker and the window title (see `win_title`).
    app: u8,
    opacity: u8,
    rule_applied: bool,
    /// Workspace (0-based) this window belongs to; only the active workspace's
    /// windows are drawn and receive input.
    ws: usize,
    /// Monotonic milliseconds when this window appeared — drives its open fade.
    born: u64,
    /// Minimized windows are not composited and receive no input; a click on the
    /// app's dock icon (task switcher) restores and raises them.
    minimized: bool,
    /// Current layout state (see `WinState`). Driven by the title-bar maximize
    /// chip and by dragging a tiled window's title bar (macOS-style restore).
    state: WinState,
    /// Floating geometry `(cx, cy, sw, sh)` snapshotted when the window leaves the
    /// floating state, so returning to `Floating` restores the exact pre-maximize
    /// position and size. `None` while the window is floating.
    restore: Option<(i32, i32, i32, i32)>,
    /// Self-test choreography step (config-gated `[startup] self_test`, test.toml
    /// only): 0 = not yet maximized, 1 = maximized (awaiting restore), 2 = restored.
    /// Lets the headless harness drive a full maximize->restore round trip without a
    /// mouse. Always 0 on the real desktop (`self_test` is false in `default.toml`).
    st_step: u8,
    /// Desktop tick when this window entered `st_step == 1` (maximize pushed).
    /// The restore leg normally waits for the client to re-import the work-area
    /// buffer (`sw >= ww`); a client that DECLINES the server-pushed resize (a
    /// fixed-size dialog like gui_settings never grows) would otherwise stall the
    /// self-test forever, so after `ST_SETTLE_TICKS` we advance anyway — the
    /// compositor must never deadlock on a client's right to keep its own size.
    st_since: u32,
    /// Pixel format the owning client imported its buffer with (mirrored from
    /// `ClientState.format` in the per-frame sync). `FORMAT_ARGB8888` selects the
    /// per-pixel src-over blit in `blit_surface` so a translucent client (e.g. a
    /// terminal with `bg_alpha < 255`) shows the desktop through its background;
    /// `FORMAT_XRGB8888` keeps the opaque fast-path copy.
    format: u32,
}

impl Win {
    /// Full outer rect (border + title bar + content) in screen space.
    fn outer(&self) -> (i32, i32, i32, i32) {
        let x = self.cx - self.border;
        let y = self.cy - self.title_h - self.border;
        let w = self.sw + 2 * self.border;
        let h = self.sh + self.title_h + 2 * self.border;
        (x, y, w, h)
    }
    fn contains(&self, mx: i32, my: i32) -> bool {
        let (x, y, w, h) = self.outer();
        mx >= x && mx < x + w && my >= y && my < y + h
    }
    fn content_position(&self,x:i32,y:i32)->(i32,i32){
        (((x-self.cx)as i64*self.surface_w as i64/self.sw.max(1)as i64)as i32,
         ((y-self.cy)as i64*self.surface_h as i64/self.sh.max(1)as i64)as i32)
    }
    /// The client content region (below the title bar).
    fn in_content(&self, mx: i32, my: i32) -> bool {
        mx >= self.cx && mx < self.cx + self.sw && my >= self.cy && my < self.cy + self.sh
    }
    fn in_title(&self, mx: i32, my: i32) -> bool {
        mx >= self.cx && mx < self.cx + self.sw && my >= self.cy - self.title_h && my < self.cy
    }
    /// Role 0=close, 1=minimize, 2=maximize; one rect for paint and input.
    fn control_rect(&self, role: i32) -> (i32, i32, i32, i32) {
        let sz = (self.title_h - 12).max(4);
        let x = if self.traffic_lights { self.cx + 8 + role * (sz + 6) }
            else { self.cx + self.sw - (role + 1) * sz - 6 - role * 4 };
        (x, self.cy - self.title_h + (self.title_h - sz) / 2, sz, sz)
    }
    fn in_close(&self, mx: i32, my: i32) -> bool { in_rect(self.control_rect(0), mx, my) }
    fn in_min(&self, mx: i32, my: i32) -> bool { in_rect(self.control_rect(1), mx, my) }
    fn in_max(&self, mx: i32, my: i32) -> bool { in_rect(self.control_rect(2), mx, my) }
    /// True when the window is not in its free-form floating state (currently only
    /// `Maximized`): it occupies a compositor-computed region, so a title-bar drag
    /// restores it to `restore` before following the pointer (macOS-style).
    fn is_tiled(&self) -> bool {
        self.state != WinState::Floating
    }
}
/// Persistent interactive compositor. Owns a full-screen back buffer, decorates
/// each client surface, and lets the real mouse focus/raise/drag/close windows.
/// Drain any queued client protocol messages (non-blocking) and advance each
/// client toward `ready`. Called every desktop tick so runtime-spawned clients
/// complete their handshake while the compositor keeps rendering. Bounded per
/// tick so a chatty client cannot starve the frame. Routing is by the
/// kernel-authenticated sender pid, so clients stay isolated.
fn pump_clients(
    server: &mut Server,
    clients: &mut Vec<ClientState>,
    reload: &mut bool,
    notify_out: &mut Vec<alloc::string::String>,
) {
    let mut rx = [0u8; 256];
    let mut sender: u32 = 0;
    for _ in 0..64 {
        let n = libdunit::ipc_recv_from(&mut rx, &mut sender);
        if n <= 0 {
            break; // EAGAIN (queue empty) or error — nothing more this tick
        }
        let n = n as usize;
        let idx = match clients.iter().position(|c| c.pid == sender) {
            Some(i) => i,
            None => continue, // message from an unknown pid — ignore
        };
        handle_client_payload(server, &mut clients[idx], &rx[..n], reload, notify_out);
        for (fc, fp) in &server.composite() {
            let status = if fp.len() >= 52 {
                u32::from_le_bytes([fp[48], fp[49], fp[50], fp[51]])
            } else {
                0
            };
            for c in clients.iter_mut() {
                if c.conn != *fc {
                    continue;
                }
                libdunit::ipc_send(c.pid, fp);
                if status == 0 && !c.buf_ptr.is_null() && c.surf_w > 0 && c.surf_h > 0 {
                    c.ready = true;
                }
            }
        }
    }
}

// --- Desktop wallpaper (ported from the legacy Stack A renderer) ---------
// The backdrop image lives in the (read-only) asset VFS as a 1600x900 24-bit
// BMP. It is loaded and pre-scaled to the framebuffer once at session start
// into an XRGB8888 buffer the size of `back`, so compositing the desktop each
// frame is a single `copy_from_slice` (no per-pixel BMP sampling in the hot
// loop). A missing/garbage file yields `None` and the gradient fallback stays.
// The path is no longer hardcoded here: it comes from `[desktop] wallpaper` in
// the config (baseline `/assets/wallpapers/wallpaper.bmp`).
const WALLPAPER_WIDTH: usize = 1920;
const WALLPAPER_HEIGHT: usize = 1080;
const WALLPAPER_OFFSET: usize = 54;
const WALLPAPER_STRIDE: usize = WALLPAPER_WIDTH * 3;

/// Slurp a whole (binary) VFS file into a byte vector. Unlike
/// `settings::read_file` this imposes no UTF-8 requirement and a larger cap, so
/// it can carry the multi-megabyte wallpaper. `None` on open error.
fn read_binary(path: &str, cap: usize) -> Option<Vec<u8>> {
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
fn validate_wallpaper_bmp(data: &[u8]) -> bool {
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
fn load_wallpaper(path: &str, bw: usize, bh: usize, background: u32) -> Option<Vec<u32>> {
    if path.ends_with(".svg") {
        let mut image = image_result(path, dunit_image::load(path, bw as u32, bh as u32))?;
        for pixel in &mut image.pixels { *pixel = blend(background, *pixel); }
        libdunit::println(&alloc::format!("gui_server: SVG wallpaper rendered {}x{}", bw, bh));
        return Some(image.pixels);
    }
    let data = read_binary(path, 8 * 1024 * 1024)?;
    if !validate_wallpaper_bmp(&data) {
        return None;
    }
    let mut out: Vec<u32> = Vec::new();
    out.resize(bw * bh, 0xFF00_0000);
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
fn resolve_app_icon(icon: &str, theme: &mut Option<dunit_image::IconTheme>, side: u32) -> Option<dunit_image::Image> {
    if icon.is_empty() { return None; }
    let result = if icon.starts_with('/') { dunit_image::load(icon, side, side) }
        else { theme.as_mut()?.icon(icon, side) };
    image_result(icon, result)
}

fn load_icon_theme(name: &str) -> Option<dunit_image::IconTheme> {
    match dunit_image::IconTheme::open(name) {
        Ok(theme) => Some(theme),
        Err(error) => {
            libdunit::println(&alloc::format!("gui_server: icon theme rejected {} {:?}", name, error));
            None
        }
    }
}

fn image_result(path: &str, result: Result<dunit_image::Image, dunit_image::Error>) -> Option<dunit_image::Image> {
    match result {
        Ok(image) => Some(image),
        Err(dunit_image::Error::Missing) => None,
        Err(error) => {
            libdunit::println(&alloc::format!("gui_server: image rejected {} {:?}", path, error));
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
fn blit_icon(
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
fn unpremul(asum: u64, rsum: u64, gsum: u64, bsum: u64, weight: u64) -> u32 {
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
fn sample_bilinear(src: &[u32], sw: usize, sh: usize, rx: usize, ry: usize, dw: usize, dh: usize) -> u32 {
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
fn sample_area(src: &[u32], sw: usize, sh: usize, rx: usize, ry: usize, dw: usize, dh: usize) -> u32 {
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
enum WidgetKind {
    Clock,
    Monitor,
}

/// One resolved desktop widget: its kind plus the placement/accent resolved from
/// its per-widget file (falling back to the `[widgets] corner` default / theme).
#[derive(Clone, Copy)]
struct DeskWidget {
    kind: WidgetKind,
    corner: i32,
    accent: u32,
    clock: settings::WidgetCfg,
}

/// Resolve the config-driven desktop-widget set. Each widget kind the compositor
/// implements is included ONLY when its `widgets/<name>.toml` exists AND is
/// enabled — so a widget is removed by deleting its file or setting
/// `enabled = false` (the Phase 8 removal switch; single source of truth, no
/// `[widgets] clock/monitor` bool). Per-widget `corner < 0` inherits
/// `default_corner` (`[widgets] corner`); `accent == 0` inherits the theme accent
/// at draw time. Called once at bring-up and again on each live config reload —
/// never per frame, so the per-widget files are not re-read every tick.
fn resolve_desk_widgets(default_corner: i32) -> Vec<DeskWidget> {
    // The compositor's registry of IMPLEMENTED widget kinds. This is renderer
    // implementation (each kind has bespoke draw code below), not desktop policy:
    // policy — which of these are actually shown — lives in the per-widget files.
    let known: [(&str, WidgetKind); 2] =
        [("clock", WidgetKind::Clock), ("monitor", WidgetKind::Monitor)];
    let mut v: Vec<DeskWidget> = Vec::new();
    for (name, kind) in known {
        let wc = settings::WidgetCfg::load(name);
        if wc.from_file && wc.enabled {
            let corner = if wc.corner >= 0 { wc.corner.min(3) } else { default_corner };
            v.push(DeskWidget { kind, corner, accent: wc.accent, clock: wc });
        }
    }
    v
}

/// Apply the configured `[display]` resolution via the kernel display backend.
/// `0/0` (baseline) keeps the current/boot resolution. A fixed backend (Limine/
/// GOP) reports EOPNOTSUPP (-95) — the desktop simply stays at the boot mode.
/// This is the compositor's mechanism→policy seam: the KERNEL owns the mode-set,
/// the CONFIG owns which mode (CLAUDE.md §6-10).
fn apply_display_mode(d: &settings::Display) {
    if d.width == 0 || d.height == 0 {
        return;
    }
    let r = libdunit::set_video_mode(d.width, d.height);
    if r == 0 {
        libdunit::println(&alloc::format!(
            "gui_server: [display] resolution set to {}x{}",
            d.width, d.height
        ));
    } else if r == -95 {
        libdunit::println("gui_server: [display] fixed backend — boot resolution kept");
    } else {
        libdunit::println(&alloc::format!(
            "gui_server: [display] set_video_mode {}x{} rejected ({})",
            d.width, d.height, r
        ));
    }
}

/// Pointer edges and client capture survive framebuffer/session recreation.
#[derive(Default)]
struct PointerSession {
    left: bool,
    right: bool,
    captured_pid: Option<u32>,
}

/// Run one interactive desktop session. Returns `true` when the session asked to
/// be RE-ENTERED (a live `[display]` change: the caller loops and this function's
/// head re-applies the new mode, re-queries the framebuffer and rebuilds every
/// buffer at the new size — cleaner than mutating bw/bh across the live loop).
/// Returns `false` (or diverges via the compositor loop) otherwise.
fn run_desktop_session(
    server: &mut Server,
    clients: &mut Vec<ClientState>,
    pending_notifies: &mut Vec<alloc::string::String>,
    first_baseline: Option<settings::Display>,
    pending_reload: bool,
    pointer: &mut PointerSession,
) -> bool {
    // Load config FIRST: `[display]` must program the video mode BEFORE we query
    // the framebuffer, so bw/bh and every buffer derived from them size to the
    // chosen resolution from the start (no mid-loop buffer surgery).
    // Slice 9: the palette is data. Overlay /system/share/dwm/default.toml on the
    // Green Tea baseline; a missing/garbage file keeps the baseline (last-known-good).
    // Slice C: `theme`/`ly`/`fx` are `mut` because a gui_settings "reload" signal
    // re-reads the config live and re-seeds them (see the reload block in `loop`).
    let cfg = settings::load_config();
    // Boot-time schema/shape gate (mirrors the live-reload path): a config from a
    // FUTURE schema, or one that parsed to an incoherent shape, is `!valid`. Rather
    // than half-apply an unknown layout we fall back WHOLESALE to the built-in
    // baseline — the same last-known-good discipline the reload block uses, applied
    // to the very first load. A well-formed current/older file passes through as-is
    // (older ones are already forward-migrated inside `load_config`).
    let mut cfg = if cfg.valid {
        cfg
    } else {
        libdunit::println("gui_server: boot config invalid (schema/shape) — using baseline");
        settings::load_system_config()
    };

    // Config-driven runtime resolution: apply BEFORE `get_framebuffer` so the
    // query below returns the chosen mode's geometry. `applied_display` is the
    // baseline the live-reload block diffs against to detect a GUI resolution edit.
    //
    // On the FIRST session entry we apply (and baseline against) `first_baseline` —
    // the display mode captured at boot, BEFORE any client could run. A settings
    // client's resolution self-test can present during bring-up (serve_two_clients
    // / resize_one_client) and rewrite `[display]` + ping reload THERE; that ping is
    // carried over as `pending_reload`. Baselining against the boot mode (not the
    // already-rewritten config file) means the carried-over reload is diffed as a
    // real change and drives the SAME re-entry a purely-runtime edit would — so the
    // GUI<->[display] round trip is proven deterministically, independent of exactly
    // when the settings client happened to present. Re-entries pass `None` and apply
    // whatever the (now authoritative) config requests.
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
    // The application registry (id/name/exec/icon/label), the dock/launcher/
    // startup pin-lists and the workspace count are DATA, owned here so the dock,
    // launcher menu, taskbar, workspace switcher and window titles all render off
    // one live model. A malformed config never lands: `load_config` keeps the
    // last-known-good baseline (an invalid file yields the baseline registry).
    let mut shell = match shell_runtime::Shell::prepare(cfg.shell.clone(), &cfg.apps, bw, bh) {
        Ok(shell) => shell,
        Err(error)=>{
            libdunit::println(&alloc::format!("gui_server: boot shell rejected {}; recovering packaged desktop",error));
            cfg=settings::load_document_from("/assets/dwm/default.toml").and_then(|text|settings::validate_document(&text)).unwrap_or_else(|_|settings::Config::defaults());
            match shell_runtime::Shell::prepare(cfg.shell.clone(),&cfg.apps,bw,bh){Ok(shell)=>shell,Err(error)=>{libdunit::println(&alloc::format!("gui_server: packaged shell unavailable {}",error));return false;}}
        }
    };
    let mut theme=cfg.settings.theme;let mut ly=cfg.settings.layout;
    let mut policy = cfg.policy;
    let mut apps: Applications = cfg.apps;
    if cfg.settings.from_file {
        libdunit::println(&alloc::format!("gui_server: settings loaded from {}",settings::active_config_path()));
    } else {
        libdunit::println("gui_server: settings default (no config file)");
    }
    // Serial diagnostic: the RESOLVED registry sizes, so a config→behavior change
    // is observable headless without a screenshot. Flip a count in the TOML and
    // this line moves — proof the desktop is config-driven, not baked into Rust.
    libdunit::println(&alloc::format!(
        "gui_server: config apps={} dock={} launcher={} startup={} ws={}",
        apps.apps.len(),
        apps.dock.len(),
        apps.launcher.len(),
        apps.startup.len(),
        apps.workspaces
    ));

    // Schema-version handling (M4): the single predicate `settings::version_supported`
    // decides which document versions this build accepts, in the config layer — the
    // compositor only reports the RESOLVED version and runs a boundary self-check so
    // the acceptance is observable headless. The guard proves the predicate rejects a
    // future schema (SCHEMA_VERSION+1) and the non-schema 0, and accepts the current
    // one — the same rule that made a bad-version boot fall back to baseline above.
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

    // Shell-strip placement (Phase 5): the panel and the system tray each dock to
    // a configurable screen edge (`[panel] pos` / `[tray] pos`), and the tray has
    // its own thickness (`[tray] size`). These are `mut` because the live-reload
    // block re-seeds them, exactly like theme/layout. `reserved_insets` turns them
    // (plus the configured dock) into the per-edge margins every geometry
    // computation below subtracts, so maximize/spawn/drag all honor the edges.
    let mut panel_edge: Edge = cfg.settings.panel.pos;
    let tray_edge: Edge = cfg.settings.tray.pos;
    let tray_size: i32 = cfg.settings.tray.size;
    // Serial diagnostic: the resolved edges, so a `[panel]`/`[tray] pos` flip is
    // observable headless. Flip the TOML and this line moves — proof the shell
    // layout is config-driven, not baked into Rust.
    libdunit::println(&alloc::format!(
        "gui_server: panel edge={} tray edge={} tray_size={}",
        panel_edge.as_str(),
        tray_edge.as_str(),
        tray_size
    ));
    let mut ins = shell.insets;

    // Config-driven desktop-widget set (Phase 8): each widget is present only when
    // its widgets/<name>.toml exists AND is enabled, so removing a widget = delete
    // its file (or `enabled = false`). `[widgets] enabled` is the master switch;
    // `[widgets] corner` the default corner. The render block below iterates THIS
    // set, not hardcoded per-part bools — the per-widget files are the single
    // source of truth. Rebuilt on live reload; the marker traces the resolved
    // decision so a config→behavior change is observable headless.
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

    // Launcher icons, one slot per REGISTRY entry (indexed by app id → the same
    // index the dock/launcher pin-lists resolve to). The shared loader resolves
    // configured manifests, archive indices and aliases, or flat legacy themes.
    // An app selects a semantic key, upstream name, archive-relative SVG path,
    // or absolute VFS image. Missing assets use that app's label initial.
    let mut app_icons: Vec<Option<dunit_image::Image>> = Vec::with_capacity(apps.apps.len());
    let mut icon_count = 0usize;
    {
        let mut icon_theme = load_icon_theme(cfg.settings.desktop.icon_theme.as_str());
        for entry in apps.apps.iter() {
            let icon = resolve_app_icon(entry.icon.as_str(), &mut icon_theme, (ly.dock_w + 6).max(32) as u32);
            if icon.is_some() {
                icon_count += 1;
            }
            app_icons.push(icon);
        }
    }
    if icon_count > 0 {
        libdunit::println(&alloc::format!("gui_server: icon assets loaded theme={} count={} raster={}px",
            cfg.settings.desktop.icon_theme.as_str(), icon_count, (ly.dock_w + 6).max(32)));
    } else {
        libdunit::println("gui_server: launcher icons absent — initials fallback");
    }

    // Brand logo for the app-menu wordmark: a 32x32 straight-alpha RGBA with a
    // circular mask baked in (path from `[desktop] logo`). `None` when the asset
    // is missing/mis-sized → the wordmark draws its flat accent mark instead.
    let mut logo = image_result(cfg.settings.desktop.logo.as_str(),
        dunit_image::load(cfg.settings.desktop.logo.as_str(), 32, 32));
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

    // Window/session restore (M4): the remembered per-app geometry, loaded once
    // from the writable session store. Gated by `[session] restore`; when off we
    // start with an empty mirror so nothing is restored *or* rewritten. The marker
    // traces the resolved policy + how many windows were remembered, so the
    // save→restore round-trip is observable headless.
    let mut session: Vec<settings::WinGeom> = if cfg.settings.session.restore {
        settings::load_session()
    } else {
        Vec::new()
    };
    libdunit::println(&alloc::format!(
        "gui_server: session restore={} entries={}",
        cfg.settings.session.restore as u32,
        session.len()
    ));

    // Build the window model from presented clients, tiling them if their slot
    // origins collide, and keeping the title bar on-screen.
    let animation_epoch = animation_time_ms().unwrap_or(0);
    let mut wins: Vec<Win> = Vec::new();
    let mut z: Vec<usize> = Vec::new();
    let mut offset = 0i32;
    for i in 0..clients.len() {
        // Runtime clients become ready in pump_clients, whereas presented is
        // the startup isolation proof. Both own live surfaces. Rebuild both on
        // a display change, or runtime windows disappear behind win_created.
        if !(clients[i].presented || clients[i].ready) || clients[i].buf_ptr.is_null() {
            continue;
        }
        let sw = clients[i].surf_w as i32;
        let sh = clients[i].surf_h as i32;
        let mut cx = clients[i].slot_x as i32 + offset;
        let mut cy = clients[i].slot_y as i32 + ly.title_h + ly.border + offset;
        // Keep the whole window (title bar included) clear of the reserved panel/
        // tray strips on every edge (Phase 5 `reserved_insets`), not just a fixed
        // top panel + left dock.
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
            cy = (bh as i32 - ins.bottom - sh - ly.border).max(ins.top + ly.title_h + ly.border);
        }
        // Session restore: if this app has a remembered position, reopen it there
        // (clamped back on-screen), so windows return to where the user left them.
        // Size stays client-owned here (a floating resize would need a CONFIGURE
        // round-trip); position is the immediately-safe part restored at boot.
        if let Some(g) = session_lookup(&session, &apps, clients[i].app) {
            cx = g
                .x
                .max(ins.left + ly.border)
                .min((bw as i32 - ins.right - sw - ly.border).max(ins.left + ly.border));
            cy = g
                .y
                .max(ins.top + ly.title_h + ly.border)
                .min((bh as i32 - ins.bottom - sh - ly.border).max(ins.top + ly.title_h + ly.border));
        }
        z.push(wins.len());
        wins.push(Win {
            pid: clients[i].pid,
            cx,
            cy,
            sw,
            sh,
            surface_w: sw,
            surface_h: sh,
            desired_size: None,
            buf_ptr: clients[i].buf_ptr,
            buf_size: clients[i].buf_size,
            alive: true,
            traffic_lights: ly.traffic_lights,
            title_h: ly.title_h,
            border: ly.border,
            app: clients[i].app,
            opacity: 255,
            rule_applied: false,
            ws: 0,
            born: animation_epoch,
            minimized: false,
            motion: None,
            state: WinState::Floating,
            lua_floating: false,
            restore: None,
            st_step: 0,
            st_since: 0,
            format: clients[i].format,
        });
        if let Some(p) = clients[i].placement {
            let win = wins.last_mut().unwrap();
            win.cx = p.cx.clamp(ins.left + ly.border,
                (bw as i32 - ins.right - ly.border - win.sw).max(ins.left + ly.border));
            win.cy = p.cy.clamp(ins.top + ly.title_h + ly.border,
                (bh as i32 - ins.bottom - ly.border - win.sh).max(ins.top + ly.title_h + ly.border));
            win.state = p.state;
            win.restore = p.restore;
            win.lua_floating = p.lua_floating;
            win.st_step = p.st_step;
            win.st_since = 0;
            win.minimized = p.minimized;
            win.ws = p.ws.min(apps.workspaces.saturating_sub(1));
            if !cfg.settings.scripts.enabled && win.state == WinState::Tiled {
                set_window_state(server, clients, win, WinState::Floating, bw, bh, &ly, &ins);
            } else if win.state == WinState::Maximized {
                // The old display's work area no longer describes maximized
                // geometry. Refresh through the existing CONFIGURE path while
                // keeping the original floating restore snapshot.
                set_window_state(server, clients, win, WinState::Maximized, bw, bh, &ly, &ins);
            }
        }
        clients[i].ready = true;
        clients[i].win_created = true;
        offset += 32;
    }

    // Config-gated session round-trip proof (`test.toml` `[startup] self_test`).
    // The harness has no mouse to drag/close windows, so exercise the persistence
    // MECHANISM directly: remember every initial window, write the store, read it
    // back through the real VFS file, and confirm the geometry survived. This is
    // the same `session_remember` + `save_session` + `load_session` path the live
    // desktop drives on window create/move/close. Never runs on the real desktop.
    if apps.self_test {
        for w in wins.iter() {
            session_remember(&mut session, &apps, w);
        }
        let wrote = settings::save_session(&session);
        let reloaded = settings::load_session();
        let matched = reloaded.len() == session.len()
            && session.iter().all(|s| {
                reloaded
                    .iter()
                    .any(|r| r.id.as_str() == s.id.as_str() && r.x == s.x && r.y == s.y && r.w == s.w && r.h == s.h)
            });
        libdunit::println(&alloc::format!(
            "gui_server: session roundtrip {} (wrote={} saved={} reloaded={})",
            if wrote && matched { "ok" } else { "FAIL" },
            wrote as u32,
            session.len(),
            reloaded.len()
        ));
    }
    // An EMPTY window set is a first-class state now: a clean desktop boots into
    // a usable but window-less session (panel/dock/launcher/wallpaper/widgets are
    // live and can spawn apps on demand). We no longer early-return here — the
    // compositor loop below runs regardless so the desktop is interactive even
    // with zero clients. The old `if wins.is_empty() { return; }` gate defeated
    // the clean-boot goal (`[startup] entries = []`).
    // Windows present at session start (the smoke's two gui_clients). The
    // keyboard-ready marker waits until a *runtime*-pumped window appears and
    // takes focus — i.e. the startup gui_terminal — so headless keystroke
    // injection lands in the terminal (topmost) rather than a static client.
    let initial_wins = wins.len();
    let mut prev_left = pointer.left;
    let mut prev_right = pointer.right;
    let mut drag: Option<(usize, i32, i32)> = None; // (win index, off_x, off_y)
    let mut pressed_win = pointer.captured_pid.and_then(|pid|
        wins.iter().position(|w| w.alive && w.pid == pid));
    let mut input_focus: Option<usize> = None; // window under the pointer
    let mut last_lx = i32::MIN;
    let mut last_ly = i32::MIN;
    // Bounded so a headless run (mouse never moves) still terminates and lets
    // the harness force-quit; ~60000 * 16ms ≈ 16 min of interactive use.
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
    // --- Visual-effects state (concept §5) -------------------------------
    let mut fx = cfg.settings.effects;
    // Keyboard-shortcut policy (`[shortcuts]`): whether the Alt/Super-Tab window
    // switcher is offered and which modifier arms it. `mut` so a live reload can
    // flip the switcher on/off or swap Alt<->Super without a restart (re-seeded in
    // the reload block, single source of truth — never read from a constant).
    let mut shortcuts = cfg.settings.shortcuts;
    // Live window-switcher overlay state (armed by `switch_mod`+Tab, committed on
    // modifier release). Empty/inactive until the user (or the self-test) arms it.
    let mut switcher = Switcher::new();
    // One-shot latch so the config-gated switcher self-test drives exactly one
    // cycle (test.toml `[startup] self_test`), never on the real desktop.
    let mut switcher_st_done = false;
    // Desktop widget card (concept §5): painted on the wallpaper behind windows.
    let mut wg = cfg.settings.widgets;
    // Quick-settings applet (`[quicksettings]`): a panel cell whose flyout flips the
    // LIVE effect/widget flags (`fx.blur` / `wg.enabled`) — single source of truth,
    // the flyout mutates the very fields the renderer reads, never a shadow copy.
    // `mut` so a live reload re-seeds `enabled`; the flyout closes on reload so a
    // now-disabled applet can't leave a stale card up.
    let mut quicksettings = cfg.settings.quicksettings;
    let mut qs_open = false;
    let mut active_qs_anchor=None;
    let mut qs_st_done = false;
    // Desktop notifications (`[notifications]`): a bounded queue of transient
    // launch-feedback toasts, each auto-dismissed after `timeout_ms` (→ ~16 ms
    // frames) and stacked in the configured corner. All policy is config-driven.
    let mut notifications = cfg.settings.notifications;
    let mut notifs: Vec<Toast> = Vec::new();
    let mut notify_st_done = false;
    let mut notify_expired_logged = false;
    // Flush any notifications a client posted during bring-up (before this session's
    // toast queue existed) through the SAME config-gated `notify_post` path the
    // per-tick client-notify drain and app-launch sites use — so a startup toast
    // (e.g. gui_demo's "I'm up") is honored exactly once here rather than lost to
    // the bring-up race. Policy stays the compositor's; the client only named text.
    for text in pending_notifies.drain(..) {
        notify_post(&mut notifs, &notifications, animation_epoch, &text);
        libdunit::println(&alloc::format!(
            "gui_server: client notify \"{}\" queued={} (enabled={})",
            text,
            notifs.len(),
            notifications.enabled as u32,
        ));
    }
    // One-shot flag for the global keyboard-shortcut self-test (below).
    let mut shortcut_st_done = false;
    // Config-gated client-recycle self-test (`test.toml` `[startup] self_test`).
    // Proves the "close many windows, then open new ones" bug is fixed: once all
    // startup windows have been through the maximize/restore self-test, close them
    // ALL, wait for the per-frame reaper to prune the `clients` Vec (which is what
    // frees spawn slots), then relaunch — the ceiling must be lifted. A tiny state
    // machine because close→exit→reap→relaunch spans several frames. Never runs on
    // the real desktop (`self_test` is false in `default.toml`).
    let mut recycle_phase: u8 = 0; // 0=wait 1=draining 2=done
    let mut recycle_pre: usize = 0; // clients present when we closed them
    let mut recycle_deadline: u32 = 0; // tick budget for the drain to finish
    // Durations stay in milliseconds even when rendering misses a frame.
    let mut anim_duration_ms = if fx.anim { fx.anim_ms as u32 } else { 0 };
    // Workspace-switch crossfade: timestamped on the monotonic animation clock.
    let mut ws_switch_tick = animation_epoch;
    // Launcher-menu drop-in: monotonic timestamp when the dropdown opens.
    let mut menu_anim_tick = animation_epoch;
    let mut previous_animation_time = animation_epoch;
    let mut menu_was_open = false;
    // Per-dock-icon hover progress (0..255), eased toward hovered/idle each
    // frame. Sized to the live dock pin-count (config `[dock] entries`).

    // Ping-pong scratch for the backdrop blur, sized for the largest blurred
    // region (panel / dock strip / menu) and allocated once.
    let panel_area = bw * ly.panel_h.max(0) as usize;
    let dock_area = ly.dock_w.max(0) as usize * bw.max(bh);
    let menu_area = ly.menu_w.max(0) as usize * (apps.launcher.len() * ly.menu_item_h.max(0) as usize);
    let scratch_len = panel_area.max(dock_area).max(menu_area).max(1);
    let mut blur_a: Vec<u32> = alloc::vec![0u32; scratch_len];
    let mut blur_b: Vec<u32> = alloc::vec![0u32; scratch_len];
    // Scratch for translucent (ARGB) windows: the true backing (wallpaper + lower
    // windows) behind a window's content rect, snapshotted before the opaque frame
    // gradient overwrites it, so the client blends over what's actually behind the
    // window rather than the window's own frame. Reused across frames.
    let mut win_backing: Vec<u32> = Vec::new();
    // One-shot latch: log the first per-pixel ARGB composite so the transparency
    // path is observable in the serial log (headless proof that a translucent
    // client is blended, not opaque-copied). Never fires when every client is XRGB.
    let mut argb_blit_logged = false;
    // Frame-time instrumentation so the blur cost is observable in the serial
    // log even headless: report ms/frame once warm, then periodically.
    let mut prev_uptime = 0u64;
    let mut prev_tick = 0u32;
    // Slice C: raised by a gui_settings "reload" control message (see
    // `handle_client_payload`); drained at the top of the frame to re-read the
    // config and re-seed live theme/layout/effects state.
    // Seed with any reload pinged during bring-up (a settings client that presented
    // in serve_two_clients/resize_one_client). The loop's reload block processes it
    // on the first tick, diffing the edited config against `applied_display` above.
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
    // Client-posted notification texts collected by `pump_clients` this tick
    // (NOTIFY_MAGIC), turned into config-gated toasts right after the pump. Reused
    // across frames (cleared each tick) so no per-frame allocation in the hot path.
    let mut notify_requests: Vec<alloc::string::String> = Vec::new();
    loop {
        let animation_clock = animation_time_ms();
        let animation_now = animation_clock.unwrap_or(previous_animation_time);
        // Advance any runtime-spawned clients through their protocol handshake,
        // then hand each newly-ready client a cascaded, focused window.
        notify_requests.clear();
        pump_clients(server, clients, &mut reload_requested, &mut notify_requests);

        // Live settings reload (GUI<->TOML round-trip): a gui_settings client
        // rewrote the config and pinged us. Re-read it and re-seed everything
        // derived from theme/layout/effects — plus the asset paths in [desktop]
        // (wallpaper + icon theme), so a config change swaps them live too. Never
        // fatal — load() falls back to the baseline on a bad file.
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
                shell_runtime::Shell::prepare(ncfg.shell.clone(),&ncfg.apps,bw,bh).and_then(|mut shell|{
                    shell.prepare_text(next_font.as_ref(),&ncfg.apps,&ncfg.settings.layout)?;Ok(shell)})
            }else{Err(alloc::string::String::from("invalid config"))};
            if let Err(error) = &next_shell { libdunit::println(&alloc::format!("gui_server: shell reload rejected {}",error)); }
            let mut next_icon_theme = if ncfg.valid {
                load_icon_theme(ncfg.settings.desktop.icon_theme.as_str())
            } else { None };
            if !ncfg.valid || next_icon_theme.is_none() || next_shell.is_err() {
                // Malformed/incoherent config: keep the running (last-known-good)
                // state WHOLESALE. Never partially apply a bad file — a broken
                // reload must not blank the dock or drop the registry.
                libdunit::println("gui_server: settings reload rejected (invalid config) — keeping last good");
            } else {
                // Live resolution change (GUI edited `[display]`): a runtime mode-
                // set would strand bw/bh and every buffer sized from them. Instead
                // RE-ENTER the session — the head re-applies the mode, re-queries the
                // framebuffer and rebuilds all buffers at the new size. Windows
                // survive because they are rebuilt from the persistent `clients`.
                // 0/0 means "keep current", so only an explicit new mode re-enters.
                if ncfg.settings.display != applied_display
                    && ncfg.settings.display.width != 0
                    && ncfg.settings.display.height != 0
                {
                    libdunit::println(
                        "gui_server: [display] changed on reload — re-entering session",
                    );
                    for win in wins.iter().filter(|w| w.alive) {
                        if let Some(client) = clients.iter_mut().find(|c| c.pid == win.pid) {
                            client.placement = Some(Placement { cx: win.cx, cy: win.cy,
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
                    return true;
                }
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
                theme = ncfg.settings.theme;
                ly = ncfg.settings.layout;
                fx = ncfg.settings.effects;
                shortcuts = ncfg.settings.shortcuts;
                wg = ncfg.settings.widgets;
                // Quick-settings + notification policy travel through the same
                // reload (single source of truth); close any open flyout so a
                // now-disabled applet can't leave a stale card behind.
                quicksettings = ncfg.settings.quicksettings;
                qs_open = false;
                active_qs_anchor=None;
                notifications = ncfg.settings.notifications;
                // Re-resolve the config-driven widget set so a live edit that adds/
                // removes a widgets/<name>.toml (or flips its `enabled`) takes effect.
                desk_widgets = resolve_desk_widgets(ncfg.settings.widgets.corner);
                shell = next_shell.unwrap();
                shell.describe();
                effects.clear();
                desktop_tracks=shell_motion::Tracks::default();

                hovered_component=None;
                font = next_font;
                policy = ncfg.policy;
                for win in &mut wins {win.app=apps.apps.get(win.app as usize).and_then(|old|ncfg.apps.apps.iter().position(|new|new.id==old.id)).map(|index|index as u8).unwrap_or(255);}
                for client in clients.iter_mut(){client.app=apps.apps.get(client.app as usize).and_then(|old|ncfg.apps.apps.iter().position(|new|new.id==old.id)).map(|index|index as u8).unwrap_or(255);}
                apps = ncfg.apps;
                event_engine.set_context(&shell.config,&apps);
                menu_entries = apps.launcher.clone();
                custom_menu_origin = None;
                // Re-seed the shell-strip placement + recompute the reserved
                // margins, so a live `[panel]`/`[tray] pos` edit moves the panel,
                // tray and the maximize/work area together.
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
                // The blur scratch is sized for the largest blurred region; a larger
                // panel/dock/menu after reload needs a bigger buffer (never shrink —
                // a smaller region simply uses a prefix of the existing allocation).
                let panel_area = bw * ly.panel_h.max(0) as usize;
                let dock_area = ly.dock_w.max(0) as usize * bw.max(bh);
                let menu_area =
                    ly.menu_w.max(0) as usize * (menu_entries.len() * ly.menu_item_h.max(0) as usize);
                let need = panel_area.max(dock_area).max(menu_area).max(1);
                if blur_a.len() < need {
                    blur_a.resize(need, 0);
                    blur_b.resize(need, 0);
                }
                // Re-seed the config-driven assets (wallpaper + icon theme) and the
                // registry-derived state (icons per entry, dock hover slots).
                wallpaper = load_wallpaper(ncfg.settings.desktop.wallpaper.as_str(), bw, bh, theme.desktop);

                logo=image_result(ncfg.settings.desktop.logo.as_str(),dunit_image::load(ncfg.settings.desktop.logo.as_str(),32,32));
                app_icons.clear();
                for entry in apps.apps.iter() {
                    app_icons.push(resolve_app_icon(entry.icon.as_str(), &mut next_icon_theme, (ly.dock_w + 6).max(32) as u32));
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

        // Turn this tick's client-posted notification requests (NOTIFY_MAGIC) into
        // toasts. Policy stays the compositor's: `notify_post` is a no-op when
        // `[notifications]` is disabled, and the timeout/corner come from config —
        // the client only supplied the (bounds-checked) text. Drained AFTER the
        // reload block so a just-reloaded `notifications` policy applies at once.
        for text in notify_requests.drain(..) {
            notify_post(&mut notifs, &notifications, animation_now, &text);
            libdunit::println(&alloc::format!(
                "gui_server: client notify \"{}\" queued={} (enabled={})",
                text,
                notifs.len(),
                notifications.enabled as u32,
            ));
        }

        // Per-frame client reaper. A window closed via the title-bar chip or
        // Cmd-Q sets `alive=false` and sends IN_QUIT; the client then exits. A
        // client can also exit on its own (crash / voluntary quit). Either way its
        // resources must be reclaimed, or they leak: BEFORE this the `clients` Vec
        // was never pruned, so after `max_windows` cumulative windows the spawn
        // gate (`clients.len() >= max_windows`) blocked every new window — the
        // "close a bunch of windows, then can't open new ones" bug. And the
        // protocol `Server` caps connections (MAX_CONNECTIONS), so an un-dropped
        // `ConnId` per closed window would eventually starve `server.connect()`
        // too. Poll every already-presented client with the non-blocking `wait`
        // (the kernel reaps only a truly-terminated child and returns an error for
        // a live one, so a running window is never touched; a not-yet-`ready`
        // client is skipped so a still-spawning child is never reaped early). On a
        // reap, free ALL of it: the protocol connection + its surfaces/buffers
        // (`disconnect_peer`), the compositor's shared-buffer mapping
        // (`vm_unmap` + `handle_close`), the `ClientState` (this is what lifts the spawn
        // ceiling), and the `Win` with full z/drag/focus index fixup.
        {
            let mut ci = 0;
            while ci < clients.len() {
                if !clients[ci].ready {
                    ci += 1;
                    continue;
                }
                let pid = clients[ci].pid;
                let mut st = libdunit::WaitStatus::empty();
                if libdunit::wait(pid, &mut st) != pid as isize {
                    ci += 1; // still running (or not reapable yet)
                    continue;
                }
                server.disconnect_peer(clients[ci].conn);
                if clients[ci].mapped_handle != 0 {
                    libdunit::shared_buffer::release(clients[ci].mapped_handle, clients[ci].buf_ptr as usize, clients[ci].buf_size);
                }
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
                // `ci` not advanced: the compacted next client now sits at `ci`.
            }
        }

        // Per-frame Win<->ClientState reconciliation. A client that re-imported a
        // buffer after a resize (maximize / restore / fullscreen) now carries a
        // fresh backing pointer + geometry in its `ClientState`; mirror it into the
        // owning `Win` so the compositor blits the new buffer at the new size and
        // never the freed old one. Position stays compositor-owned (set by drag or
        // by `toggle_maximize`), so only the buffer + surface size are synced here.
        for w in wins.iter_mut() {
            if !w.alive {
                continue;
            }
            if let Some(c) = clients.iter().find(|c| c.pid == w.pid) {
                w.buf_ptr = c.buf_ptr;
                w.buf_size = c.buf_size;
                w.surface_w=c.surf_w as i32;w.surface_h=c.surf_h as i32;
                if w.desired_size==Some((w.surface_w,w.surface_h)){w.desired_size=None;}
                if let Some((width,height))=w.desired_size{w.sw=width;w.sh=height;}else{w.sw=w.surface_w;w.sh=w.surface_h;}
                w.format = c.format;
            }
        }

        for i in 0..clients.len() {
            if !clients[i].ready || clients[i].win_created || clients[i].buf_ptr.is_null() {
                continue;
            }
            let sw = clients[i].surf_w as i32;
            let sh = clients[i].surf_h as i32;
            let step = (wins.len() as i32 % 6) * 40;
            let mut cx = 90 + step + ly.border;
            let mut cy = ins.top + ly.title_h + ly.border + step;
            if cx < ins.left + ly.border {
                cx = ins.left + ly.border;
            }
            if cx + sw + ins.right + ly.border > bw as i32 {
                cx = (bw as i32 - ins.right - sw - ly.border).max(ins.left + ly.border);
            }
            if cy + sh + ins.bottom + ly.border > bh as i32 {
                cy = (bh as i32 - ins.bottom - sh - ly.border).max(ins.top + ly.title_h + ly.border);
            }
            // Session restore for a runtime-launched app: reopen at the remembered
            // position (clamped on-screen). A window remembered as maximized is
            // re-maximized just after creation via the shared `set_window_state`
            // path (below), once the Win exists.
            let remembered = session_lookup(&session, &apps, clients[i].app);
            if let Some(g) = remembered {
                cx = g
                    .x
                    .max(ins.left + ly.border)
                    .min((bw as i32 - ins.right - sw - ly.border).max(ins.left + ly.border));
                cy = g
                    .y
                    .max(ins.top + ly.title_h + ly.border)
                    .min((bh as i32 - ins.bottom - sh - ly.border).max(ins.top + ly.title_h + ly.border));
            }
            let wi = wins.len();
            wins.push(Win {
                pid: clients[i].pid,
                cx,
                cy,
                sw,
                sh,
                surface_w: sw,
                surface_h: sh,
            desired_size: None,
                buf_ptr: clients[i].buf_ptr,
                buf_size: clients[i].buf_size,
                alive: true,
                traffic_lights: ly.traffic_lights,
                title_h: ly.title_h,
                border: ly.border,
                app: clients[i].app,
            opacity: 255,
            rule_applied: false,
                ws: clients[i].ws,
                born: animation_now,
                minimized: false,
                motion: None,
            state: WinState::Floating,
            lua_floating: false,
                restore: None,
                st_step: 0,
                st_since: 0,
                format: clients[i].format,
            });
            clients[i].win_created = true;
            z.push(wi);
            // Re-maximize a window that was remembered maximized (uses the same
            // CONFIGURE path the chip drives), then persist the freshly-placed
            // window so its geometry is remembered from birth.
            if remembered.map_or(false, |g| g.maximized) {
                set_window_state(server, clients, &mut wins[wi], WinState::Maximized, bw, bh, &ly, &ins);
            }
            session_remember(&mut session, &apps, &wins[wi]);
            settings::save_session(&session);
        }

        // Config-gated resize self-exercise (`test.toml` `[startup] self_test`).
        // The automated harness has no mouse, so the title-bar maximize chip and
        // the drag-to-restore gesture can't be exercised by hand. Instead drive
        // EVERY live window through the SAME `set_window_state` path those gestures
        // use — proving server-push CONFIGURE + client buffer re-import across a
        // real process boundary, in BOTH directions:
        //   step 0 -> 1: maximize on first sight (window fills the work area);
        //   step 1 -> 2: after a short settle, restore to the floating geometry.
        // Maximizing every window (not just runtime-pumped ones) makes the proof
        // independent of which client presents first in `serve_two_clients`. The
        // restore leg is what verifies the user-visible "un-maximize" actually
        // works end-to-end (the same `set_window_state(Floating)` the chip and the
        // drag-restore call). Never runs on the real desktop (`self_test` is false
        // in `default.toml`).
        if apps.self_test {
            let (_, _, ww, wh) = work_area(bw, bh, &ly, &ins);
            for wi in 0..wins.len() {
                if !wins[wi].alive {
                    continue;
                }
                if wins[wi].st_step == 0 && wins[wi].state == WinState::Floating {
                    set_window_state(server, clients, &mut wins[wi], WinState::Maximized, bw, bh, &ly, &ins);
                    wins[wi].st_step = 1;
                    wins[wi].st_since = ticks;
                    libdunit::println(&alloc::format!(
                        "gui_server: reconfigure w={} h={} acked (self-test)",
                        ww, wh
                    ));
                } else if wins[wi].st_step == 1
                    && (wins[wi].sw >= ww
                        || ticks.saturating_sub(wins[wi].st_since) >= ST_SETTLE_TICKS)
                {
                    // The client has observably re-imported the work-area-sized
                    // buffer (its synced surface reached the maximize width), so the
                    // maximize round trip is complete. Now prove the reverse leg —
                    // the user-visible un-maximize — via the SAME
                    // `set_window_state(Floating)` the chip and drag-to-restore call.
                    // A fixed-size client that never grew (declined the resize) falls
                    // through here on the ST_SETTLE_TICKS timeout so the self-test
                    // (and the recycle proof gated on it) can't stall on it.
                    let grew = wins[wi].sw >= ww;
                    let (rw, rh) = wins[wi]
                        .restore
                        .map(|(_, _, w, h)| (w, h))
                        .unwrap_or((wins[wi].sw, wins[wi].sh));
                    set_window_state(server, clients, &mut wins[wi], WinState::Floating, bw, bh, &ly, &ins);
                    wins[wi].st_step = 2;
                    libdunit::println(&alloc::format!(
                        "gui_server: reconfigure w={} h={} {} (self-test)",
                        rw, rh,
                        if grew { "restored" } else { "settled (client kept size)" }
                    ));
                }
            }
        }

        // Config-gated window-switcher proof (`test.toml` `[startup] self_test`,
        // and only when `[shortcuts] switcher` is on). The harness has no keyboard
        // to hold Alt+Tab, so drive the SAME `Switcher` state the key-drain path
        // arms: snapshot the MRU list, arm (advancing off the front window), commit
        // to the highlight, then raise it exactly as the modifier-release path does.
        // Proves the switcher selects a DIFFERENT window and that committing makes
        // it the topmost/focused one. One-shot; never runs on the real desktop.
        if apps.self_test && !switcher_st_done && shortcuts.switcher {
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
                            front_pid, sel_pid
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
                switcher_st_done = true;
            }
        }

        // Config-gated notification proof (`test.toml` `[startup] self_test`). The
        // harness has no mouse to click the dock, so post one toast through the
        // SAME `notify_post` path a launch uses, prove it enqueues, and prove the
        // auto-dismiss predicate removes it once its timeout elapses — checked
        // against the very predicate the per-frame prune runs (`now < expire`), so
        // the queue lifecycle is verified without a fragile wall-clock wait. The
        // toast stays queued and expires at its monotonic deadline (below),
        // which trips the end-to-end `notify expired` marker.
        if apps.self_test && !notify_st_done && notifications.enabled {
            let before = notifs.len();
            notify_post(&mut notifs, &notifications, animation_now, "Self-test");
            let posted = notifs.len();
            let future = animation_now.saturating_add(notifications.timeout_ms as u64 + 1);
            let survivors = notifs.iter().filter(|t| future < t.expire).count();
            let ok = posted == before + 1 && survivors < posted;
            // Report the DELTA this post added (`added`), not the absolute queue
            // size: a client (e.g. autostarted gui_demo) may have a live startup
            // toast already queued, so the absolute count is timing/config-dependent
            // while "this post enqueued exactly one" is the real invariant. `queued`
            // trails as a diagnostic. corner/timeout_ms prove the policy is config-
            // driven; "ok" folds in the enqueue-delta + auto-dismiss predicate.
            libdunit::println(&alloc::format!(
                "gui_server: notify posted added={} corner={} timeout_ms={} {} (self-test) queued={}",
                posted - before,
                notifications.corner,
                notifications.timeout_ms,
                if ok { "ok" } else { "FAIL" },
                posted,
            ));
            notify_st_done = true;
        }

        // Config-gated quick-settings proof. The applet can't be clicked headless,
        // so flip the SAME live flags its flyout toggles (`fx.blur`, `wg.enabled`)
        // and confirm each change lands on the field the renderer reads — proving
        // the applet mutates real state, not a shadow copy — then restore them so
        // the desktop's configured look is untouched. One-shot.
        if apps.self_test && !qs_st_done && quicksettings.enabled {
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
            qs_st_done = true;
        }

        // Config-gated global-shortcut proof (`test.toml` `[startup] self_test`).
        // The harness cannot hold `cmd_mod`+key, so exercise the SAME pure
        // `match_shortcut` decision the key-drain loop uses: assert every ENABLED
        // action maps from its `cmd_mod`+key chord, that a bare key (no modifier)
        // maps to nothing (app typing is never stolen), and — the behavioural
        // anchor — that APPLYING a workspace action really moves `current_ws`
        // (config→behaviour, not just parse). Launcher/close/maximize reuse the
        // exact mouse-chip effect paths, so proving their mapping proves the wiring.
        if apps.self_test && !shortcut_st_done {
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
            map_ok &= match_shortcut(SC_Q, 0, true, &shortcuts, apps.workspaces)
                == ShortcutAction::None;
            // Behavioural: apply a workspace jump and read it back, then restore.
            let mut effect_ok = true;
            if shortcuts.ws_switch && apps.workspaces >= 3 {
                let before = current_ws;
                if let ShortcutAction::Workspace(ws) =
                    match_shortcut(0x04, m, true, &shortcuts, apps.workspaces) // cmd_mod + '3'
                {
                    current_ws = ws;
                }
                effect_ok = current_ws == 2;
                current_ws = before;
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
            shortcut_st_done = true;
        }

        // Config-gated client-recycle self-test (see the state decls above). The
        // reported scheduler bug was that after closing enough windows no new one
        // could open, because the `clients` Vec was never pruned. This proves the
        // fix end-to-end across a real process boundary: close every window, let
        // the per-frame reaper drain the `clients` Vec, then relaunch and confirm
        // spawning succeeds again. Starts only after the maximize/restore self-test
        // has finished on every startup window, so it never disturbs the earlier
        // one-shot proofs.
        if apps.self_test {
            match recycle_phase {
                0 => {
                    let live: Vec<usize> =
                        (0..wins.len()).filter(|&i| wins[i].alive).collect();
                    let all_settled =
                        !live.is_empty() && live.iter().all(|&i| wins[i].st_step == 2);
                    // Wait for the ARGB steady-state composite proof (`argb blit`)
                    // to have fired first — otherwise closing every window would
                    // preempt the frame in which the translucent terminal finally
                    // composites at full reveal. Recycle is deliberately the LAST
                    // self-test, so it must not race the earlier ones.
                    if all_settled && argb_blit_logged && !clients.is_empty() {
                        recycle_pre = clients.len();
                        for &wi in &live {
                            effects.start(&wins[wi],&back,bw,bh,wins[wi].outer(),animation_now,anim_duration_ms,
                        animation::Effect::Fade,animation::Easing::ease_out(),true,
                        &window_effects::PaintStyle{theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});
                    wins[wi].alive = false;
                            send_input(wins[wi].pid, IN_QUIT, 0, 0, 0);
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
                    // The reaper prunes `clients` as each closed client exits. Once
                    // at least one slot is freed, relaunch to prove the ceiling is
                    // lifted; fill until the (now reachable) cap and report.
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
        let m = libdunit::get_mouse_state();
        let mx = m.x as i32;
        let my = m.y as i32;
        let workspace_input=desktop_tracks.sample(desktop_motion::WORKSPACE,(0,0,bw as i32,bh as i32),animation_now);
        let (wx,wy)=desktop_motion::logical_point(workspace_input.rect,mx,my,bw,bh);
        let left = m.left();
        let press = left && !prev_left;
        let release = !left && prev_left;
        let right = m.right();
        let rpress = right && !prev_right;

        // Shell-strip rects for THIS frame (recomputed cheaply so a live reload
        // that moved the panel/tray takes effect at once). The panel and the dock
        // always exist; the tray gets its OWN strip only when it does not share the
        // panel's edge (otherwise it renders inside the panel — the classic look).
        // `in_shell` marks the shell-owned region: a click/right-click there is the
        // shell's, never routed to a client window.
        let panel_rect = panel_strip(&ly, &ins, panel_edge, bw, bh);

        let raw_menu_org=custom_menu_origin.unwrap_or_else(||menu_origin(&ly,panel_edge,panel_rect,menu_entries.len()as i32));
        let menu_view=desktop_tracks.sample(desktop_motion::LAUNCHER,(raw_menu_org.0,raw_menu_org.1,ly.menu_w,(menu_entries.len().max(1)as i32)*ly.menu_item_h),animation_now);
        let menu_org=(menu_view.rect.0,menu_view.rect.1);
        let mut menu_ly=ly;menu_ly.menu_w=menu_view.rect.2;menu_ly.menu_item_h=(menu_view.rect.3/menu_entries.len().max(1)as i32).max(1);
        // Quick-settings applet cell on the panel (only when enabled); its flyout
        // flies out below/beside the panel and is NOT part of `in_shell` (a
        // transient overlay), so its clicks are caught by the `qs_open` branch
        // below before window routing — exactly like the launcher dropdown.
        let (qs_applet,qs_edge)=active_qs_anchor.or_else(||shell.first_quicksettings_anchor()).unwrap_or(((0,0,0,0),panel_edge));
        let in_shell = |x: i32, y: i32| shell.contains(x,y);

        // --- Right-button press: forward to the content window under the cursor
        // as IN_DOWN with button=1 (context-menu trigger). Panel/dock/title are
        // shell-owned and ignore the right button; a right-click there is a no-op.
        if rpress && !menu_open && !qs_open && !in_shell(mx, my) {
            let mut zi = z.len();
            while zi > 0 {
                zi -= 1;
                let wi = z[zi];
                if wins[wi].alive && wins[wi].ws == current_ws && !wins[wi].minimized && !effects.covers(wins[wi].pid,animation_now) && effects.input_geometry(&wins[wi],animation_now).in_content(wx,wy) {
                    z.retain(|&i| i != wi);
                    z.push(wi);
                    let (x,y)=effects.input_geometry(&wins[wi],animation_now).content_position(wx,wy);send_input(wins[wi].pid,IN_DOWN,x,y,1);
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
            // Quick-settings flyout is open: a click on a toggle row flips the LIVE
            // flag it represents (single source of truth — `fx.blur` / `wg.enabled`
            // are exactly the fields the renderer reads); any other click just
            // dismisses the flyout. Either way the click is consumed and the flyout
            // closes, mirroring the launcher-dropdown model.
            let flyout = qs_flyout_rect(&ly, qs_edge, qs_applet, 2);
            if in_rect(qs_row_rect(&ly, flyout, 0), mx, my) {
                fx.blur = !fx.blur;
                libdunit::println(&alloc::format!("gui_server: quicksettings blur={}", fx.blur as u32));
            } else if in_rect(qs_row_rect(&ly, flyout, 1), mx, my) {
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
                if wins[wi].in_close(wx, wy) {
                    effects.start(&wins[wi],&back,bw,bh,wins[wi].outer(),animation_now,anim_duration_ms,
                        animation::Effect::Fade,animation::Easing::ease_out(),true,
                        &window_effects::PaintStyle{theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});
                    wins[wi].alive = false;
                    send_input(wins[wi].pid, IN_QUIT, 0, 0, 0);
                    drag = None;
                    // Remember where it was before it goes away, so reopening the
                    // app restores its last geometry.
                    session_remember(&mut session, &apps, &wins[wi]);
                    settings::save_session(&session);
                } else if wins[wi].in_min(wx, wy) {
                    // Minimize: drop it from compositing/input; the dock task
                    // switcher restores it on click.
                    let target=shell.app_target(wins[wi].app as usize,&apps).unwrap_or((bw as i32/2,bh as i32-1,1,1));
                    effects.minimize(&wins[wi],&back,bw,bh,target,animation_now,anim_duration_ms,
                        &window_effects::PaintStyle{theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});
                    if let Some(name)=policy.for_app(apps.apps.get(wins[wi].app as usize).map(|a|a.id.as_str()).unwrap_or("")).animation {
                        let (effect,easing)=window_effects::preset(&name,wins[wi].outer(),target);effects.configure(wins[wi].pid,effect,anim_duration_ms,easing,animation_now);
                    }
                    wins[wi].minimized = true;
                    drag = None;
                } else if wins[wi].in_max(wx, wy) {
                    // Maximize/restore: toggle the window between the work area
                    // and its saved floating geometry via a server-push CONFIGURE
                    // (Phase 3). The owning client re-imports at the new size and
                    // the per-frame sync mirrors the fresh buffer back.
                    toggle_maximize(server, clients, &mut wins[wi], bw, bh, &ly, &ins);
                    drag = None;
                    // Persist the new maximize state (with the preserved floating
                    // rect) so a reopen restores it in the same state.
                    session_remember(&mut session, &apps, &wins[wi]);
                    settings::save_session(&session);
                } else if wins[wi].in_title(wx, wy) {
                    // Title-bar drag. macOS-style restore: grabbing a *tiled*
                    // window (e.g. maximized) first restores it to its floating
                    // size, then it follows the pointer keeping the SAME
                    // proportional grip along the now-narrower title bar — so the
                    // window "shrinks under the cursor" instead of snapping to a
                    // corner. A floating window just starts dragging from where it
                    // was grabbed. `ox`/`oy` are offsets from the window content
                    // origin to the cursor (oy is negative — the bar sits above the
                    // content), matching the drag-move math below.
                    if scripts.enabled { wins[wi].lua_floating = true; }
                    wins[wi].motion = None;
                    if wins[wi].is_tiled() {
                        let old_w = wins[wi].sw.max(1);
                        let grip_x = (wx - wins[wi].cx).clamp(0, old_w);
                        let grip_y =
                            (wy - (wins[wi].cy - wins[wi].title_h)).clamp(0, (wins[wi].title_h - 1).max(0));
                        let rest_w = wins[wi]
                            .restore
                            .map(|(_, _, w, _)| w)
                            .unwrap_or(old_w)
                            .max(1);
                        set_window_state(server, clients, &mut wins[wi], WinState::Floating, bw, bh, &ly, &ins);
                        let ox = (grip_x as i64 * rest_w as i64 / old_w as i64) as i32;
                        let oy = grip_y - wins[wi].title_h;
                        wins[wi].cx = wx - ox;
                        wins[wi].cy = wy - oy;
                        drag = Some((wi, ox, oy));
                    } else {
                        drag = Some((wi, wx - wins[wi].cx, wy - wins[wi].cy));
                    }
                } else if wins[wi].in_content(wx, wy) {
                    // Click landed on client content: forward it to the client.
                    let (x,y)=effects.input_geometry(&wins[wi],animation_now).content_position(wx,wy);send_input(wins[wi].pid,IN_DOWN,x,y,0);
                    pressed_win = Some(wi);
                }
            }
        }
        if !left {
            // Drag released: persist the window's final position so it reopens
            // where the user dropped it (save on drag-END, not every move frame).
            if let Some((wi, _, _)) = drag {
                session_remember(&mut session, &apps, &wins[wi]);
                settings::save_session(&session);
            }
            drag = None;
        }

        // --- Drag move ---
        if let Some((wi, ox, oy)) = drag {
            let mut ncx = wx - ox;
            let mut ncy = wy - oy;
            ncx = ncx
                .max(ins.left + ly.border)
                .min(bw as i32 - ins.right - wins[wi].sw - ly.border);
            ncy = ncy
                .max(ins.top + ly.title_h + ly.border)
                .min(bh as i32 - ins.bottom - wins[wi].sh - ly.border);
            wins[wi].cx = ncx;
            wins[wi].cy = ncy;
        }

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
        if cur != input_focus {
            if let Some(old) = input_focus {
                if wins[old].alive {
                    send_input(wins[old].pid, IN_LEAVE, 0, 0, 0);
                }
            }
            input_focus = cur;
            last_lx = i32::MIN; // force the next motion to be delivered
        }
        if let Some(wi) = cur {
            let (lx,ly)=effects.input_geometry(&wins[wi],animation_now).content_position(wx,wy);
            if lx != last_lx || ly != last_ly {
                send_input(wins[wi].pid, IN_MOVE, lx, ly, 0);
                last_lx = lx;
                last_ly = ly;
            }
        }

        // --- Button release: deliver UP to the window that captured the press ---
        if release {
            if let Some(wi) = pressed_win {
                if wins[wi].alive {
                    let (x,y)=effects.input_geometry(&wins[wi],animation_now).content_position(wx,wy);send_input(wins[wi].pid,IN_UP,x,y,0);
                }
            }
            pressed_win = None;
        }
        prev_left = left;
        prev_right = right;

        // The desktop session persists even with no live windows: closing the
        // last window drops back to the empty-but-usable desktop rather than
        // ending the session. (Was: `if !wins.iter().any(alive) { break; }`,
        // which killed the compositor the moment the last client closed and made
        // a clean, window-less boot impossible.) The loop is still bounded by the
        // `ticks >= 60000` headless guard at the bottom.

        let focused = z
            .iter()
            .rev()
            .copied()
            .find(|&i| wins[i].alive && wins[i].ws == current_ws && !wins[i].minimized);

        // --- Mouse wheel: forward the tick's scroll delta to the focused window
        // as IN_SCROLL (delta in `lx`). Clients that don't scroll ignore it; the
        // terminal uses it to move through its scrollback history.
        if m.wheel != 0 {
            if let Some(wi) = focused {
                send_input(wins[wi].pid, IN_SCROLL, m.wheel, 0, 0);
            }
        }

        // --- Keyboard: drain the seat and route events to the focused window ---
        // The compositor is the sole reader of the kernel key ring. Every key
        // press (make) for a non-modifier key is forwarded to the focused window
        // as an IN_KEY event carrying the full trio the kernel already cooks:
        //   lx     = scancode (sc & 0x7F)
        //   ly     = modifier mask (KEYMOD_*)
        //   button = cooked ASCII byte (0 for extended/arrow/nav keys)
        // Old clients read only the ASCII byte in `button` (unchanged); the
        // terminal decodes scancode+mods for control keys (Ctrl-C, arrows, …).
        // Modifier-only presses and key releases are dropped — their state is
        // already reflected in the `mods` field of the events we do send.
        if focused.is_some() && wins.len() > initial_wins && !input_ready_announced {
            libdunit::println("gui_server: desktop input ready");
            input_ready_announced = true;
        }
        while let Some(ev) = libdunit::get_key_event() {
            // Window switcher (M4, config-gated). The compositor owns the seat, so
            // it intercepts the switcher gesture BEFORE forwarding keys to clients:
            //   * `switch_mod`+Tab arms/advances an MRU overlay (Tab is swallowed,
            //     never reaches the focused client);
            //   * Shift adds reverse cycling; Esc cancels without switching;
            //   * releasing the arming modifier commits — raise + un-minimize the
            //     highlighted window.
            // `mods` reflects the modifier state at the event moment, so a cleared
            // arming bit on any drained event while armed means "modifier released".
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

            // --- Launcher menu keyboard navigation (modal) ---------------------
            // While the dropdown is open it CAPTURES every key: Up/Down move the
            // highlight, Enter launches the highlighted app (the SAME `try_launch`
            // path a mouse click uses), Esc closes. Swallowing all keys keeps them
            // out of the focused client so typing can't leak past the open menu.
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

            // --- Global desktop shortcuts (config-driven `cmd_mod` + key) -------
            // Intercepted before client forwarding, exactly like the switcher. The
            // DECISION is the pure `match_shortcut` (so the self-test can prove the
            // mapping); the EFFECT here reuses the same paths the mouse chips drive.
            let custom=if ev.pressed {policy.bindings.iter().find(|b|b.scancode==ev.scancode&&b.modifiers==ev.mods).map(|b|b.action.clone())}else{None};
            if let Some(action)=custom {desktop_actions::run(action,&mut desktop_actions::Context{session:&mut session,policy:&policy,server,clients,wins:&mut wins,z:&mut z,apps:&apps,shell:&shell,effects:&mut effects,back:&back,bw,bh,now:animation_now,duration:if animation_clock.is_some(){anim_duration_ms}else{0},theme:&theme,ly:&ly,fx:&fx,font:font.as_ref(),ws:&mut current_ws,ws_tick:&mut ws_switch_tick,menu:&mut menu_open,menu_sel:&mut menu_sel,entries:&mut menu_entries,next_id:&mut next_id,notifs:&mut notifs,notifications:&notifications,lua_requests:&mut lua_requests});continue;}
            let action=match match_shortcut(ev.scancode,ev.mods,ev.pressed,&shortcuts,apps.workspaces){
                ShortcutAction::Workspace(ws)=>Some(settings::policy::Action::Workspace(ws)),ShortcutAction::Launcher=>Some(settings::policy::Action::Launcher),
                ShortcutAction::CloseWindow=>Some(settings::policy::Action::Close),ShortcutAction::MaximizeWindow=>Some(settings::policy::Action::Maximize),ShortcutAction::None=>None};
            if let Some(action)=action {desktop_actions::run(action,&mut desktop_actions::Context{session:&mut session,policy:&policy,server,clients,wins:&mut wins,z:&mut z,apps:&apps,shell:&shell,effects:&mut effects,back:&back,bw,bh,now:animation_now,duration:if animation_clock.is_some(){anim_duration_ms}else{0},theme:&theme,ly:&ly,fx:&fx,font:font.as_ref(),ws:&mut current_ws,ws_tick:&mut ws_switch_tick,menu:&mut menu_open,menu_sel:&mut menu_sel,entries:&mut menu_entries,next_id:&mut next_id,notifs:&mut notifs,notifications:&notifications,lua_requests:&mut lua_requests});continue;}

            if !ev.pressed || is_modifier_scancode(ev.scancode) {
                continue;
            }
            if let Some(wi) = focused {
                send_input(
                    wins[wi].pid,
                    IN_KEY,
                    ev.scancode as i32,
                    ev.mods as i32,
                    ev.ascii as u32,
                );
            }
        }

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
                    &window_effects::PaintStyle{theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});}
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
            if press {if let Some((target,widget))=hover {let _=event_engine.submit(lua_policy::Event{name:if widget{"widget_click"}else{"panel_click"},window_id:0,target});}}
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
                                &window_effects::PaintStyle{theme:&theme,layout:&ly,effects:&fx,font:font.as_ref(),apps:&apps});
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
                lua_policy::Poll::Ready(policy) => {
                    let ids = lua_requests.ids();
                    // Reject the WHOLE result if any cell cannot contain its
                    // decorations. Never apply an early rectangle then fail late.
                    let fits = policy
                        .rects
                        .iter()
                        .all(|r| r.w > ly.border * 2 && r.h > ly.title_h + ly.border * 2);
                    if fits {
                        lua_expected.clear();
                        for (pid, rect) in ids.iter().zip(&policy.rects) {
                            if let Some(win) = wins.iter_mut().find(|w| w.pid == *pid && w.alive) {
                                if win.state == WinState::Floating {
                                    win.restore = Some((win.cx, win.cy, win.sw, win.sh));
                                }
                                lua_expected.push((win.pid, (rect.w - ly.border * 2) as u32,
                                    (rect.h - ly.title_h - ly.border * 2) as u32));
                                win.state = WinState::Tiled;
                                let target = (ins.left + rect.x + ly.border,
                                    ins.top + rect.y + ly.title_h + ly.border);
                                let duration_ms = if fx.anim { policy.duration_ms } else { 0 };
                                if duration_ms > 0 && target != (win.cx, win.cy) {
                                    win.motion = Some(Motion { from: (win.cx, win.cy), to: target,
                                        start: animation_now, duration_ms, ease_out: policy.ease_out });
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
                        anim_duration_ms = if fx.anim { policy.duration_ms } else { 0 };
                        lua_ease_out = policy.ease_out;
                        libdunit::println(&alloc::format!(
                            "gui_server: Lua policy applied windows={} duration_ms={} easing={}",
                            ids.len(),
                            policy.duration_ms,
                            if lua_ease_out { "ease_out" } else { "linear" }
                        ));
                    } else {
                        lua_failure = Some("cell smaller than window decorations");
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

        // Desktop backdrop: the wallpaper (pre-scaled) when present, else a
        // subtle vertical gradient (or a flat fill when the gradient effect is
        // off — the pre-§5 look).
        if let Some(wp) = &wallpaper {
            back.copy_from_slice(wp);
        } else if fx.gradient {
            let top = shade(theme.desktop, 12);
            let bot = shade(theme.desktop, -8);
            let span = (bh as u32 - 1).max(1);
            for y in 0..bh {
                let row = 0xFF00_0000 | lerp_color(top, bot, (y as u32 * 255) / span);
                let base = y * bw;
                for px in &mut back[base..base + bw] {
                    *px = row;
                }
            }
        } else {
            for px in back.iter_mut() {
                *px = theme.desktop;
            }
        }

        // --- Desktop widget cards (concept §5) -------------------------------
        // Translucent plasmoids on the wallpaper, behind windows. The SET is
        // config-driven (`desk_widgets`, resolved from the per-widget files):
        // each widget is present only when its file exists and is enabled, so a
        // widget is removed by deleting its file. `[widgets] enabled` is the
        // master kill switch. Widgets sharing a corner stack into one card
        // (clock above monitor), so the default (both in corner 1) reads as one
        // plasmoid; per-widget `corner` can split them across corners. Windows
        // composite on top.
        if wg.enabled && !desk_widgets.is_empty() {
            if let Some(f) = font.as_ref() {
                let mut wst = libdunit::SystemStats::default();
                let whave = libdunit::get_system_stats(&mut wst) >= 0;
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
                    let card_w = 236i32;
                    let pad = 16i32;
                    let clock_px=clock_w.map(|w|w.clock.font_px as f32).unwrap_or(ly.title_font_px as f32);
                    let clock_h=clock_w.map(|w|if w.clock.style==settings::ClockStyle::Analog{w.clock.size as i32}else{clock_px as i32+10}).unwrap_or(0);
                    let mon_h = if mon_w.is_some() { 58 } else { 0 };
                    let card_h = pad * 2 + clock_h + mon_h;
                    let margin = 24i32;
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
                    if fx.blur {
                        blur_region(&mut back, bw, bh, cx, cy, card_w, card_h, fx.blur_radius, fx.blur_iters, &mut blur_a, &mut blur_b);
                    }
                    fill_rrect_grad(&mut back, bw, bh, cx, cy, card_w, card_h, fx.corner_radius, RR_ALL, shade(theme.menu, 16), shade(theme.menu, -8), fx.menu_alpha);
                    let secs=if whave&&wst.uptime_hz>0{wst.uptime_ticks/wst.uptime_hz}else{0};
                    let mut ty = cy + pad;
                    if let Some(clock)=clock_w {
                        if clock.clock.style==settings::ClockStyle::Analog {
                            shell_runtime::analog(&mut back,bw,bh,(cx+pad,ty,card_w-2*pad,clock_h),secs,theme.panel_text,theme.launcher);
                        } else {
                        let clk = alloc::format!("{:02}:{:02}:{:02}", (secs / 3600) % 100, (secs / 60) % 60, secs % 60);
                        let tw = text_width_ttf(f, &clk, clock_px);
                        let baseline = ty + clock_px as i32 - 4;
                        draw_text_ttf(&mut back, bw, bh, f, cx + (card_w - tw) / 2, baseline, clock_px, &clk, theme.panel_text & 0x00FF_FFFF);
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
                        let line = alloc::format!("RAM {}%    {} proc", ram_pct, wst.process_running);
                        draw_text_ttf(&mut back, bw, bh, f, cx + pad, ty + px as i32, px, &line, theme.panel_text & 0x00FF_FFFF);
                        // RAM usage bar under the text row.
                        let bar_y = ty + px as i32 + 12;
                        let bar_w = card_w - 2 * pad;
                        fill_rrect(&mut back, bw, bh, cx + pad, bar_y, bar_w, 8, 4, RR_ALL, ((fx.menu_alpha.min(255) as u32) << 24) | (shade(theme.menu, -20) & 0x00FF_FFFF));
                        let fill_w = (bar_w * ram_pct.min(100) as i32) / 100;
                        if fill_w > 0 {
                            // Per-widget accent (widgets/monitor.toml `accent`) overrides
                            // the theme launcher color for the fill; 0 = inherit theme.
                            let bar_col = if mw.accent != 0 { mw.accent } else { theme.launcher };
                            fill_rrect(&mut back, bw, bh, cx + pad, bar_y, fill_w, 8, 4, RR_ALL, 0xFF00_0000 | (bar_col & 0x00FF_FFFF));
                        }
                    }
                }
            }
        }

        // Windows, bottom-to-top: soft shadow, rounded gradient frame, rounded
        // gradient title bar, then the opaque client surface. Each window fades
        // in from `born`, and the active workspace fades in after a switch —
        // folded into one alpha `a` so both read as a single grow-in.
        let duration_ms = if animation_clock.is_some() { anim_duration_ms } else { 0 };
        let workspace_view=desktop_tracks.sample(desktop_motion::WORKSPACE,(0,0,bw as i32,bh as i32),animation_now);
        let workspace_transform=desktop_motion::changed(&workspace_view,bw,bh)&&workspace_layer.begin(&back);
        let ws_fade = animation::progress(animation_now, ws_switch_tick, duration_ms, lua_ease_out);
        for &wi in z.iter() {
            let w = &wins[wi];
            if !w.alive || w.ws != current_ws || w.minimized || effects.covers(w.pid,animation_now) {
                continue;
            }
            let is_focused = focused == Some(wi);
            let a = (animation::progress(animation_now,w.born,duration_ms,lua_ease_out).min(if desktop_tracks.has(desktop_motion::WORKSPACE,lua_policy::Property::Opacity){255}else{ws_fade}) * effects.alpha(w.pid,animation_now) / 255 * w.opacity as u32 / 255) as i32;
            if a <= 0 {
                continue;
            }
            let (visual,visual_theme,visual_fx)=effects.visual(w,animation_now,&theme,&fx);
            window_painter::paint(&mut back,bw,bh,&visual,is_focused,a,&visual_theme,&ly,&visual_fx,font.as_ref(),&apps,wx,wy,&mut win_backing,&mut argb_blit_logged,&mut blur_a,&mut blur_b);
        }

        effects.prune(&wins,animation_now);
        effects.paint(&mut back,bw,bh,animation_now,current_ws);
        if workspace_transform{workspace_layer.finish(&mut back,bw,bh,&workspace_view,&mut blur_a,&mut blur_b);}
        let frame_fx=Effects{anim:fx.anim&&animation_clock.is_some(),..fx};
        shell.paint(&mut back,bw,bh,font.as_ref(),&app_icons,logo.as_ref(),&apps,&wins,focused,current_ws,mx,my,
            animation_now,&theme,&ly,&frame_fx,&mut blur_a,&mut blur_b);

        // Launcher dropdown: acrylic rounded menu with a drop-in reveal.
        if menu_open && !menu_was_open {
            menu_anim_tick = animation_now; // stamp the open so the menu drops in
        }
        if !menu_open&&menu_was_open{menu_closed_at=animation_now;}
        menu_was_open=menu_open;
        if menu_open||(menu_closed_at>0&&animation_now.saturating_sub(menu_closed_at)<duration_ms as u64)||desktop_tracks.active(desktop_motion::LAUNCHER,animation_now){
            let menu_opacity=if desktop_tracks.has(desktop_motion::LAUNCHER,lua_policy::Property::Opacity)&&(menu_open||desktop_tracks.since(desktop_motion::LAUNCHER,lua_policy::Property::Opacity,menu_closed_at)){menu_view.opacity}else if menu_open{255}else{255-animation::progress(animation_now,menu_closed_at,duration_ms,lua_ease_out)};
            let menu_blend=menu_opacity<255&&workspace_layer.begin(&back);
            let mprog = animation::progress(animation_now, menu_anim_tick, duration_ms, lua_ease_out) as i32; // 0..255
            let (mx0, my0, mw0, _) = menu_item_rect(&menu_ly, menu_org, 0);
            let full_h = menu_entries.len() as i32 * menu_ly.menu_item_h;
            let reveal_h = full_h * mprog / 255; // grow the card downward
            if reveal_h > 0 {
                if fx.blur||menu_view.blur>0 {
                    blur_region(&mut back, bw, bh, mx0, my0, mw0, reveal_h, if menu_view.blur>0{menu_view.blur}else{fx.blur_radius}, fx.blur_iters, &mut blur_a, &mut blur_b);
                }
                if menu_view.shadow>0{draw_soft_shadow(&mut back,bw,bh,mx0,my0,mw0,reveal_h,menu_view.radius.unwrap_or(fx.corner_radius),menu_view.shadow,fx.shadow_alpha);}
                fill_rrect_grad(&mut back, bw, bh, mx0, my0, mw0, reveal_h, menu_view.radius.unwrap_or(fx.corner_radius), RR_ALL, shade(theme.menu, 14), theme.menu, fx.menu_alpha);
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
                    fill_rrect(&mut back, bw, bh, ix + 3, iy + 2, iw - 6, ih - 4, 5, RR_ALL, 0xC000_0000 | (theme.menu_hover & 0x00FF_FFFF));
                }
                // Registry icon at the row's left, then the app label after it.
                let mut text_x = ix + 8;
                if let Some(icon) = app_icons.get(ri).and_then(|s| s.as_ref()) {
                    let isz = (ih - 8).clamp(8, 22);
                    let icy = iy + (ih - isz) / 2;
                    blit_icon(&mut back, bw, bh, &icon.pixels, icon.width, icon.height, ix + 6, icy, isz, isz);
                    text_x = ix + 6 + isz + 6;
                }
                let label = apps.apps.get(ri).map(|a| a.label.as_str()).unwrap_or("");
                draw_shell_text(&mut back, bw, bh, font.as_ref(), text_x, iy + (ih - ly.panel_font_px) / 2, ly.panel_font_px, 0xff000000|(menu_view.color.unwrap_or(theme.panel_text)&0x00ff_ffff), label.as_bytes());
            }
            if menu_blend{let mut opacity=desktop_tracks.sample(0,(0,0,bw as i32,bh as i32),animation_now);opacity.opacity=menu_opacity;workspace_layer.finish(&mut back,bw,bh,&opacity,&mut blur_a,&mut blur_b);}
        }

        // --- Window switcher overlay (M4) ------------------------------------
        // A centered acrylic card with one icon tile per candidate window and the
        // highlighted app's name below it. Drawn above all windows (below only the
        // cursor) while the switcher is armed; every colour comes from the resolved
        // theme, so it inherits the desktop palette rather than hardcoding one.
        if switcher.active && !switcher.order.is_empty() {
            let n = switcher.order.len() as i32;
            let tile = 64i32;
            let gap = 12i32;
            let pad = 20i32;
            let label_h = 24i32;
            let panw = n * tile + (n - 1) * gap + 2 * pad;
            let panh = tile + label_h + 2 * pad;
            let px0 = (bw as i32 - panw) / 2;
            let py0 = (bh as i32 - panh) / 2;
            draw_soft_shadow(&mut back, bw, bh, px0, py0, panw, panh, 16, 10, 90);
            fill_rrect(&mut back, bw, bh, px0, py0, panw, panh, 16, 0x0F, 0xF0000000 | (theme.menu & 0x00FF_FFFF));
            for (k, &wi) in switcher.order.iter().enumerate() {
                let tx = px0 + pad + k as i32 * (tile + gap);
                let ty = py0 + pad;
                if k == switcher.idx {
                    fill_rrect(&mut back, bw, bh, tx - 4, ty - 4, tile + 8, tile + 8, 10, 0x0F,
                        0xFF00_0000 | (theme.menu_hover & 0x00FF_FFFF));
                }
                let app = wins[wi].app as usize;
                let isz = tile - 12;
                if let Some(ic) = app_icons.get(app).and_then(|s| s.as_ref()) {
                    blit_icon(&mut back, bw, bh, &ic.pixels, ic.width, ic.height, tx + 6, ty + 6, isz, isz);
                } else {
                    let lbl = apps.apps.get(app).map(|a| a.label.as_str()).unwrap_or("?");
                    draw_shell_text(&mut back, bw, bh, font.as_ref(), tx + 8, ty+(isz-ly.panel_font_px)/2,ly.panel_font_px,theme.panel_text, lbl.as_bytes());
                }
            }
            if let Some(&wi) = switcher.order.get(switcher.idx) {
                if let Some(f) = font.as_ref() {
                    let title = win_title(&apps, wins[wi].app);
                    let fpx = 15.0f32;
                    let tw = text_width_ttf(f, title, fpx);
                    let sx = px0 + (panw - tw) / 2;
                    let baseline = py0 + pad + tile + label_h - 6;
                    draw_text_ttf(&mut back, bw, bh, f, sx, baseline, fpx, title, theme.panel_text & 0x00FF_FFFF);
                }
            }
        }

        // --- Quick-settings flyout (M4) --------------------------------------
        // A small acrylic card of live toggles that flies out from the applet cell.
        // Each row shows and flips the SAME flag the renderer reads (fx.blur /
        // wg.enabled) with an on/off pill — no shadow state. Drawn above windows,
        // below the switcher overlay/cursor. Every colour comes from the theme.
        if quicksettings.enabled && qs_open {
            let flyout = qs_flyout_rect(&ly, qs_edge, qs_applet, 2);
            let (fx0, fy0, fw0, fh0) = flyout;
            if fx.blur {
                blur_region(&mut back, bw, bh, fx0, fy0, fw0, fh0, fx.blur_radius, fx.blur_iters, &mut blur_a, &mut blur_b);
            }
            draw_soft_shadow(&mut back, bw, bh, fx0, fy0, fw0, fh0, 10, 10, 90);
            fill_rrect_grad(&mut back, bw, bh, fx0, fy0, fw0, fh0, fx.corner_radius, RR_ALL, shade(theme.menu, 14), theme.menu, fx.menu_alpha);
            let rows = [("Blur", fx.blur), ("Widgets", wg.enabled)];
            for (i, (lbl, on)) in rows.iter().enumerate() {
                let (rx, ry, rw, rh) = qs_row_rect(&ly, flyout, i as i32);
                let hover = mx >= rx && mx < rx + rw && my >= ry && my < ry + rh;
                if hover {
                    fill_rrect(&mut back, bw, bh, rx + 2, ry + 2, rw - 4, rh - 4, 5, RR_ALL, 0xC000_0000 | (theme.menu_hover & 0x00FF_FFFF));
                }
                if let Some(f) = font.as_ref() {
                    let px = ly.panel_font_px as f32;
                    let baseline = ry + rh / 2 + 5;
                    draw_text_ttf(&mut back, bw, bh, f, rx + 10, baseline, px, lbl, theme.panel_text & 0x00FF_FFFF);
                }
                // On/off pill on the row's trailing edge, knob sliding to reflect state.
                let (pw, ph) = (22i32, 12i32);
                let px0 = rx + rw - pw - 8;
                let py0 = ry + (rh - ph) / 2;
                let pill = if *on { theme.launcher } else { theme.taskbtn };
                fill_rrect(&mut back, bw, bh, px0, py0, pw, ph, ph / 2, RR_ALL, 0xFF00_0000 | (pill & 0x00FF_FFFF));
                let knob = ph - 4;
                let kx = if *on { px0 + pw - knob - 2 } else { px0 + 2 };
                fill_rrect(&mut back, bw, bh, kx, py0 + 2, knob, knob, knob / 2, RR_ALL, 0xFFFF_FFFF);
            }
        }

        // --- Notification toasts (M4) ----------------------------------------
        // Prune expired toasts (the SAME `now < expire` predicate the self-test
        // verifies), then stack the survivors in the configured corner. The
        // one-shot `notify expired` marker fires when the self-test's toast has
        // drained end-to-end, proving the auto-dismiss path — not just the queue.
        notifs.retain(|t| animation_clock.is_some() && animation_now < t.expire);
        if notify_st_done && !notify_expired_logged && notifs.is_empty() {
            libdunit::println("gui_server: notify expired (self-test)");
            notify_expired_logged = true;
        }
        if notifications.enabled && !notifs.is_empty() {
            let margin = 16i32;
            let cardh = 30i32;
            let gap = 8i32;
            let corner = notifications.corner;
            let top = corner == 0 || corner == 1;
            let leftc = corner == 0 || corner == 2;
            for (slot, t) in notifs.iter().enumerate() {
                let px = ly.title_font_px as f32;
                let tw = font
                    .as_ref()
                    .map(|f| text_width_ttf(f, &t.text, px))
                    .unwrap_or(t.text.len() as i32 * 8);
                let cardw = (tw + 30).clamp(96, bw as i32 - 2 * margin);
                // Slide-in follows the same monotonic transition duration.
                let progress = animation::progress(animation_now, t.born, duration_ms, lua_ease_out);
                let slide = 12 - (progress * 12 / 255) as i32;
                let base_x = if leftc { margin } else { bw as i32 - margin - cardw };
                let cx = if leftc { base_x - slide } else { base_x + slide };
                let cy = if top {
                    margin + slot as i32 * (cardh + gap)
                } else {
                    bh as i32 - margin - cardh - slot as i32 * (cardh + gap)
                };
                let view=desktop_tracks.sample(desktop_motion::NOTIFICATIONS,(cx,cy,cardw,cardh),animation_now);
                let (cx,cy,cardw,cardh)=view.rect;let radius=view.radius.unwrap_or(10);
                if view.blur>0{let need=(cardw.max(0)as usize).saturating_mul(cardh.max(0)as usize).min(bw*bh);if blur_a.len()<need{blur_a.resize(need,0);blur_b.resize(need,0);}blur_region(&mut back,bw,bh,cx,cy,cardw,cardh,view.blur,1,&mut blur_a,&mut blur_b);}
                draw_soft_shadow(&mut back,bw,bh,cx,cy,cardw,cardh,radius,if view.shadow>0{view.shadow}else{8},80*view.opacity as i32/255);
                fill_rrect_grad(&mut back, bw, bh, cx, cy, cardw, cardh, radius, RR_ALL, shade(theme.menu, 14), theme.menu, fx.menu_alpha*view.opacity as i32/255);
                // Accent bar on the leading edge marks it as a toast.
                fill_rrect(&mut back, bw, bh, cx, cy + 4, 4, cardh - 8, 2, RR_ALL, (view.opacity<<24)|(theme.launcher&0x00ffffff));
                if let Some(f) = font.as_ref() {
                    let baseline = cy + cardh / 2 + 5;
                    draw_text_ttf_alpha(&mut back,bw,bh,f,cx+14,baseline,px,&t.text,view.color.unwrap_or(theme.panel_text),view.opacity);
                }
            }
        }

        draw_cursor(&mut back, bw, bh, mx, my);


        let bytes =
            unsafe { core::slice::from_raw_parts(back.as_ptr() as *const u8, back.len() * 4) };
        libdunit::fb_present(bytes, fb.width, fb.height, 0, 0);
        if !desktop_ready_announced {
            libdunit::println("gui_server: desktop ready");
            desktop_ready_announced = true;
        }

        previous_animation_time = animation_now;
        libdunit::sleep_ms(16);
        ticks += 1;
        // Report average ms/frame once the loop is warm (a single line, so the
        // blur/compositing cost shows up in the headless serial log without
        // spamming it). >16 ms means compositing overran the frame budget.
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
        if ticks >= 60000 {
            break;
        }
    }

    // Session over: tell every client to exit its input loop so serve_two_clients
    // can reap them (a QUIT to an already-exited pid is a harmless no-op).
    for w in wins.iter() {
        send_input(w.pid, IN_QUIT, 0, 0, 0);
    }
    // Natural session end (not a live-resolution re-enter): caller does not loop.
    false
}
