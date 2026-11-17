//! Exercise the cleanup fence with real task/VM owners and child-wait paths.
use super::*;

pub(super) fn check() -> Result<(), ProcessError> {
    let pid = create_user_process_record(String::from("[exit-publication]"), true, false)?;
    with_process_mut(pid, |process| {
        let tls = process.map_anonymous(0, 3 * USER_PAGE_SIZE, true, false, false, true)?;
        process.unmap_range(tls + USER_PAGE_SIZE, USER_PAGE_SIZE)?;
        process.release_tls_allocation(tls, 3 * USER_PAGE_SIZE)?;
        process.release_tls_allocation(tls, 3 * USER_PAGE_SIZE)?;
        // The public syscall-oriented operation keeps its strict holes check.
        if process.unmap_range(tls, 3 * USER_PAGE_SIZE) != Err(ProcessError::InvalidMemoryRange) {
            return Err(ProcessError::InvalidMemoryRange);
        }
        // Admission checks require a prepared instruction/stack context. This
        // metadata probe never enters HAL, but still uses mapped user backing
        // rather than bypassing the scheduler's normal readiness validation.
        let entry = process.map_anonymous(0, USER_PAGE_SIZE, false, true, false, true)?;
        let stack = process.map_anonymous(0, USER_PAGE_SIZE, true, false, false, true)?;
        process.context.rip = entry as u64;
        process.context.rsp = (stack + USER_PAGE_SIZE) as u64;
        Ok(())
    })?;
    mark_process_prepared_as_ready(pid)?;
    let backing = process_address_space(pid)?;
    let weak = Arc::downgrade(&backing);
    let mut lease = with_task_record(pid, |_, record| execution::Lease::reserve(pid, record))?;
    lease.start()?;
    let exit = ProcessExit {
        pid,
        status: ProcessExitStatus::Exited(37),
    };
    if finalization::begin(exit, false) != Err(ProcessError::NotRunnable)
        || finalization::begin(exit, true) != Err(ProcessError::NotRunnable)
    {
        return Err(ProcessError::InvalidUserContext);
    }
    lease.returned();
    finalization::begin(exit, true)?;
    if child_exit_snapshot(pid)?.is_some()
        || wait_for_child(pid) != Err(ProcessError::NotRunnable)
        || autoreap_process(pid, "finalizing-probe") != Err(ProcessError::NotRunnable)
        || reap_process(pid) != Err(ProcessError::NotRunnable)
        || finalization::publish(exit) != Err(ProcessError::NotRunnable)
        || is_pid_runnable(pid)
    {
        return Err(ProcessError::InvalidUserContext);
    }
    let (fds, vm) = with_process_mut(pid, |process| {
        Ok((process.take_fd_resources(), process.take_vm_resources()))
    })?;
    fds.release();
    drop(vm);
    if weak.upgrade().is_none() {
        return Err(ProcessError::NoAddressSpace);
    }
    drop(backing);
    if weak.upgrade().is_some() {
        return Err(ProcessError::NoAddressSpace);
    }
    drop(lease);
    // Releasing the ticket alone must not publish status or permit reaping.
    if child_exit_snapshot(pid)?.is_some()
        || wait_for_child(pid) != Err(ProcessError::NotRunnable)
        || !execution::group_entered(&process_table(), pid)
    {
        return Err(ProcessError::InvalidUserContext);
    }
    finalization::publish(exit)?;
    if child_exit_snapshot(pid)?.map(|status| status.code) != Some(37)
        || wait_for_child(pid)?.code != 37
        || process_exists(pid)
    {
        return Err(ProcessError::InvalidUserContext);
    }
    crate::memory::serial_write(
        "[EXIT-PUBLICATION] wait/reap fenced through cleanup, VM pins and HAL lease return OK\r\n",
    );
    Ok(())
}
