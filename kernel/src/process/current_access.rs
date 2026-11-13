//! Non-escaping task scopes. No current selector returns a Process reference.
use super::{
    current_entity_id, current_pid, process_record_index, process_table_mut, wake_wait_set_waiters,
    Handle, HandleError, Process, ProcessError, ProcessId,
};
use crate::sync::InterruptGuard;

// HRTB binds all Process/field borrows to the callback. IRQ masking serializes
// current UP access; this is not registry/task exclusion on multiple CPUs.
fn with_process<R>(pid: ProcessId, read: impl for<'a> FnOnce(&'a Process) -> R) -> Option<R> {
    let _irq = InterruptGuard::new();
    let table = process_table_mut();
    let index = process_record_index(table, pid)?;
    table[index].process.as_deref().map(read)
}

/// Shared by current owner, current thread and explicit-PID mutation paths.
/// Flush fd readiness only after the Process/table borrow, even on callback Err.
pub fn with_process_mut<R>(
    pid: ProcessId,
    write: impl for<'a> FnOnce(&'a mut Process) -> Result<R, ProcessError>,
) -> Result<R, ProcessError> {
    let _irq = InterruptGuard::new();
    let (result, wake) = {
        let table = process_table_mut();
        let index = process_record_index(table, pid).ok_or(ProcessError::NoSuchProcess)?;
        let process = table[index]
            .process
            .as_deref_mut()
            .ok_or(ProcessError::ProcessNotPrepared)?;
        let result = write(process);
        (result, process.take_fd_wakeup())
    };
    if wake {
        wake_wait_set_waiters();
    }
    result
}

#[inline]
pub fn with_current_process<R>(read: impl for<'a> FnOnce(&'a Process) -> R) -> Option<R> {
    let _irq = InterruptGuard::new();
    with_process(current_pid()?, read)
}

#[inline]
pub fn with_current_process_mut<R>(write: impl for<'a> FnOnce(&'a mut Process) -> R) -> Option<R> {
    let _irq = InterruptGuard::new();
    with_process_mut(current_pid()?, |process| Ok(write(process))).ok()
}

#[inline]
pub(crate) fn with_current_thread<R>(read: impl for<'a> FnOnce(&'a Process) -> R) -> Option<R> {
    let _irq = InterruptGuard::new();
    with_process(current_entity_id()?, read)
}

#[inline]
pub fn with_current_thread_mut<R>(write: impl for<'a> FnOnce(&'a mut Process) -> R) -> Option<R> {
    let _irq = InterruptGuard::new();
    with_process_mut(current_entity_id()?, |thread| Ok(write(thread))).ok()
}

/// Copy authorization before mutating the target: self-delivery and cross-task
/// delivery use exactly the same path without nested Process borrows/locks.
/// Preserve the UP validation/delivery boundary; remote cancellation is separate.
pub(crate) fn signal_current_handle(handle: Handle, value: u64) -> Option<Result<(), HandleError>> {
    let _irq = InterruptGuard::new();
    let target = match with_current_process(|process| process.signal_target(handle))? {
        Ok(target) => target,
        Err(error) => return Some(Err(error)),
    };
    Some(
        with_process_mut(target, |process| {
            process.add_pending_signal(value);
            Ok(())
        })
        .map_err(|_| HandleError::NoSuchTarget),
    )
}
