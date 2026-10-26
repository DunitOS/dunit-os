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
    modifiers: u8,
    extended: bool,
}
impl ScancodeQueue {
    pub const fn new() -> Self {
        Self {
            buffer: [Scancode::EMPTY; CAPACITY],
            read: 0,
            write: 0,
            modifiers: 0,
            extended: false,
        }
    }
    pub fn modifiers(&self) -> u8 {
        self.modifiers
    }
    pub fn push(&mut self, raw: u8) {
        let extended = self.extended;
        self.extended = raw == 0xE0;
        // Preserve the existing Shift/Ctrl/Alt/Super tracking and raw-byte order.
        let (mask, down) = match raw {
            0x2A | 0x36 => (1, true),
            0xAA | 0xB6 => (1, false),
            0x1D => (2, true),
            0x9D => (2, false),
            0x38 => (4, true),
            0xB8 => (4, false),
            0x5B | 0x5C if extended => (8, true),
            0xDB | 0xDC if extended => (8, false),
            _ => (0, false),
        };
        if down {
            self.modifiers |= mask;
        } else {
            self.modifiers &= !mask;
        }
        let next = (self.write + 1) % CAPACITY;
        if next != self.read {
            self.buffer[self.write] = Scancode {
                raw,
                modifiers: self.modifiers,
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
