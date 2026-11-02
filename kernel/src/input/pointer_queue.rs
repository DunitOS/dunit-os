//! Bounded device facts. The consumer decides what a lost stream means for UI.
use dunit_pointer_abi::{PointerEvent, PointerEventWithModifiers, LOST_BEFORE};
pub const CAPACITY: usize = 64;
#[derive(Clone, Copy)]
struct Entry {
    event: PointerEventWithModifiers,
    edge: bool,
}
const EMPTY: Entry = Entry {
    event: PointerEventWithModifiers::EMPTY,
    edge: false,
};
pub struct PointerQueue {
    entries: [Entry; CAPACITY],
    head: usize,
    len: usize,
    sequence: u64,
    buttons: u32,
}
impl PointerQueue {
    pub const fn new() -> Self {
        Self {
            entries: [EMPTY; CAPACITY],
            head: 0,
            len: 0,
            sequence: 0,
            buttons: 0,
        }
    }
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
    pub fn reset(&mut self, x: u32, y: u32, buttons: u32, modifiers: u32) {
        self.clear();
        self.buttons = buttons;
        self.push(x, y, buttons, 0, modifiers);
        self.entries[self.head].event.event.flags |= LOST_BEFORE;
        self.entries[self.head].edge = true;
    }
    pub fn push(&mut self, x: u32, y: u32, buttons: u32, wheel: i32, modifiers: u32) {
        self.sequence = self.sequence.wrapping_add(1);
        let wrapped = self.sequence == 0;
        if wrapped {
            self.sequence = 1;
            self.clear();
        }
        let edge = buttons != self.buttons;
        self.buttons = buttons;
        let mut event = PointerEventWithModifiers {
            event: PointerEvent {
                x,
                y,
                buttons,
                wheel,
                sequence: self.sequence,
                flags: if wrapped { LOST_BEFORE } else { 0 },
            },
            modifiers,
        };
        if self.len > 0 && !edge && wheel == 0 && !wrapped {
            let last = (self.head + self.len - 1) % CAPACITY;
            if !self.entries[last].edge
                && self.entries[last].event.event.wheel == 0
                && self.entries[last].event.modifiers == modifiers
            {
                event.event.flags |= self.entries[last].event.event.flags;
                self.entries[last].event = event;
                return;
            }
        }
        if self.len == CAPACITY {
            self.head = (self.head + 1) % CAPACITY;
            self.len -= 1;
            // Cancel capture BEFORE interpreting the first retained transition.
            self.entries[self.head].event.event.flags |= LOST_BEFORE;
        }
        self.entries[(self.head + self.len) % CAPACITY] = Entry { event, edge };
        self.len += 1;
    }
    pub fn peek(&self) -> Option<PointerEventWithModifiers> {
        if self.len == 0 {
            None
        } else {
            Some(self.entries[self.head].event)
        }
    }
    pub fn pop(&mut self) -> Option<PointerEventWithModifiers> {
        let e = self.peek()?;
        self.head = (self.head + 1) % CAPACITY;
        self.len -= 1;
        Some(e)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn motion_does_not_coalesce_across_modifier_boundaries() {
        let mut q = PointerQueue::new();
        q.push(1, 0, 0, 0, 0);
        q.push(2, 0, 0, 0, 1);
        q.push(3, 0, 0, 0, 1);
        q.push(4, 0, 0, 0, 3);
        q.push(5, 0, 0, 0, 0);
        let got: std::vec::Vec<_> = core::iter::from_fn(|| q.pop())
            .map(|record| (record.event.x, record.modifiers))
            .collect();
        assert_eq!(got, [(1, 0), (3, 1), (4, 3), (5, 0)]);
    }
    #[test]
    fn queued_edges_retain_snapshots_and_reset_uses_current_modifiers() {
        let mut q = PointerQueue::new();
        q.push(1, 2, 1, 0, 1);
        q.push(1, 2, 0, 0, 1);
        q.push(1, 2, 0, 0, 0);
        assert_eq!(q.pop().unwrap().modifiers, 1);
        assert_eq!(q.pop().unwrap().modifiers, 1);
        assert_eq!(q.pop().unwrap().modifiers, 0);
        q.reset(5, 6, 0, 2);
        let record = q.pop().unwrap();
        assert_eq!(record.modifiers, 2);
        assert_eq!(record.event.flags, LOST_BEFORE);
    }
    #[test]
    fn complete_click_survives_without_a_reader() {
        let mut q = PointerQueue::new();
        q.reset(1, 2, 0, 0);
        q.pop();
        q.push(10, 20, 1, 0, 0);
        q.push(11, 21, 0, 0, 0);
        let down = q.pop().unwrap().event;
        let up = q.pop().unwrap().event;
        assert_eq!((down.x, down.y, down.buttons), (10, 20, 1));
        assert_eq!((up.x, up.y, up.buttons), (11, 21, 0));
        assert!(down.sequence < up.sequence);
        assert!(q.pop().is_none());
    }
    #[test]
    fn motion_coalesces_but_edges_and_wheel_do_not() {
        let mut q = PointerQueue::new();
        q.push(1, 0, 1, 0, 0);
        q.push(2, 0, 1, 0, 0);
        q.push(3, 0, 1, 0, 0);
        q.push(4, 0, 1, 2, 0);
        q.push(5, 0, 1, -2, 0);
        assert_eq!(q.pop().unwrap().event.x, 1);
        assert_eq!(q.pop().unwrap().event.x, 3);
        assert_eq!(q.pop().unwrap().event.wheel, 2);
        assert_eq!(q.pop().unwrap().event.wheel, -2);
        assert!(q.pop().is_none());
    }
    #[test]
    fn overflow_marks_first_survivor_and_stays_bounded() {
        let mut q = PointerQueue::new();
        for n in 0..CAPACITY + 20 {
            q.push(n as u32, 0, (n % 2) as u32, 1, 0);
        }
        assert_eq!(q.len, CAPACITY);
        let first = q.pop().unwrap().event;
        assert_eq!(first.x, 20);
        assert_eq!(first.flags, LOST_BEFORE);
        let mut previous = first.sequence;
        while let Some(queued) = q.pop() {
            let e = queued.event;
            assert!(e.sequence > previous);
            assert_eq!(e.flags, 0);
            previous = e.sequence;
        }
    }
    #[test]
    fn handover_discards_old_session_and_peek_does_not_consume() {
        let mut q = PointerQueue::new();
        q.push(1, 2, 1, 1, 0);
        q.reset(30, 40, 0, 0);
        assert_eq!(q.peek(), q.peek());
        let e = q.pop().unwrap().event;
        assert_eq!(
            (e.x, e.y, e.buttons, e.wheel, e.flags),
            (30, 40, 0, 0, LOST_BEFORE)
        );
        assert!(q.pop().is_none());
    }
}
