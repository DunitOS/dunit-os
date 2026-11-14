//! Context/lifecycle publication. Task -> registry, then unlocked notifications.
use super::*;

fn make_ready(pid: ProcessId, reason: &str) -> Result<(), ProcessError> {
    let _irq = InterruptGuard::new();
    let from = with_task_record(pid, |process, record| {
        if matches!(record.state, ProcessState::Dead | ProcessState::Reaped) {
            return Err(ProcessError::ProcessNotPrepared);
        }
        let from = record.state;
        record.state = ProcessState::Ready;
        process.state = ProcessState::Ready;
        Ok(from)
    })?;
    if from != ProcessState::Ready {
        log_process_transition(pid, from, ProcessState::Ready, reason);
    }
    if scheduler::enqueue_ready(pid).is_err() {
        crate::memory::serial_write("[SCHED] enqueue rejected pid=");
        serial_write_u64(pid.0);
        crate::memory::serial_write("\r\n");
        return Err(ProcessError::NotRunnable);
    }
    Ok(())
}
pub fn mark_process_prepared_as_ready(pid: ProcessId) -> Result<(), ProcessError> {
    make_ready(pid, "prepare-ready")
}
pub fn mark_process_ready(pid: ProcessId) {
    let _ = make_ready(pid, "elf-ready");
}
pub fn save_current_user_context_for_yield(next: ProcessId) -> Result<ProcessId, ProcessError> {
    let _irq = InterruptGuard::new();
    let pid = current_entity_id().ok_or(ProcessError::NoCurrentProcess)?;
    with_task_record(pid, |process, record| {
        if record.state != ProcessState::Running {
            return Err(ProcessError::NotRunnable);
        }
        if process.is_kernel {
            return Err(ProcessError::InvalidUserContext);
        }
        unsafe {
            crate::hal::syscall_capture_user_context(&mut process.context, 0);
        }
        record.state = ProcessState::Ready;
        process.state = ProcessState::Ready;
        Ok(())
    })?;
    crate::cpu::runtime()
        .yield_requested
        .store(true, Ordering::SeqCst);
    crate::cpu::runtime()
        .schedule_hint
        .store(next.0, Ordering::SeqCst);
    // The dispatch lease publishes queue admission after HAL/FPU return.
    Ok(pid)
}

fn capture_block(key: WaitKey, deadline: Option<u64>, restart: bool) -> Result<(), ProcessError> {
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
        record.state = ProcessState::Blocked;
        process.state = ProcessState::Blocked;
        WAIT_QUEUE.lock().push(WaitEntry {
            entity,
            key,
            deadline,
        });
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
/// Predicate and registration still rely on the UP syscall/IRQ transaction.
pub fn block_current(ipc: Option<ProcessId>, deadline: Option<u64>) -> Result<(), ProcessError> {
    capture_block(
        ipc.map(WaitKey::Ipc).unwrap_or(WaitKey::Timer),
        deadline,
        false,
    )
}
pub fn pipe_block_current(id: u64) -> Result<(), ProcessError> {
    capture_block(WaitKey::Pipe(id), None, true)
}
pub fn poll_block_current(deadline_ns: u64) -> Result<(), ProcessError> {
    readiness_block(deadline_ns, WaitKey::Poll)
}
pub(crate) fn wait_set_block_current(deadline_ns: u64) -> Result<(), ProcessError> {
    readiness_block(deadline_ns, WaitKey::WaitSet)
}
fn readiness_block(deadline_ns: u64, key: WaitKey) -> Result<(), ProcessError> {
    let tick_ns = 1_000_000_000 / crate::clock::TICKS_PER_SECOND;
    let deadline =
        (deadline_ns != u64::MAX).then(|| deadline_ns.saturating_add(tick_ns - 1) / tick_ns);
    capture_block(key, deadline, true)
}
pub(super) fn wake_entry(entry: WaitEntry) {
    let ready = with_task_record(entry.entity, |process, record| {
        if record.state != ProcessState::Blocked {
            return Ok(false);
        }
        record.state = ProcessState::Ready;
        process.state = ProcessState::Ready;
        Ok(true)
    })
    .unwrap_or(false);
    if ready {
        let _ = scheduler::enqueue_ready(entry.entity);
    }
}
pub fn mark_process_finished(exit: ProcessExit) {
    let _irq = InterruptGuard::new();
    remove_waiter(exit.pid);
    let from = with_task_record(exit.pid, |process, record| {
        let from = record.state;
        record.state = ProcessState::Dead;
        record.status = Some(exit.status);
        record.has_run = true;
        process.state = ProcessState::Dead;
        process.status = Some(exit.status);
        Ok(from)
    })
    .ok();
    scheduler::remove(exit.pid);
    if let Some(from) = from {
        if from != ProcessState::Dead {
            log_process_transition(exit.pid, from, ProcessState::Dead, "finished");
        }
    }
    wake_wait_set_waiters();
}

pub fn timer_preempt_save_and_schedule(frame: &crate::interrupts::InterruptFrame) {
    if !PREEMPTION_ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let _irq = InterruptGuard::new();
    let Some(current) = current_entity_id() else {
        return;
    };
    if !with_task_record(current, |process, record| {
        Ok(record.state == ProcessState::Running && !process.is_kernel)
    })
    .unwrap_or(false)
    {
        return;
    }
    let Some(next) = scheduler::pick_next_candidate_excluding(current) else {
        return;
    };
    if with_task_record(current, |process, record| {
        if record.state != ProcessState::Running || process.is_kernel {
            return Err(ProcessError::NotRunnable);
        }
        process.context = CpuContext {
            rax: frame.rax,
            rbx: frame.rbx,
            rcx: frame.rcx,
            rdx: frame.rdx,
            rsi: frame.rsi,
            rdi: frame.rdi,
            rbp: frame.rbp,
            rsp: frame.rsp,
            r8: frame.r8,
            r9: frame.r9,
            r10: frame.r10,
            r11: frame.r11,
            r12: frame.r12,
            r13: frame.r13,
            r14: frame.r14,
            r15: frame.r15,
            rip: frame.rip,
            rflags: frame.rflags,
        };
        record.state = ProcessState::Ready;
        process.state = ProcessState::Ready;
        Ok(())
    })
    .is_err()
    {
        return;
    }
    log_process_transition(
        current,
        ProcessState::Running,
        ProcessState::Ready,
        "preempt",
    );
    let _ = scheduler::enqueue_ready(current);
    PREEMPTION_COUNT.fetch_add(1, Ordering::SeqCst);
    crate::cpu::runtime()
        .yield_requested
        .store(true, Ordering::SeqCst);
    crate::cpu::runtime()
        .schedule_hint
        .store(next.0, Ordering::SeqCst);
    crate::cpu::runtime()
        .preempt_switch_requested
        .store(true, Ordering::SeqCst);
}
