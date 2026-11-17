//! Close execution admission before cleanup; publish exit only after cleanup.
use super::*;

/// `returned` is valid only for the local dispatcher after HAL/FPU/TLS return.
/// External kill may retire unused reservations, but never an entered CPU.
pub(super) fn begin(exit: ProcessExit, returned: bool) -> Result<(), ProcessError> {
    let from = if returned {
        let task = registry::task(exit.pid)?;
        let _irq = InterruptGuard::new();
        let mut process = task.lock();
        let mut records = process_table();
        let index = process_record_index(&records, exit.pid).ok_or(ProcessError::NoSuchProcess)?;
        registry::verify(&records[index], &task)?;
        if let Some(owner) = records[index].owner {
            let owner_index =
                process_record_index(&records, owner).ok_or(ProcessError::NoSuchProcess)?;
            if matches!(
                records[owner_index].state,
                ProcessState::Dead | ProcessState::Reaped
            ) {
                return Err(ProcessError::NotRunnable);
            }
        } else if records
            .iter()
            .any(|record| record.owner == Some(exit.pid) && record.execution.retirement_blocked())
        {
            // A task Arc is not permission to destroy an active sibling's VM.
            return Err(ProcessError::NotRunnable);
        }
        close_admission(&mut process, &mut records[index], exit, true)?
    } else {
        registry::with_quiescent_group(exit.pid, |process, record| {
            close_admission(process, record, exit, false)
        })?
    };
    // Dead prevents new dispatch and stale wakes. The execution cleanup fence
    // also excludes wait/reap/replace until publish, even after lease drop.
    scheduler::remove(exit.pid);
    remove_waiter(exit.pid);
    log_process_transition(exit.pid, from, ProcessState::Dead, "finalize");
    Ok(())
}

fn close_admission(
    process: &mut Process,
    record: &mut ProcessRecord,
    exit: ProcessExit,
    returned: bool,
) -> Result<ProcessState, ProcessError> {
    if process.is_kernel {
        return Err(ProcessError::InvalidUserContext);
    }
    if matches!(record.state, ProcessState::Dead | ProcessState::Reaped) {
        return Err(ProcessError::NotRunnable);
    }
    if !returned && record.owner.is_some() {
        return Err(ProcessError::NoSuchProcess);
    }
    if !record
        .execution
        .begin_finalization(returned.then(crate::cpu::index))
    {
        return Err(ProcessError::NotRunnable);
    }
    let from = record.state;
    record.state = ProcessState::Dead;
    record.status = None;
    record.has_run = true;
    match exit.status {
        ProcessExitStatus::Exited(code) => process.exit(code),
        ProcessExitStatus::Fault(fault) => process.fault(fault),
    }
    Ok(from)
}

pub(super) fn publish(exit: ProcessExit) -> Result<(), ProcessError> {
    let retired = {
        let mut records = process_table();
        let index = process_record_index(&records, exit.pid).ok_or(ProcessError::NoSuchProcess)?;
        let record = &mut records[index];
        if record.state != ProcessState::Dead || !record.execution.complete_finalization() {
            return Err(ProcessError::NotRunnable);
        }
        record.status = Some(exit.status);
        if record.owner.is_some() && record.thread_auto_reap {
            Some(records.remove(index))
        } else {
            None
        }
    };
    // Detached status storage and any final task owner retire outside metadata.
    drop(retired);
    wake_wait_set_waiters();
    Ok(())
}
