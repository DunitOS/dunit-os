use super::{ProcessError, ProcessId};
use crate::sync::{InterruptGuard, IrqSafeSpinLock};
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Candidate {
    pid: ProcessId,
    generation: u64,
}

/// Queue storage only. Process-state lookup must never run under this lock.
pub struct Scheduler {
    ready_queue: Vec<Candidate>,
    cursor: usize,
    generation: u64,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            ready_queue: Vec::new(),
            cursor: 0,
            generation: 0,
        }
    }

    fn enqueue(&mut self, pid: ProcessId) {
        if self.ready_queue.iter().any(|entry| entry.pid == pid) {
            return;
        }
        self.generation = self
            .generation
            .checked_add(1)
            .expect("scheduler generation exhausted");
        self.ready_queue.push(Candidate {
            pid,
            generation: self.generation,
        });
    }

    fn remove_candidate(&mut self, candidate: Candidate) {
        if let Some(index) = self
            .ready_queue
            .iter()
            .position(|entry| *entry == candidate)
        {
            self.ready_queue.remove(index);
            // next_candidate already advanced past this entry; account for the
            // shift so removing a stale entry does not skip its successor.
            if index < self.cursor {
                self.cursor -= 1;
            }
        }
    }

    pub fn remove(&mut self, pid: ProcessId) {
        if let Some(candidate) = self
            .ready_queue
            .iter()
            .find(|entry| entry.pid == pid)
            .copied()
        {
            self.remove_candidate(candidate);
        }
    }

    fn next_candidate(&mut self) -> Option<Candidate> {
        if self.ready_queue.is_empty() {
            return None;
        }
        if self.cursor >= self.ready_queue.len() {
            self.cursor = 0;
        }
        let candidate = self.ready_queue[self.cursor];
        self.cursor += 1;
        Some(candidate)
    }

    fn contains(&self, candidate: Candidate) -> bool {
        self.ready_queue.contains(&candidate)
    }
    pub fn len(&self) -> usize {
        self.ready_queue.len()
    }
}

static SCHEDULER: IrqSafeSpinLock<Option<Scheduler>> = IrqSafeSpinLock::new(None);

pub fn init() {
    *SCHEDULER.lock() = Some(Scheduler::new());
    crate::memory::serial_write(
        "[SCHED] UP round-robin init: timer-preemption=on fpu=fxsave smp=off\r\n",
    );
}

fn with_scheduler_mut<R>(f: impl FnOnce(&mut Scheduler) -> R) -> Option<R> {
    SCHEDULER.lock().as_mut().map(f)
}

fn with_scheduler<R>(f: impl FnOnce(&Scheduler) -> R) -> Option<R> {
    SCHEDULER.lock().as_ref().map(f)
}

pub fn enqueue_ready(pid: ProcessId) -> Result<(), ProcessError> {
    let _irq = InterruptGuard::new();
    // Queue -> process-table acquisition is forbidden. Ready state may become
    // stale later; dispatch validates it again before using a queue candidate.
    if !super::is_pid_runnable(pid) {
        return Err(ProcessError::NotRunnable);
    }
    with_scheduler_mut(|scheduler| scheduler.enqueue(pid))
        .map(|_| ())
        .ok_or(ProcessError::SchedulerUnavailable)
}

pub fn remove(pid: ProcessId) {
    let _ = with_scheduler_mut(|scheduler| scheduler.remove(pid));
}

fn pick_next(excluded: Option<ProcessId>) -> Option<ProcessId> {
    let _irq = InterruptGuard::new();
    // Bounded, allocation-free traversal. The IRQ guard preserves the current
    // UP transaction; generations protect queue identity across unlocked checks.
    let budget = ready_len();
    for _ in 0..budget {
        let candidate = with_scheduler_mut(|scheduler| scheduler.next_candidate()).flatten()?;
        let runnable = super::is_pid_runnable(candidate.pid);
        if !runnable {
            let _ = with_scheduler_mut(|scheduler| scheduler.remove_candidate(candidate));
        } else if Some(candidate.pid) != excluded
            && with_scheduler(|scheduler| scheduler.contains(candidate)).unwrap_or(false)
        {
            return Some(candidate.pid);
        }
    }
    None
}

pub fn pick_next_candidate() -> Option<ProcessId> {
    pick_next(None)
}

pub fn pick_next_candidate_excluding(pid: ProcessId) -> Option<ProcessId> {
    pick_next(Some(pid))
}

pub fn ready_len() -> usize {
    with_scheduler(|scheduler| scheduler.len()).unwrap_or(0)
}

/// A readiness observation, not an atomic SMP Ready -> Running claim.
pub fn has_ready_tasks() -> bool {
    let _irq = InterruptGuard::new();
    for index in 0..ready_len() {
        let candidate =
            with_scheduler(|scheduler| scheduler.ready_queue.get(index).copied()).flatten();
        if let Some(candidate) = candidate {
            if super::is_pid_runnable(candidate.pid)
                && with_scheduler(|scheduler| scheduler.contains(candidate)).unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}
