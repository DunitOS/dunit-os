//! Owned cleanup batches: extraction borrows a Process, release never does.
use super::*;

#[derive(Default)]
pub(super) struct FdResources {
    entries: BTreeMap<ProcessFd, FdEntry>,
    refs: BTreeMap<crate::fs::vfs::FileDescriptor, u32>,
}

impl FdResources {
    pub(super) fn release(mut self) -> usize {
        let count = self.refs.len();
        self.clear();
        count
    }

    fn clear(&mut self) {
        for (_, entry) in core::mem::take(&mut self.entries) {
            if let FdTarget::Pipe { id, write } = entry.target {
                if crate::pipe::release(id, write) {
                    wake_pipe_waiters(id);
                }
            }
        }
        // One VFS reference per distinct description, irrespective of dup count.
        for (fd, _) in core::mem::take(&mut self.refs) {
            if let Some(mut vfs) = crate::fs::vfs::get_vfs() {
                let _ = vfs.close(fd);
            }
        }
    }

    fn inherited(entries: &[(u32, FdEntry)]) -> Result<Self, ProcessError> {
        let mut result = Self::default();
        for (fd, entry) in entries {
            if *fd >= MAX_PROCESS_FD || result.entries.contains_key(fd) {
                return Err(ProcessError::InvalidFd);
            }
            if let FdTarget::Vfs(vfs_fd) = entry.target {
                if !result.refs.contains_key(&vfs_fd) {
                    let retained = {
                        let mut vfs = crate::fs::vfs::get_vfs().ok_or(ProcessError::InvalidFd)?;
                        vfs.retain(vfs_fd)
                    };
                    retained.map_err(|_| ProcessError::InvalidFd)?;
                }
                *result.refs.entry(vfs_fd).or_insert(0) += 1;
            }
            if let FdTarget::Pipe { id, write } = entry.target {
                crate::pipe::retain(id, write);
            }
            let mut entry = entry.clone();
            entry.identity = NEXT_FD_IDENTITY.fetch_add(1, Ordering::Relaxed);
            result.entries.insert(*fd, entry);
        }
        Ok(result)
    }
}
impl Drop for FdResources {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Process table lookup/mutation is separate from VFS retention and rollback.
pub fn inherit_process_fds(pid: ProcessId, entries: &[(u32, FdEntry)]) -> Result<(), ProcessError> {
    let mut incoming = FdResources::inherited(entries)?;
    let outgoing = with_process_mut(pid, |child| {
        core::mem::swap(&mut child.fd_table, &mut incoming.entries);
        core::mem::swap(&mut child.vfs_fd_refs, &mut incoming.refs);
        Ok(incoming)
    })?;
    outgoing.release();
    Ok(())
}

pub(super) struct VmResources {
    pid: ProcessId,
    address_space: Option<AddressSpace>,
    pages: BTreeMap<usize, u64>,
    owned: Vec<u64>,
    handles: Vec<HandleObject>,
}
impl Drop for VmResources {
    fn drop(&mut self) {
        // Callers must switch away from the old address space before extraction.
        drop(self.address_space.take());
        crate::pty::on_process_exit(self.pid);
        for (_, id) in core::mem::take(&mut self.pages) {
            shared_vm_release(id, 1);
        }
        for id in core::mem::take(&mut self.owned) {
            shared_vm_release(id, 1);
        }
        for object in core::mem::take(&mut self.handles) {
            match object {
                HandleObject::Display => crate::handle::release_display(self.pid),
                HandleObject::Input => crate::handle::release_input(self.pid),
                HandleObject::SharedFrames { id } => shared_vm_release(id, 1),
                _ => {} // Owned file/endpoint Arcs close here, outside Process access.
            }
        }
    }
}
impl Process {
    pub(super) fn take_fd_resources(&mut self) -> FdResources {
        FdResources {
            entries: core::mem::take(&mut self.fd_table),
            refs: core::mem::take(&mut self.vfs_fd_refs),
        }
    }
    pub(super) fn take_vm_resources(&mut self) -> VmResources {
        self.guard_regions.clear();
        self.tls_allocation = None;
        VmResources {
            pid: self.pid,
            address_space: self.address_space.take(),
            pages: core::mem::take(&mut self.shared_pages),
            owned: core::mem::take(&mut self.shared_owned),
            handles: self.handle_table.drain_objects(),
        }
    }
}

#[cfg(feature = "boot-smoke-tests")]
pub fn run_resource_cleanup_smoke() -> bool {
    fn check() -> Result<(), ProcessError> {
        use crate::fs::vfs::{get_vfs, OpenFlags};
        let mut owner = Process::new_kernel(allocate_pid());
        let flags = OpenFlags::from_bits(OpenFlags::READ_WRITE.bits() | OpenFlags::CREATE.bits());
        let fd = get_vfs()
            .ok_or(ProcessError::InvalidFd)?
            .open_at("/", "/tmp/resource-cleanup", flags)
            .map_err(|_| ProcessError::InvalidFd)?;
        let pfd = owner.open_vfs_fd(fd)?;
        owner.dup_fd(pfd, None)?;
        // A failed partial inheritance must roll back its sole global retain,
        // without closing the original owner's description (including its dup).
        let invalid = [(3, FdEntry::vfs(fd)), (4, FdEntry::vfs(u32::MAX))];
        if FdResources::inherited(&invalid).is_ok() {
            return Err(ProcessError::InvalidFd);
        }
        if get_vfs().ok_or(ProcessError::InvalidFd)?.fstat(fd).is_err() {
            return Err(ProcessError::InvalidFd);
        }
        let inherited = FdResources::inherited(&owner.snapshot_fds())?;
        let detached = owner.take_fd_resources();
        if owner.fd_count() != 0 {
            return Err(ProcessError::InvalidFd);
        }
        // Dup descriptors release one global reference, not two.
        detached.release();
        if get_vfs().ok_or(ProcessError::InvalidFd)?.fstat(fd).is_err() {
            return Err(ProcessError::InvalidFd);
        }
        inherited.release();
        let mut vfs = get_vfs().ok_or(ProcessError::InvalidFd)?;
        if vfs.fstat(fd) != Err(crate::fs::vfs::VfsError::InvalidDescriptor) {
            return Err(ProcessError::InvalidFd);
        }
        vfs.remove_at("/", "/tmp/resource-cleanup")
            .map_err(|_| ProcessError::InvalidFd)?;
        drop(vfs);
        // Owner now contains no external resources; dropping it cannot reclose fd.
        drop(owner);
        let baseline = crate::fs::vfs::root_memfs_stats().open_handles;
        let child = create_user_process_record(String::from("[cleanup-smoke]"), true, false)?;
        let fd = get_vfs()
            .ok_or(ProcessError::InvalidFd)?
            .open_at("/", "/tmp/resource-kill", flags)
            .map_err(|_| ProcessError::InvalidFd)?;
        with_process_mut(child, |process| process.open_vfs_fd(fd))?;
        let cap = get_vfs()
            .ok_or(ProcessError::InvalidFd)?
            .open_capability("/", "/tmp/resource-kill", OpenFlags::READ)
            .map_err(|_| ProcessError::InvalidFd)?;
        with_process_mut(child, |process| {
            Ok(process.handle_create_file(cap, RIGHT_READ))
        })?;
        let pipe = crate::pipe::create();
        with_process_mut(child, |process| process.install_pipe_fd(pipe, true))?;
        if kill_process(child, 29)? {
            return Err(ProcessError::InvalidUserContext);
        }
        // Exit must release externally visible resources before the parent waits.
        let eof = matches!(
            crate::pipe::read(pipe, &mut [0u8; 1]),
            crate::pipe::PipeRead::Eof
        );
        crate::pipe::release(pipe, false);
        if !eof
            || get_vfs().ok_or(ProcessError::InvalidFd)?.fstat(fd).is_ok()
            || crate::fs::vfs::root_memfs_stats().open_handles != baseline
        {
            return Err(ProcessError::InvalidFd);
        }
        let waited = wait_for_child(child)?;
        if waited.code != 29 || process_exists(child) {
            return Err(ProcessError::NotChild);
        }
        get_vfs()
            .ok_or(ProcessError::InvalidFd)?
            .remove_at("/", "/tmp/resource-kill")
            .map_err(|_| ProcessError::InvalidFd)?;
        Ok(())
    }
    if check().is_ok() {
        crate::memory::serial_write(
            "[PROCESS-RESOURCES] detach, dup, rollback, kill EOF/cap close before wait OK\r\n",
        );
        true
    } else {
        crate::memory::serial_write("[PROCESS-RESOURCES] FAIL\r\n");
        false
    }
}
