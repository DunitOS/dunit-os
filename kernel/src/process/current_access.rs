//! Bounded reads of the current process. Only owned results may leave the scope.
use super::Process;
use crate::sync::InterruptGuard;

/// The higher-ranked callback cannot return a reference borrowed from Process.
/// IRQ masking preserves the current UP read scope; cross-CPU exclusion/pinning
/// remains a separate process-registry gate before AP syscall dispatch.
#[inline]
pub fn with_current_process<R>(read: impl for<'a> FnOnce(&'a Process) -> R) -> Option<R> {
    let _irq = InterruptGuard::new();
    super::current_process().map(read)
}

/// Mutable access has the same non-escaping result contract as read access.
#[inline]
pub fn with_current_process_mut<R>(write: impl for<'a> FnOnce(&'a mut Process) -> R) -> Option<R> {
    let _irq = InterruptGuard::new();
    finish_write(super::current_process_mut().map(|process| {
        let result = write(process);
        (result, process.take_fd_wakeup())
    }))
}

/// Thread-local state belongs to the current thread, not its process owner.
#[inline]
pub fn with_current_thread_mut<R>(write: impl for<'a> FnOnce(&'a mut Process) -> R) -> Option<R> {
    let _irq = InterruptGuard::new();
    finish_write(super::current_thread_mut().map(|process| {
        let result = write(process);
        (result, process.take_fd_wakeup())
    }))
}

// The result contains owned data only, so no Process borrow survives rescan.
fn finish_write<R>(result: Option<(R, bool)>) -> Option<R> {
    result.map(|(value, wake)| {
        if wake {
            super::wake_wait_set_waiters();
        }
        value
    })
}
