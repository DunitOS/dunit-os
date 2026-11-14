//! Registration storage only: task/scheduler wake callbacks run after unlocking.
use super::ProcessId;
use crate::sync::IrqSafeSpinLock;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WaitKey {
    Timer,
    Ipc(ProcessId),
    /// Dunit-native futex key: threads share their owner's address space, so
    /// (owner, user virtual address of the word) uniquely names a futex word.
    Word {
        owner: ProcessId,
        addr: u64,
    },
    /// A pipe end, named by the global pipe-object id. Readers block here when
    /// the pipe is empty with writers still open; writers block when it is full
    /// with readers still open.
    Pipe(u64),
    Poll,
    WaitSet,
}

#[derive(Clone, Copy)]
pub(super) struct WaitEntry {
    pub(super) entity: ProcessId,
    pub(super) key: WaitKey,
    pub(super) deadline: Option<u64>,
}

struct Registration {
    generation: u64,
    entry: WaitEntry,
}

#[derive(Clone, Copy)]
pub(crate) struct Checkpoint(u64);

pub(super) struct WaitQueue {
    entries: Vec<Registration>,
    next_generation: u64,
    notifications: u64,
}

impl WaitQueue {
    pub(super) const fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_generation: 0,
            notifications: 0,
        }
    }

    pub(super) fn checkpoint(&self) -> Checkpoint {
        Checkpoint(self.notifications)
    }
    pub(super) fn changed(&self, checkpoint: Checkpoint) -> bool {
        self.notifications != checkpoint.0
    }

    pub(super) fn push(&mut self, entry: WaitEntry) -> u64 {
        let generation = self.next_generation;
        // Never reuse a generation: a callback may remove and re-register a PID.
        self.next_generation = generation
            .checked_add(1)
            .expect("wait generation exhausted");
        self.entries.push(Registration { generation, entry });
        generation
    }

    pub(super) fn remove_entity(&mut self, entity: ProcessId) {
        self.entries
            .retain(|registration| registration.entry.entity != entity);
    }
}

/// Wake only registrations present at the start of this pass, in FIFO order.
/// A callback may reenter, cancel or re-register without holding the queue lock.
/// No temporary batch allocation on the IRQ path. `max == 0` means all matches.
/// A checkpoint taken before the predicate is compared under this same lock
/// at admission; callbacks carry the selected registration identity.
pub(super) fn wake_matching(
    queue: &IrqSafeSpinLock<WaitQueue>,
    matches: impl Fn(&WaitEntry) -> bool,
    max: usize,
    mut wake: impl FnMut(WaitEntry, u64),
) -> usize {
    let cutoff = {
        let mut queue = queue.lock();
        // Publish even with no registrations: a producer may win the
        // predicate/check/register gap. Checkpoint admission observes it.
        queue.notifications = queue
            .notifications
            .checked_add(1)
            .expect("wait notification exhausted");
        queue.next_generation
    };
    let mut woken = 0;
    while max == 0 || woken < max {
        let entry = {
            let mut queue = queue.lock();
            let index = queue.entries.iter().position(|registration| {
                registration.generation < cutoff && matches(&registration.entry)
            });
            index.map(|index| queue.entries.remove(index))
        };
        let Some(entry) = entry else {
            break;
        };
        wake(entry.entry, entry.generation);
        woken += 1;
    }
    woken
}
