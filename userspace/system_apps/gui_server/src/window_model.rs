//! Window placement, decorations, shared hit geometry and session persistence.
use super::*;
/// Screen margins reserved by configured shell surfaces.
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
/// replays that snapshot. Geometry follows the request immediately; the current
/// buffer stays owned and scales until its replacement is acknowledged.
/// Reapplying Maximized refreshes the work area after a display change.
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
    win.state = new_state;
    if let Some(target) = target {
        request_window_geometry(server, clients, win, target);
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

/// Restore a tiled window under the same proportional title-bar grip.
pub(super) fn begin_window_move(
    server: &mut Server, clients: &mut [ClientState], win: &mut Win, window: usize,
    point: (i32, i32), screen: (usize, usize), ly: &Layout, ins: &Insets,
) -> Capture {
    let (x, y) = point;
    win.motion = None;
    if win.is_tiled() {
        let old_width = win.sw.max(1);
        let grip_x = (x - win.cx).clamp(0, old_width);
        let grip_y = (y - win.cy + win.title_h).clamp(0, (win.title_h - 1).max(0));
        let restored_width = win.restore.map(|r| r.2).unwrap_or(old_width).max(1);
        set_window_state(server, clients, win, WinState::Floating, screen.0, screen.1, ly, ins);
        win.cx = x - (grip_x as i64 * restored_width as i64 / old_width as i64) as i32;
        win.cy = y - grip_y + win.title_h;
    }
    let outer = win.outer();
    Capture::Move { window, offset: (x - outer.0, y - outer.1) }
}

pub(super) fn advance_window_capture(
    server: &mut Server, clients: &mut [ClientState], win: &mut Win, capture: Capture,
    point: (i32, i32), screen: (usize, usize), ins: &Insets,
    minimum: (i32, i32),
) {
    let bounds = (ins.left, ins.top, (screen.0 as i32 - ins.left - ins.right).max(1),
                  (screen.1 as i32 - ins.top - ins.bottom).max(1));
    let minimum = (minimum.0 + 2 * win.border,
                   minimum.1 + win.title_h + 2 * win.border);
    let rect = capture.geometry(win.outer(), point, bounds, minimum);
    let position = (rect.0 + win.border, rect.1 + win.title_h + win.border);
    let size = ((rect.2 - 2 * win.border).max(1),
                (rect.3 - win.title_h - 2 * win.border).max(1));
    if (win.sw, win.sh) == size {
        win.cx = position.0;
        win.cy = position.1;
    } else {
        request_window_geometry(server, clients, win, (position.0, position.1, size.0, size.1));
    }
}

pub(super) fn restore_floating_geometry(
    server: &mut Server, clients: &mut [ClientState], win: &mut Win,
    geometry: (i32, i32, i32, i32), screen: (usize, usize), ly: &Layout, ins: &Insets,
    minimum: (i32, i32),
) {
    let area = work_area(screen.0, screen.1, ly, ins);
    let width = geometry.2.clamp(minimum.0.max(1).min(area.2), area.2);
    let height = geometry.3.clamp(minimum.1.max(1).min(area.3), area.3);
    request_window_geometry(server, clients, win,
        (geometry.0.clamp(area.0, area.0 + area.2 - width),
         geometry.1.clamp(area.1, area.1 + area.3 - height), width, height));
}

pub(super) fn window_minimum(
    win: &Win, skin: &dunit_style::shell::Visuals,
    policy: &settings::policy::PolicyConfig, apps: &Applications,
) -> (i32, i32) {
    let app = win_app_id(apps, win.app).unwrap_or("");
    let resolved = policy.for_app(app);
    (resolved.min_width.unwrap_or(skin.window_min_width).max(skin.window_min_width),
     resolved.min_height.unwrap_or(skin.window_min_height).max(skin.window_min_height))
}

fn request_window_geometry(
    server: &mut Server, clients: &mut [ClientState], win: &mut Win,
    geometry: (i32, i32, i32, i32),
) {
    let (x, y, width, height) = geometry;
    let pending = win.desired_size.is_some();
    win.cx = x;
    win.cy = y;
    win.sw = width;
    win.sh = height;
    win.desired_size = (pending || (win.surface_w, win.surface_h) != (width, height))
        .then_some((width, height));
    if win.desired_size.is_some() {
        reconfigure_client(server, clients, win.pid, width as u32, height as u32, 1);
    }
}

// The session path is in MemFS. Reboot persistence remains a separate gate.

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
pub(super) fn session_remember(session: &mut Session, apps: &Applications, w: &Win) {
    let Some(id) = win_app_id(apps, w.app) else {
        return;
    };
    let (gx, gy, gw, gh) = match w.restore {
        Some((rx, ry, rw, rh)) if w.state != WinState::Floating => (rx, ry, rw, rh),
        _ => (w.cx, w.cy, w.sw, w.sh),
    };
    let maximized = w.state == WinState::Maximized;
    let ge = settings::WinGeom::new(id, gx, gy, gw, gh, maximized);
    session.remember(ge);
}

/// Find a remembered geometry for the app `app` (registry index) in the mirror.
pub(super) fn session_lookup(
    session: &Session,
    apps: &Applications,
    app: u8,
) -> Option<settings::WinGeom> {
    let id = win_app_id(apps, app)?;
    session.lookup(id)
}

/// Human-readable window title for a window's registry index (`Win.app`), looked
/// up in the live application registry. An unknown index (0xFF, e.g. a client
/// spawned for an app not in the registry) falls back to a generic "Window".
pub(super) fn win_title<'a>(apps: &'a Applications, app: u8) -> &'a str {
    apps.apps
        .get(app as usize)
        .map(|a| a.name.as_str())
        .unwrap_or("Window")
}

/// Floating geometry, a Lua tile or the maximized work area.
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
    pub(super) size: (i32, i32),
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
    /// Resize probe: 0=request maximize, 1=await maximize, 2=await restore,
    /// 3=settled. Enabled only by the test desktop configuration.
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
    /// Monotonic client commit counter mirrored from `ClientState.buf_seq`.
    pub(super) buf_seq: u64,
}

impl Win {
    pub(super) fn from_client(client: &ClientState, origin: (i32, i32), ly: &Layout, now: u64, ws: usize) -> Self {
        let (sw, sh) = (client.surf_w as i32, client.surf_h as i32);
        Self {
            pid: client.pid, cx: origin.0, cy: origin.1, sw, sh,
            surface_w: sw, surface_h: sh, desired_size: None,
            buf_ptr: client.buf_ptr, buf_size: client.buf_size, alive: true,
            traffic_lights: ly.traffic_lights, title_h: ly.title_h, border: ly.border,
            app: client.app, opacity: 255, rule_applied: false, ws, born: now,
            minimized: false, motion: None, state: WinState::Floating,
            lua_floating: false, restore: None, st_step: 0, st_deadline: None,
            format: client.format, buf_seq: client.buf_seq,
        }
    }
    pub(super) fn resize_edges(&self, x: i32, y: i32, skin: &dunit_style::shell::Visuals) -> Option<window_gesture::Edges> {
        if self.state != WinState::Floating { return None; }
        window_gesture::resize_edges(self.outer(), (x, y), skin.window_resize_grip)
    }
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
