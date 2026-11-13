//! Registry membership and task data have separate ownership/exclusion.
use super::{Process, ProcessError, ProcessId, ProcessRecord};
use crate::sync::{InterruptGuard, IrqSafeSpinLock, IrqSafeSpinLockGuard};
use alloc::sync::Arc;
use alloc::vec::Vec;

pub(super) type Task = Arc<IrqSafeSpinLock<Process>>;
static RECORDS: IrqSafeSpinLock<Vec<ProcessRecord>> = IrqSafeSpinLock::new(Vec::new());

// Metadata only. Never acquire task/AddressSpace/VFS locks or run destructors
// under this guard. Remove/replace returns owned retired data to drop outside.
pub(super) fn table() -> IrqSafeSpinLockGuard<'static, Vec<ProcessRecord>> {
    RECORDS.lock()
}

pub(super) fn task(pid: ProcessId) -> Result<Task, ProcessError> {
    let records = table();
    let record = records
        .iter()
        .find(|record| record.pid == pid)
        .ok_or(ProcessError::NoSuchProcess)?;
    record
        .process
        .clone()
        .ok_or(ProcessError::ProcessNotPrepared)
}

pub(super) fn verify(record: &ProcessRecord, expected: &Task) -> Result<(), ProcessError> {
    match record.process.as_ref() {
        Some(actual) if Arc::ptr_eq(actual, expected) => Ok(()),
        _ => Err(ProcessError::ProcessNotPrepared),
    }
}

/// A copied lease may have been detached/replaced before its mutex was acquired.
/// Call while holding the task lock; it never acquires another task lock.
pub(super) fn validate(pid: ProcessId, expected: &Task) -> Result<(), ProcessError> {
    let records = table();
    let record = records
        .iter()
        .find(|record| record.pid == pid)
        .ok_or(ProcessError::NoSuchProcess)?;
    verify(record, expected)
}

/// Short state/context transactions only, never callbacks, I/O or notification.
/// Lock order task -> registry; no reference/guard may escape through R.
pub(super) fn with_record<R>(
    pid: ProcessId,
    update: impl for<'a, 'b> FnOnce(&'a mut Process, &'b mut ProcessRecord) -> Result<R, ProcessError>,
) -> Result<R, ProcessError> {
    let _irq = InterruptGuard::new();
    let task = task(pid)?;
    let mut process = task.lock();
    let mut records = table();
    let record = records
        .iter_mut()
        .find(|record| record.pid == pid)
        .ok_or(ProcessError::NoSuchProcess)?;
    verify(record, &task)?;
    update(&mut process, record)
}
