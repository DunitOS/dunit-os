//! Keyboard navigation, focus order and client removal bookkeeping.
use super::*;
pub(super) const SC_TAB: u8 = 0x0F;
pub(super) const SC_ESC: u8 = 0x01;
// Make-codes the global desktop shortcuts watch. Like Tab/Esc above these are
// keyboard-protocol invariants (the physical key positions), NOT desktop policy —
// only WHICH MODIFIER arms them and WHETHER each action is enabled is config
// (`[shortcuts] cmd_mod` + the per-action bools). The digit row 1..9/0 is the
// PC set-1 make sequence 0x02..0x0B.
pub(super) const SC_SPACE: u8 = 0x39;
pub(super) const SC_Q: u8 = 0x10;
pub(super) const SC_M: u8 = 0x32;
pub(super) const SC_UP: u8 = 0x48;
pub(super) const SC_DOWN: u8 = 0x50;
pub(super) const SC_ENTER: u8 = 0x1C;

/// Map a top-row digit make-code to its value 1..=9 (0 maps to 10, so it can
/// index workspace 10 when there are that many). Non-digit scancodes yield None.
pub(super) fn digit_from_scancode(sc: u8) -> Option<u32> {
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
pub(super) enum ShortcutAction {
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
pub(super) fn match_shortcut(
    scancode: u8,
    mods: u8,
    pressed: bool,
    sc: &settings::Shortcuts,
    ws_count: usize,
) -> ShortcutAction {
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
pub(super) struct Switcher {
    pub(super) active: bool,
    pub(super) order: Vec<usize>,
    pub(super) idx: usize,
}

impl Switcher {
    pub(super) fn new() -> Self {
        Switcher {
            active: false,
            order: Vec::new(),
            idx: 0,
        }
    }

    /// Arm over a fresh MRU snapshot. The current front window is index 0, so the
    /// first Tab moves to the next candidate (classic Alt-Tab "previous window").
    pub(super) fn begin(&mut self, mru: &[usize], backward: bool) {
        self.order.clear();
        self.order.extend_from_slice(mru);
        self.active = !self.order.is_empty();
        self.idx = 0;
        if self.order.len() >= 2 {
            self.advance(backward);
        }
    }

    pub(super) fn advance(&mut self, backward: bool) {
        let n = self.order.len();
        if n == 0 {
            return;
        }
        self.idx = if backward {
            (self.idx + n - 1) % n
        } else {
            (self.idx + 1) % n
        };
    }

    pub(super) fn selected(&self) -> Option<usize> {
        self.order.get(self.idx).copied()
    }

    pub(super) fn cancel(&mut self) {
        self.active = false;
    }

    /// Close the overlay and yield the highlighted window index to raise.
    pub(super) fn commit(&mut self) -> Option<usize> {
        let sel = self.selected();
        self.active = false;
        sel
    }
}

/// Candidate windows for the switcher in most-recently-used order (topmost of the
/// z-order first). Only live windows on the active workspace are offered; a
/// minimized one is still listed — committing to it un-minimizes it.
pub(super) fn switcher_mru(z: &[usize], wins: &[Win], ws: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for &i in z.iter().rev() {
        if wins.get(i).map_or(false, |w| w.alive && w.ws == ws) {
            out.push(i);
        }
    }
    out
}

/// Move `wi` to the top of the z-order (mirrors the click-to-raise path).
pub(super) fn raise_window(z: &mut Vec<usize>, wi: usize) {
    z.retain(|&i| i != wi);
    z.push(wi);
}

/// Repair the three `Option<wins index>` cursors after the `Win` at `wi` was
/// removed from `wins`. An index equal to the removed slot is cleared; any index
/// above it shifts down by one to track the `Vec::remove` compaction. `drag`
/// carries a `(win, off_x, off_y)` tuple; the other two are bare indices.
pub(super) fn adjust_indices_after_removal(
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
pub(super) fn reap_windows_of(
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
