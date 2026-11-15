//! User execution loop and HAL return requests.
use super::*;

pub fn exit_current_process(code: i32) -> Option<ProcessId> {
    with_current_process_mut(|process| {
        process.exit(code);
        process.pid
    })
}

pub fn request_current_user_exit(code: i32) -> Option<ProcessId> {
    let pid = with_current_process(|process| (!process.is_kernel).then_some(process.pid)).flatten()?;
    crate::cpu::runtime().exit_code.store(code, Ordering::SeqCst);
    crate::cpu::runtime().exit_kind.store(0, Ordering::SeqCst);
    crate::cpu::runtime().exit_requested.store(true, Ordering::SeqCst);
    Some(pid)
}

pub fn request_current_user_fault(fault: ProcessFault) -> Option<ProcessId> {
    let pid = with_current_process(|process| (!process.is_kernel).then_some(process.pid)).flatten()?;
    crate::cpu::runtime().exit_code.store(fault.exit_code(), Ordering::SeqCst);
    crate::cpu::runtime().exit_kind.store(fault_kind_to_i32(fault), Ordering::SeqCst);
    crate::cpu::runtime().exit_requested.store(true, Ordering::SeqCst);
    Some(pid)
}

pub fn user_fault_escape_requested() -> bool {
    crate::cpu::runtime().exit_requested.load(Ordering::SeqCst) && crate::cpu::runtime().exit_kind.load(Ordering::SeqCst) != 0
}

pub fn preempt_switch_requested() -> bool {
    crate::cpu::runtime().preempt_switch_requested.load(Ordering::SeqCst)
}

pub fn clear_preempt_switch() {
    crate::cpu::runtime().preempt_switch_requested.store(false, Ordering::SeqCst);
}

/// Runtime switch for diagnostics; normal operation is preemptive.
pub fn set_preemption_enabled(enabled: bool) {
    PREEMPTION_ENABLED.store(enabled, Ordering::SeqCst);
}

pub fn preemption_enabled() -> bool {
    PREEMPTION_ENABLED.load(Ordering::SeqCst)
}

/// Number of timer-driven context switches committed so far.
pub fn preemption_count() -> u64 {
    PREEMPTION_COUNT.load(Ordering::SeqCst)
}

/// Reset the preemption counter (used to bracket a smoke-test window).
pub fn reset_preemption_count() {
    PREEMPTION_COUNT.store(0, Ordering::SeqCst);
}

pub(super) fn take_process_exit_request() -> Option<(i32, ProcessExitStatus)> {
    if crate::cpu::runtime().exit_requested.swap(false, Ordering::SeqCst) {
        let code = crate::cpu::runtime().exit_code.load(Ordering::SeqCst);
        let kind = crate::cpu::runtime().exit_kind.swap(0, Ordering::SeqCst);
        let status = match kind {
            0 => ProcessExitStatus::Exited(code),
            value => ProcessExitStatus::Fault(fault_kind_from_i32(value)),
        };
        Some((code, status))
    } else {
        None
    }
}

pub(super) fn take_process_yield_request() -> bool {
    crate::cpu::runtime().yield_requested.swap(false, Ordering::SeqCst)
}

pub(super) fn take_schedule_hint() -> Option<ProcessId> {
    let pid = crate::cpu::runtime().schedule_hint.swap(0, Ordering::SeqCst);
    if pid == 0 {
        None
    } else {
        Some(ProcessId(pid))
    }
}

pub fn enter_user_process(pid: ProcessId) -> Result<ProcessExit, ProcessError> {
    let mut scope = dispatch::DispatchScope::new();
    let root_pid = pid;
    let mut next_pid = pid;
    loop {
        if is_pid_blocked(next_pid) {
            next_pid = crate::process::scheduler::pick_next_candidate()
                .ok_or(ProcessError::SchedulerUnavailable)?;
        }
        let mut frame = dispatch::prepare(next_pid)?;
        crate::cpu::runtime().current_pid.store(next_pid.0, Ordering::SeqCst);

        unsafe { frame.run(&mut scope)?; }

        #[cfg(feature = "boot-smoke-tests")]
        fault_irq_smoke::verify();

        if take_process_yield_request() {
        } else {
            let mut status = ProcessExitStatus::Fault(ProcessFault::Unknown);
            let is_thread = owner_of(next_pid) != Some(next_pid);
            let detached_fds = with_process_mut(next_pid, |finished| {
                status = if let Some((_code, requested_status)) = take_process_exit_request() {
                    match requested_status {
                        ProcessExitStatus::Exited(code) => finished.exit(code),
                        ProcessExitStatus::Fault(fault) => finished.fault(fault),
                    }
                    requested_status
                } else if finished.state != ProcessState::Dead {
                    finished.fault(ProcessFault::Unknown);
                    ProcessExitStatus::Fault(ProcessFault::Unknown)
                } else {
                    finished
                        .status
                        .unwrap_or(ProcessExitStatus::Fault(ProcessFault::Unknown))
                };
                Ok(if is_thread { None } else { Some(finished.take_fd_resources()) })
            })?;
            let closed = detached_fds.map(|fds| fds.release()).unwrap_or(0);
            if closed > 0 {
                crate::memory::serial_write("[PROCESS-RUN] cleaned fds=");
                serial_write_usize(closed);
                crate::memory::serial_write("\r\n");
            }
            let exit = ProcessExit {
                pid: next_pid,
                status,
            };
            mark_process_finished(exit);
            if is_thread {
                release_finished_thread(next_pid)?;
            } else {
                cleanup_owned_threads(next_pid)?;
                scope.restore_root();
                let resources = with_process_mut(next_pid, |finished| Ok(finished.take_vm_resources()))?;
                drop(resources);
            }
            let reaped_children = if is_thread {
                0
            } else {
                cleanup_prepared_children(next_pid)
            };
            if reaped_children > 0 {
                crate::memory::serial_write("[PROCESS-RUN] reaped prepared children=");
                serial_write_usize(reaped_children);
                crate::memory::serial_write("\r\n");
            }
            if next_pid == root_pid {
                return Ok(exit);
            }
        }

        drop(frame); // release execution only after HAL return and exit bookkeeping

        let hinted = take_schedule_hint()
            .filter(|candidate| *candidate != next_pid && is_pid_runnable(*candidate));
        let preferred_root =
            (next_pid != root_pid && is_pid_runnable(root_pid)).then_some(root_pid);
        match hinted
            .or(preferred_root)
            .or_else(|| crate::process::scheduler::pick_next_candidate_excluding(next_pid))
        {
            Some(candidate) => {
                next_pid = candidate;
            }
            None => {
                if next_pid == root_pid || is_pid_blocked(root_pid) {
                    return Err(ProcessError::SchedulerUnavailable);
                }
                next_pid = root_pid;
            }
        }
    }
}



const fn fault_kind_from_i32(value: i32) -> ProcessFault {
    match value {
        1 => ProcessFault::PageFault,
        2 => ProcessFault::GeneralProtection,
        3 => ProcessFault::InvalidOpcode,
        4 => ProcessFault::DivideByZero,
        _ => ProcessFault::Unknown,
    }
}
