//! Native admission checks; real HAL entry is covered by app/FPU/fault scenes.
use super::*;
use execution::Lease;

pub fn run_execution_ownership_smoke() -> bool {
    fn ready(pid: ProcessId) -> Result<(), ProcessError> {
        let mut process = Process::new_user(pid)?;
        process.map_anonymous(0, USER_PAGE_SIZE, true, false, false, true)?;
        process.context.rip = 0x200000;
        process.context.rsp = 0x7fffffffe000;
        insert_process_record(
            pid,
            None,
            String::from("[execution-admission]"),
            ProcessState::Ready,
            false,
            false,
            Some(process),
        );
        Ok(())
    }
    fn reserve(pid: ProcessId) -> Result<Lease, ProcessError> {
        with_task_record(pid, |_, record| Lease::reserve(pid, record))
    }
    fn check() -> Result<(), ProcessError> {
        let pid = allocate_pid();
        ready(pid)?;
        let mut lease = reserve(pid)?;
        if is_pid_runnable(pid) || reserve(pid).is_ok() {
            return Err(ProcessError::NotRunnable);
        }
        lease.start()?;
        if autoreap_process(pid, "active-execution-smoke") != Err(ProcessError::NotRunnable)
            || kill_process(pid, 19) != Err(ProcessError::NotRunnable)
            || !with_process_mut(pid, |p| Ok(p.address_space.is_some()))?
        {
            return Err(ProcessError::InvalidUserContext);
        }
        with_task_record(pid, |process, record| {
            process.state = ProcessState::Blocked;
            record.state = ProcessState::Blocked;
            record.wait_generation = Some(WAIT_QUEUE.lock().push(WaitEntry {
                entity: pid,
                key: WaitKey::Timer,
                deadline: Some(0),
            }));
            Ok(())
        })?;
        // A producer wins the wake before the old CPU has completed return.
        wake_expired_waiters();
        if is_pid_runnable(pid) || reserve(pid).is_ok() {
            return Err(ProcessError::NotRunnable);
        }
        lease.returned();
        if is_pid_runnable(pid)
            || autoreap_process(pid, "return-execution-smoke") != Err(ProcessError::NotRunnable)
        {
            return Err(ProcessError::InvalidUserContext);
        }
        drop(lease);
        if !is_pid_runnable(pid) {
            return Err(ProcessError::NotRunnable);
        }
        let mut stale = reserve(pid)?;
        autoreap_process(pid, "reserved-execution-smoke")?;
        ready(pid)?; // same PID, new task Arc, and even the same first ticket
        let fresh = reserve(pid)?;
        if stale.start() != Err(ProcessError::ProcessNotPrepared) {
            return Err(ProcessError::InvalidUserContext);
        }
        drop(stale);
        if is_pid_runnable(pid) || reserve(pid).is_ok() {
            return Err(ProcessError::NotRunnable);
        }
        drop(fresh);
        if !is_pid_runnable(pid) {
            return Err(ProcessError::NotRunnable);
        }
        autoreap_process(pid, "execution-smoke-done")?;
        Ok(())
    }
    if check().is_ok() {
        crate::memory::serial_write(
            "[TASK-EXECUTION] exclusive admission, return handoff and retirement gates OK\r\n",
        );
        true
    } else {
        crate::memory::serial_write("[TASK-EXECUTION] FAIL\r\n");
        false
    }
}
