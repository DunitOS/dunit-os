//! Window placement, decorations, shared hit geometry and session persistence.
use super::*;
/// Opcode of an outbound protocol packet (offset 8, little-endian u16).
/// Reserved screen margins (px) that the shell strips subtract from the usable
/// desktop, one accumulator per edge. `reserved_insets` fills it from the
/// configured panel/tray/dock edges, and every geometry
/// computation (work area, spawn/drag clamps, dock strip origin) reads from here
/// so "what real estate is free" has a single source of truth.
#[derive(Clone, Copy)]
pub(super) struct Insets {
    pub(super) top: i32,
    pub(super) bottom: i32,
    pub(super) left: i32,
    pub(super) right: i32,
}

/// Add `size` px to the margin on `edge`.
pub(super) fn add_edge(ins: &mut Insets, edge: Edge, size: i32) {
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
pub(super) fn strip_rect(
    edge: Edge,
    thickness: i32,
    off: i32,
    ins: &Insets,
    bw: usize,
    bh: usize,
) -> (i32, i32, i32, i32) {
    let bw = bw as i32;
    let bh = bh as i32;
    match edge {
        Edge::Top => (0, off, bw, thickness),
        Edge::Bottom => (0, bh - off - thickness, bw, thickness),
        Edge::Left => (off, ins.top, thickness, (bh - ins.top - ins.bottom).max(1)),
        Edge::Right => (
            bw - off - thickness,
            ins.top,
            thickness,
            (bh - ins.top - ins.bottom).max(1),
        ),
    }
}

/// The maximize target rect `(cx, cy, w, h)` = the framebuffer minus the reserved
/// shell margins (`reserved_insets`) and the window's own title bar + border. One
/// helper so the interactive chip, the drag-restore path and the headless
/// self-test all compute the same geometry, and so it honours whatever edges the
/// panel/tray are configured on.
pub(super) fn work_area(bw: usize, bh: usize, ly: &Layout, ins: &Insets) -> (i32, i32, i32, i32) {
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
pub(super) fn set_window_state(
    server: &mut Server,
    clients: &mut [ClientState],
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
    win.desired_size = None;
    win.state = new_state;
    if let Some((tx, ty, tw, th)) = target {
        win.cx = tx;
        win.cy = ty;
        reconfigure_client(
            server,
            clients,
            win.pid,
            tw as u32,
            th as u32,
            STATE_ACTIVATED,
        );
    }
}

/// Toggle the title-bar maximize chip: `Maximized` <-> `Floating`.
pub(super) fn toggle_maximize(
    server: &mut Server,
    clients: &mut [ClientState],
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
pub(super) fn win_app_id<'a>(apps: &'a Applications, app: u8) -> Option<&'a str> {
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
pub(super) fn session_remember(session: &mut Vec<settings::WinGeom>, apps: &Applications, w: &Win) {
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
pub(super) fn session_lookup(
    session: &[settings::WinGeom],
    apps: &Applications,
    app: u8,
) -> Option<settings::WinGeom> {
    let id = win_app_id(apps, app)?;
    session.iter().find(|e| e.id.as_str() == id).copied()
}

// Hardware make-codes the switcher watches. The trigger key (Tab) and the cancel
// key (Esc) are keyboard-protocol invariants, not desktop policy — only which
// MODIFIER arms the switcher is config (`[shortcuts] switch_mod`).
/// Human-readable window title for a window's registry index (`Win.app`), looked
/// up in the live application registry. An unknown index (0xFF, e.g. a client
/// spawned for an app not in the registry) falls back to a generic "Window".
pub(super) fn win_title<'a>(apps: &'a Applications, app: u8) -> &'a str {
    apps.apps
        .get(app as usize)
        .map(|a| a.name.as_str())
        .unwrap_or("Window")
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
pub(super) enum WinState {
    Tiled,
    Floating,
    Maximized,
}

/// Window policy retained across display-session re-entry. Client buffers stay
/// owned by ClientState; this records only the compositor's placement decisions.
#[derive(Clone, Copy)]
pub(super) struct Placement {
    pub(super) cx: i32,
    pub(super) cy: i32,
    pub(super) state: WinState,
    pub(super) restore: Option<(i32, i32, i32, i32)>,
    pub(super) lua_floating: bool,
    pub(super) st_step: u8,
    pub(super) minimized: bool,
    pub(super) ws: usize,
}

/// Per-window compositor state, derived from a `ClientState` but with a live
/// content origin the user can drag around.
#[derive(Clone, Copy)]
pub(super) struct Motion {
    pub(super) from: (i32, i32),
    pub(super) to: (i32, i32),
    pub(super) start: u64,
    pub(super) duration_ms: u32,
    pub(super) ease_out: bool,
}

#[derive(Clone)]
pub(super) struct Win {
    pub(super) motion: Option<Motion>,
    pub(super) pid: u32,
    pub(super) lua_floating: bool,
    pub(super) cx: i32,
    pub(super) cy: i32,
    pub(super) sw: i32,
    pub(super) sh: i32,
    pub(super) surface_w: i32,
    pub(super) surface_h: i32,
    pub(super) desired_size: Option<(i32, i32)>,
    pub(super) buf_ptr: *const u8,
    pub(super) buf_size: usize,
    pub(super) alive: bool,
    /// Decoration geometry copied from the resolved `[layout]` config so each
    /// window's hit-tests and framing stay self-contained.
    pub(super) title_h: i32,
    pub(super) border: i32,
    pub(super) traffic_lights: bool,
    /// Index into the live application registry (0xFF = unknown) — drives the
    /// dock running marker and the window title (see `win_title`).
    pub(super) app: u8,
    pub(super) opacity: u8,
    pub(super) rule_applied: bool,
    /// Workspace (0-based) this window belongs to; only the active workspace's
    /// windows are drawn and receive input.
    pub(super) ws: usize,
    /// Monotonic milliseconds when this window appeared — drives its open fade.
    pub(super) born: u64,
    /// Minimized windows are not composited and receive no input; a click on the
    /// app's dock icon (task switcher) restores and raises them.
    pub(super) minimized: bool,
    /// Current layout state (see `WinState`). Driven by the title-bar maximize
    /// chip and by dragging a tiled window's title bar (macOS-style restore).
    pub(super) state: WinState,
    /// Floating geometry `(cx, cy, sw, sh)` snapshotted when the window leaves the
    /// floating state, so returning to `Floating` restores the exact pre-maximize
    /// position and size. `None` while the window is floating.
    pub(super) restore: Option<(i32, i32, i32, i32)>,
    /// Self-test choreography step (config-gated `[startup] self_test`, test.toml
    /// only): 0 = not yet maximized, 1 = maximized (awaiting restore), 2 = restored.
    /// Lets the headless harness drive a full maximize->restore round trip without a
    /// mouse. Always 0 on the real desktop (`self_test` is false in `default.toml`).
    pub(super) st_step: u8,
    /// Absolute deadline for the resize self-test's existing four-second grace.
    /// A resumed probe receives a fresh deadline after display-session re-entry.
    pub(super) st_deadline: Option<frame_pacing::FrameDeadline>,
    /// Pixel format the owning client imported its buffer with (mirrored from
    /// `ClientState.format` in the per-frame sync). `FORMAT_ARGB8888` selects the
    /// per-pixel src-over blit in `blit_surface` so a translucent client (e.g. a
    /// terminal with `bg_alpha < 255`) shows the desktop through its background;
    /// `FORMAT_XRGB8888` keeps the opaque fast-path copy.
    pub(super) format: u32,
}

impl Win {
    /// Full outer rect (border + title bar + content) in screen space.
    pub(super) fn outer(&self) -> (i32, i32, i32, i32) {
        let x = self.cx - self.border;
        let y = self.cy - self.title_h - self.border;
        let w = self.sw + 2 * self.border;
        let h = self.sh + self.title_h + 2 * self.border;
        (x, y, w, h)
    }
    pub(super) fn contains(&self, mx: i32, my: i32) -> bool {
        let (x, y, w, h) = self.outer();
        mx >= x && mx < x + w && my >= y && my < y + h
    }
    pub(super) fn content_position(&self, x: i32, y: i32) -> (i32, i32) {
        (
            ((x - self.cx) as i64 * self.surface_w as i64 / self.sw.max(1) as i64) as i32,
            ((y - self.cy) as i64 * self.surface_h as i64 / self.sh.max(1) as i64) as i32,
        )
    }
    /// The client content region (below the title bar).
    pub(super) fn in_content(&self, mx: i32, my: i32) -> bool {
        mx >= self.cx && mx < self.cx + self.sw && my >= self.cy && my < self.cy + self.sh
    }
    pub(super) fn in_title(&self, mx: i32, my: i32) -> bool {
        mx >= self.cx && mx < self.cx + self.sw && my >= self.cy - self.title_h && my < self.cy
    }
    /// Role 0=close, 1=minimize, 2=maximize; one rect for paint and input.
    pub(super) fn control_rect(
        &self,
        role: i32,
        skin: &dunit_style::shell::Visuals,
    ) -> (i32, i32, i32, i32) {
        skin.control_rect(
            (self.cx, self.cy, self.sw, self.sh),
            self.title_h,
            self.traffic_lights,
            role,
        )
    }
    pub(super) fn in_close(&self, mx: i32, my: i32, skin: &dunit_style::shell::Visuals) -> bool {
        in_rect(self.control_rect(0, skin), mx, my)
    }
    pub(super) fn in_min(&self, mx: i32, my: i32, skin: &dunit_style::shell::Visuals) -> bool {
        in_rect(self.control_rect(1, skin), mx, my)
    }
    pub(super) fn in_max(&self, mx: i32, my: i32, skin: &dunit_style::shell::Visuals) -> bool {
        in_rect(self.control_rect(2, skin), mx, my)
    }
    /// True when the window is not in its free-form floating state (currently only
    /// `Maximized`): it occupies a compositor-computed region, so a title-bar drag
    /// restores it to `restore` before following the pointer (macOS-style).
    pub(super) fn is_tiled(&self) -> bool {
        self.state != WinState::Floating
    }
}
