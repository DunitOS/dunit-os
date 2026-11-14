//! Producer checkpoint, atomic registration and generation-checked wake.
use super::*;

fn capture_block(
    key: WaitKey,
    deadline: Option<u64>,
    restart: bool,
    checkpoint: Option<WaitCheckpoint>,
) -> Result<(), ProcessError> {
    let _irq = InterruptGuard::new();
    let entity = current_entity_id().ok_or(ProcessError::NoCurrentProcess)?;
    with_task_record(entity, |process, record| {
        if record.state != ProcessState::Running {
            return Err(ProcessError::NotRunnable);
        }
        if process.is_kernel {
            return Err(ProcessError::InvalidUserContext);
        }
        unsafe {
            if restart {
                crate::hal::syscall_restart_user_context(&mut process.context);
            } else {
                crate::hal::syscall_capture_user_context(&mut process.context, 0);
            }
        }
        let mut queue = WAIT_QUEUE.lock();
        let retry = checkpoint
            .map(|checkpoint| queue.changed(checkpoint))
            .unwrap_or(false)
            || deadline
                .map(|deadline| crate::clock::monotonic_ticks() >= deadline)
                .unwrap_or(false);
        record.state = if retry {
            ProcessState::Ready
        } else {
            ProcessState::Blocked
        };
        process.state = record.state;
        record.wait_generation = if retry {
            None
        } else {
            Some(queue.push(WaitEntry {
                entity,
                key,
                deadline,
            }))
        };
        Ok(())
    })?;
    crate::cpu::runtime()
        .yield_requested
        .store(true, Ordering::SeqCst);
    crate::cpu::runtime()
        .schedule_hint
        .store(0, Ordering::SeqCst);
    Ok(())
}
/// Snapshot before the producer predicate, then compare at registration.
/// A winning notification retries/returns without ever parking.
pub(crate) fn block_current(
    ipc: Option<ProcessId>,
    deadline: Option<u64>,
    checkpoint: Option<WaitCheckpoint>,
) -> Result<(), ProcessError> {
    capture_block(
        ipc.map(WaitKey::Ipc).unwrap_or(WaitKey::Timer),
        deadline,
        false,
        checkpoint,
    )
}
pub(crate) fn pipe_block_current(id: u64, checkpoint: WaitCheckpoint) -> Result<(), ProcessError> {
    capture_block(WaitKey::Pipe(id), None, true, Some(checkpoint))
}
pub(crate) fn poll_block_current(
    deadline_ns: u64,
    checkpoint: WaitCheckpoint,
) -> Result<(), ProcessError> {
    readiness_block(deadline_ns, WaitKey::Poll, checkpoint)
}
pub(crate) fn wait_set_block_current(
    deadline_ns: u64,
    checkpoint: WaitCheckpoint,
) -> Result<(), ProcessError> {
    readiness_block(deadline_ns, WaitKey::WaitSet, checkpoint)
}
fn readiness_block(
    deadline_ns: u64,
    key: WaitKey,
    checkpoint: WaitCheckpoint,
) -> Result<(), ProcessError> {
    let tick_ns = 1_000_000_000 / crate::clock::TICKS_PER_SECOND;
    let deadline =
        (deadline_ns != u64::MAX).then(|| deadline_ns.saturating_add(tick_ns - 1) / tick_ns);
    capture_block(key, deadline, true, Some(checkpoint))
}
pub(super) fn wake_entry(entry: WaitEntry, generation: u64) {
    let ready = with_task_record(entry.entity, |process, record| {
        if record.state != ProcessState::Blocked || record.wait_generation != Some(generation) {
            return Ok(false);
        }
        record.wait_generation = None;
        record.state = ProcessState::Ready;
        process.state = ProcessState::Ready;
        Ok(true)
    })
    .unwrap_or(false);
    if ready {
        let _ = scheduler::enqueue_ready(entry.entity);
    }
}
