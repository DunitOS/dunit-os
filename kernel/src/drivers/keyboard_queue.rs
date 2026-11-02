//! PS/2 queue records carry modifier/extension state at arrival, not at drain.
const CAPACITY: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scancode {
    pub raw: u8,
    pub modifiers: u8,
    pub extended: bool,
}
impl Scancode {
    const EMPTY: Self = Self {
        raw: 0,
        modifiers: 0,
        extended: false,
    };
}
pub struct ScancodeQueue {
    buffer: [Scancode; CAPACITY],
    read: usize,
    write: usize,
    held_modifiers: u8,
    extended: bool,
}
impl ScancodeQueue {
    pub const fn new() -> Self {
        Self {
            buffer: [Scancode::EMPTY; CAPACITY],
            read: 0,
            write: 0,
            held_modifiers: 0,
            extended: false,
        }
    }
    pub fn modifiers(&self) -> u8 {
        // Both physical sides contribute to each logical modifier. Releasing
        // one side must not clear the other still-held key.
        u8::from(self.held_modifiers & 0x03 != 0)
            | (u8::from(self.held_modifiers & 0x0c != 0) << 1)
            | (u8::from(self.held_modifiers & 0x30 != 0) << 2)
            | (u8::from(self.held_modifiers & 0xc0 != 0) << 3)
    }
    pub fn push(&mut self, raw: u8) {
        let extended = self.extended;
        self.extended = raw == 0xE0;
        // E0 distinguishes right Ctrl/Alt and both Super keys. PrintScreen's
        // E0-prefixed fake Shift is not a physical Shift transition.
        let mask = match (raw & 0x7f, extended) {
            (0x2a, false) => 1,
            (0x36, false) => 2,
            (0x1d, false) => 4,
            (0x1d, true) => 8,
            (0x38, false) => 16,
            (0x38, true) => 32,
            (0x5b, true) => 64,
            (0x5c, true) => 128,
            _ => 0,
        };
        if raw & 0x80 == 0 {
            self.held_modifiers |= mask;
        } else {
            self.held_modifiers &= !mask;
        }
        let next = (self.write + 1) % CAPACITY;
        if next != self.read {
            self.buffer[self.write] = Scancode {
                raw,
                modifiers: self.modifiers(),
                extended,
            };
            self.write = next;
        }
    }
    pub fn pop(&mut self) -> Option<Scancode> {
        if self.read == self.write {
            return None;
        }
        let event = self.buffer[self.read];
        self.read = (self.read + 1) % CAPACITY;
        Some(event)
    }
    pub fn pop_event(&mut self) -> Option<Scancode> {
        while let Some(event) = self.pop() {
            if event.raw != 0xE0 {
                return Some(event);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn releasing_either_modifier_side_keeps_the_other_held() {
        let cases: &[(&[u8], &[u8], &[u8], &[u8], u8)] = &[
            (&[0x2a], &[0x36], &[0xaa], &[0xb6], 1),
            (&[0x1d], &[0xe0, 0x1d], &[0x9d], &[0xe0, 0x9d], 2),
            (&[0x38], &[0xe0, 0x38], &[0xb8], &[0xe0, 0xb8], 4),
            (
                &[0xe0, 0x5b],
                &[0xe0, 0x5c],
                &[0xe0, 0xdb],
                &[0xe0, 0xdc],
                8,
            ),
        ];
        for &(left, right, left_up, right_up, mask) in cases {
            for reverse in [false, true] {
                let mut q = ScancodeQueue::new();
                for &byte in left.iter().chain(right) {
                    q.push(byte);
                }
                let (first, second) = if reverse {
                    (right_up, left_up)
                } else {
                    (left_up, right_up)
                };
                for &byte in first {
                    q.push(byte);
                }
                assert_eq!(q.modifiers(), mask);
                q.push(0x1e);
                let key = core::iter::from_fn(|| q.pop_event()).last().unwrap();
                assert_eq!((key.raw, key.modifiers), (0x1e, mask));
                for &byte in second {
                    q.push(byte);
                }
                assert_eq!(q.modifiers(), 0);
            }
        }
    }
    #[test]
    fn repeated_makes_and_printscreen_fake_shifts_do_not_corrupt_physical_state() {
        let mut q = ScancodeQueue::new();
        for raw in [0xe0, 0x2a, 0xe0, 0xaa] {
            q.push(raw);
        }
        assert_eq!(q.modifiers(), 0);
        q.push(0x2a);
        q.push(0x2a); // typematic repeats are not additional physical holds
        for raw in [0xe0, 0x2a, 0xe0, 0xaa] {
            q.push(raw);
        }
        assert_eq!(q.modifiers(), 1);
        q.push(0xaa);
        assert_eq!(q.modifiers(), 0);
    }
    #[test]
    fn complete_shortcut_released_before_drain_keeps_press_modifiers() {
        let mut q = ScancodeQueue::new();
        for raw in [0xE0, 0x5B, 0x32, 0xB2, 0xE0, 0xDB] {
            q.push(raw);
        }
        assert_eq!(q.modifiers(), 0);
        let events: std::vec::Vec<_> = core::iter::from_fn(|| q.pop_event()).collect();
        assert_eq!(
            events
                .iter()
                .map(|e| (e.raw, e.modifiers))
                .collect::<std::vec::Vec<_>>(),
            [(0x5B, 8), (0x32, 8), (0xB2, 8), (0xDB, 0)]
        );
        assert!(events[0].extended && events[3].extended);
        assert!(!events[1].extended);
    }
    #[test]
    fn extension_survives_prefix_and_payload_in_separate_drains() {
        let mut q = ScancodeQueue::new();
        q.push(0xE0);
        assert_eq!(q.pop_event(), None);
        q.push(0x4B);
        assert_eq!(
            q.pop_event(),
            Some(Scancode {
                raw: 0x4B,
                modifiers: 0,
                extended: true
            })
        );
        q.push(0x1E);
        assert!(!q.pop_event().unwrap().extended);
    }
    #[test]
    fn snapshots_follow_shift_ctrl_and_alt_transitions_in_fifo_order() {
        let mut q = ScancodeQueue::new();
        for raw in [0x2A, 0x1D, 0x38, 0x1E, 0xAA, 0x9D, 0xB8] {
            q.push(raw);
        }
        assert_eq!(q.modifiers(), 0);
        let masks: std::vec::Vec<_> = core::iter::from_fn(|| q.pop())
            .map(|e| e.modifiers)
            .collect();
        assert_eq!(masks, [1, 3, 7, 7, 6, 4, 0]);
    }
    #[test]
    fn ring_wrap_and_overflow_preserve_raw_order_and_live_state() {
        let mut q = ScancodeQueue::new();
        for _ in 0..CAPACITY * 3 {
            q.push(0x1E);
            assert_eq!(q.pop().unwrap().raw, 0x1E);
        }
        for _ in 0..CAPACITY - 1 {
            q.push(0x1E);
        }
        q.push(0x2A); // dropped record still updates the physical key state
        assert_eq!(q.modifiers(), 1);
        for _ in 0..CAPACITY - 1 {
            assert_eq!(q.pop().unwrap().raw, 0x1E);
        }
        assert_eq!(q.pop(), None);
        q.push(0xAA);
        assert_eq!(q.pop().unwrap().modifiers, 0);
    }
}
