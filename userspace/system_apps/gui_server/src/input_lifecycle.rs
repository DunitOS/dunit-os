//! Keyboard focus and physical-key ownership, independent of native transport.
//! Repeats of a held key cannot become a fresh press in another focus scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FocusChange {
    pub previous: Option<u32>,
    pub current: Option<u32>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyObservation {
    pub fresh: bool,
    pub released_to: Option<u32>,
}
pub(crate) struct Lifecycle {
    focused: Option<u32>,
    held: [bool; 128],
    owners: [Option<u32>; 128],
    cancelled: [bool; 128],
}
impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}
impl Lifecycle {
    pub const fn new() -> Self {
        Self {
            focused: None,
            held: [false; 128],
            owners: [None; 128],
            cancelled: [false; 128],
        }
    }
    pub fn current(&self)->Option<u32>{self.focused}
    pub fn focus(&mut self, next: Option<u32>) -> Option<FocusChange> {
        if next == self.focused {
            return None;
        }
        let previous = self.focused;
        self.focused = next;
        for (held, cancelled) in self.held.iter().zip(&mut self.cancelled) {
            if *held {
                *cancelled = true;
            }
        }
        Some(FocusChange {
            previous,
            current: next,
        })
    }
    /// Observe every make/break, including compositor-owned shortcuts. An
    /// intercepted make stays held without an owner until its physical break.
    pub fn observe(&mut self, scancode: u8, pressed: bool) -> KeyObservation {
        let index = scancode as usize;
        if index >= self.held.len() {
            return KeyObservation::default();
        }
        if pressed {
            let fresh = !self.held[index];
            if fresh {
                self.owners[index] = None;
                self.cancelled[index] = false;
            }
            self.held[index] = true;
            KeyObservation {
                fresh,
                released_to: None,
            }
        } else {
            self.held[index] = false;
            let owner = self.owners[index].take();
            let released_to =
                owner.filter(|pid| !self.cancelled[index] && self.focused == Some(*pid));
            self.cancelled[index] = false;
            KeyObservation {
                fresh: false,
                released_to,
            }
        }
    }
    /// Called only for a make actually eligible for a negotiated client. Legacy
    /// clients continue to receive their existing make stream independently.
    pub fn deliver_make(&mut self, scancode: u8, pid: u32, fresh: bool) -> bool {
        let index = scancode as usize;
        if index >= self.held.len()
            || self.focused != Some(pid)
            || !self.held[index]
            || self.cancelled[index]
        {
            return false;
        }
        if fresh {
            self.owners[index] = Some(pid);
        }
        self.owners[index] == Some(pid)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focus_transitions_are_once_and_old_precedes_new() {
        let mut l = Lifecycle::new();
        assert_eq!(
            l.focus(Some(1)),
            Some(FocusChange {
                previous: None,
                current: Some(1)
            })
        );
        assert_eq!(l.focus(Some(1)), None);
        assert_eq!(
            l.focus(Some(2)),
            Some(FocusChange {
                previous: Some(1),
                current: Some(2)
            })
        );
        assert_eq!(
            l.focus(None),
            Some(FocusChange {
                previous: Some(2),
                current: None
            })
        );
    }
    #[test]
    fn ordinary_repeats_have_one_owner_and_one_release() {
        let mut l = Lifecycle::new();
        l.focus(Some(1));
        let first = l.observe(0x39, true);
        assert!(first.fresh);
        assert!(l.deliver_make(0x39, 1, first.fresh));
        let repeat = l.observe(0x39, true);
        assert!(!repeat.fresh);
        assert!(l.deliver_make(0x39, 1, repeat.fresh));
        assert_eq!(l.observe(0x39, false).released_to, Some(1));
        assert_eq!(l.observe(0x39, false).released_to, None);
    }
    #[test]
    fn held_press_cannot_migrate_or_reactivate_after_focus_returns() {
        let mut l = Lifecycle::new();
        l.focus(Some(1));
        let e = l.observe(0x39, true);
        assert!(l.deliver_make(0x39, 1, e.fresh));
        l.focus(Some(2));
        let e = l.observe(0x39, true);
        assert!(!l.deliver_make(0x39, 2, e.fresh));
        l.focus(Some(1));
        let e = l.observe(0x39, true);
        assert!(!l.deliver_make(0x39, 1, e.fresh));
        assert_eq!(l.observe(0x39, false).released_to, None);
        let e = l.observe(0x39, true);
        assert!(l.deliver_make(0x39, 1, e.fresh));
    }
    #[test]
    fn shell_consumed_make_does_not_gain_owner_on_repeat() {
        let mut l = Lifecycle::new();
        l.focus(None);
        assert!(l.observe(0x0f, true).fresh);
        l.focus(Some(2));
        let e = l.observe(0x0f, true);
        assert!(!l.deliver_make(0x0f, 2, e.fresh));
        l.observe(0x0f, false);
        let e = l.observe(0x0f, true);
        assert!(l.deliver_make(0x0f, 2, e.fresh));
    }
    #[test]
    fn unknown_scancode_cannot_index_key_state() {
        let mut l = Lifecycle::new();
        l.focus(Some(1));
        assert_eq!(l.observe(255, true), KeyObservation::default());
        assert!(!l.deliver_make(255, 1, true));
    }
}
