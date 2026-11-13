//! Real registry/task retirement and dispatcher leases, without synthetic IRET.
use super::*;

pub fn run_registry_ownership_smoke() -> bool {
    fn check() -> Result<(), ProcessError> {
        use crate::fs::vfs::{get_vfs, OpenFlags, VfsError};
        let before = get_vfs().ok_or(ProcessError::InvalidFd)?.open_file_count();
        let fd = get_vfs()
            .ok_or(ProcessError::InvalidFd)?
            .open_at(
                "/",
                "/tmp/registry-pin",
                OpenFlags::from_bits(OpenFlags::READ_WRITE.bits() | OpenFlags::CREATE.bits()),
            )
            .map_err(|_| ProcessError::InvalidFd)?;
        let pid = allocate_pid();
        let mut process = Process::new_user(pid)?;
        let local_fd = process.open_vfs_fd(fd)?;
        let weak_fpu = Arc::downgrade(&process.fpu_state);
        insert_process_record(
            pid,
            None,
            String::from("[registry-pin]"),
            ProcessState::Prepared,
            false,
            false,
            Some(process),
        );
        let weak = Arc::downgrade(&registry::task(pid)?);
        with_process_mut(pid, |process| {
            // These acquire registry metadata while the task mutex is held.
            let _ = process_stats();
            let _ = get_process_snapshots();
            autoreap_process(pid, "registry-pin-smoke")?;
            if process_exists(pid) || weak.upgrade().is_none() || process.get_fd(local_fd).is_none()
            {
                return Err(ProcessError::NoSuchProcess);
            }
            process.add_pending_signal(17);
            if process.handle_take_signals() != 17 {
                return Err(ProcessError::InvalidUserContext);
            }
            Ok(())
        })?;
        if weak.upgrade().is_some() || weak_fpu.upgrade().is_some() {
            return Err(ProcessError::InvalidUserContext);
        }
        {
            let mut vfs = get_vfs().ok_or(ProcessError::InvalidFd)?;
            if vfs.open_file_count() != before || vfs.fstat(fd) != Err(VfsError::InvalidDescriptor)
            {
                return Err(ProcessError::InvalidFd);
            }
            vfs.remove_at("/", "/tmp/registry-pin")
                .map_err(|_| ProcessError::InvalidFd)?;
        }
        // A pending HAL frame pins the whole task, in addition to stack/FPU/AS.
        let pid = allocate_pid();
        let mut process = Process::new_user(pid)?;
        process.map_anonymous(0, USER_PAGE_SIZE, true, false, false, true)?;
        process.context.rip = 0x200000;
        process.context.rsp = 0x7fffffffe000;
        let weak_space = Arc::downgrade(
            process
                .address_space
                .as_ref()
                .ok_or(ProcessError::NoAddressSpace)?,
        );
        insert_process_record(
            pid,
            None,
            String::from("[registry-dispatch]"),
            ProcessState::Ready,
            false,
            false,
            Some(process),
        );
        let weak = Arc::downgrade(&registry::task(pid)?);
        let frame = dispatch::prepare(pid)?;
        autoreap_process(pid, "registry-dispatch-smoke")?;
        if weak.upgrade().is_none() || weak_space.upgrade().is_none() {
            return Err(ProcessError::NoSuchProcess);
        }
        drop(frame);
        if weak.upgrade().is_some() || weak_space.upgrade().is_some() {
            return Err(ProcessError::InvalidUserContext);
        }
        Ok(())
    }
    if check().is_ok() {
        crate::memory::serial_write(
            "[TASK-REGISTRY] callback pin, detached cleanup, dispatch pin and reentry OK\r\n",
        );
        true
    } else {
        crate::memory::serial_write("[TASK-REGISTRY] FAIL\r\n");
        false
    }
}
