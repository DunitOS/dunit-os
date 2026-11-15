//! Descriptor transactions; external owners retire after the task scope.
use super::*;

impl Process {
    /// Register a freshly opened VFS descriptor as a new process fd and count a
    /// reference to it.
    pub fn open_vfs_fd(
        &mut self,
        vfs_fd: crate::fs::vfs::FileDescriptor,
    ) -> Result<ProcessFd, ProcessError> {
        let pfd = self.allocate_fd(FdEntry::vfs(vfs_fd))?;
        *self.vfs_fd_refs.entry(vfs_fd).or_insert(0) += 1;
        Ok(pfd)
    }

    /// Install a pipe end as a new process fd. The pipe's end refcount is set by
    /// `crate::pipe::create` for the two initial fds, so this does not retain.
    pub fn install_pipe_fd(&mut self, id: u64, write: bool) -> Result<ProcessFd, ProcessError> {
        self.allocate_fd(FdEntry::pipe(id, write))
    }

    /// Drop a process fd, returning any follow-up the syscall layer must do
    /// (close a VFS file whose last reference went away, or wake a pipe end).
    pub fn drop_fd(&mut self, fd: ProcessFd) -> Result<FdCleanup, ProcessError> {
        let entry = self.fd_table.remove(&fd).ok_or(ProcessError::InvalidFd)?;
        self.fd_wakeup_pending = true;
        let cleanup = self.release_target(entry.target);
        self.retire_fd(entry);
        Ok(cleanup)
    }

    fn release_target(&mut self, target: FdTarget) -> FdCleanup {
        match target {
            FdTarget::Vfs(vfs_fd) => {
                if let Some(count) = self.vfs_fd_refs.get_mut(&vfs_fd) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        self.vfs_fd_refs.remove(&vfs_fd);
                        return FdCleanup::CloseVfs(vfs_fd);
                    }
                }
                FdCleanup::None
            }
            FdTarget::Pipe { id, write } => {
                if crate::pipe::release(id, write) {
                    FdCleanup::WakePipe(id)
                } else {
                    FdCleanup::None
                }
            }
            _ => FdCleanup::None,
        }
    }

    fn retain_target(&mut self, target: FdTarget) {
        match target {
            FdTarget::Vfs(vfs_fd) => {
                *self.vfs_fd_refs.entry(vfs_fd).or_insert(0) += 1;
            }
            FdTarget::Pipe { id, write } => crate::pipe::retain(id, write),
            _ => {}
        }
    }

    /// dup / dup2. `newfd = None` picks the lowest free fd (dup); `Some(nf)`
    /// targets exactly `nf` (dup2), releasing whatever it held first. Returns the
    /// new fd plus the cleanup for a displaced dup2 target.
    pub fn dup_fd(
        &mut self,
        oldfd: ProcessFd,
        newfd: Option<ProcessFd>,
    ) -> Result<(ProcessFd, FdCleanup), ProcessError> {
        let mut entry = self.fd_table.get(&oldfd).ok_or(ProcessError::InvalidFd)?.clone();
        match newfd {
            Some(nf) if nf == oldfd => Ok((nf, FdCleanup::None)),
            Some(nf) => {
                if nf >= MAX_PROCESS_FD { return Err(ProcessError::InvalidFd); }
                entry.cloexec = false;
                self.retain_target(entry.target);
                let cleanup = match self.fd_table.remove(&nf) {
                    Some(old) => {
                        let cleanup = self.release_target(old.target);
                        self.retire_fd(old);
                        cleanup
                    }
                    None => FdCleanup::None,
                };
                entry.identity = NEXT_FD_IDENTITY.fetch_add(1, Ordering::Relaxed);
                self.fd_table.insert(nf, entry);
                self.fd_wakeup_pending = true;
                Ok((nf, cleanup))
            }
            None => {
                entry.cloexec = false;
                let target = entry.target;
                let pfd = self.allocate_fd(entry)?;
                self.retain_target(target);
                Ok((pfd, FdCleanup::None))
            }
        }
    }

    pub fn allocate_fd(&mut self, entry: FdEntry) -> Result<ProcessFd, ProcessError> {
        self.allocate_fd_from(entry, 0)
    }
    pub fn allocate_fd_from(&mut self, mut entry: FdEntry, minimum: u32) -> Result<ProcessFd, ProcessError> {
        if minimum >= MAX_PROCESS_FD {
            self.retire_fd(entry);
            return Err(ProcessError::InvalidFd);
        }
        for fd in minimum..MAX_PROCESS_FD {
            if !self.fd_table.contains_key(&fd) {
                entry.identity = NEXT_FD_IDENTITY.fetch_add(1, Ordering::Relaxed);
                self.fd_table.insert(fd, entry);
                return Ok(fd);
            }
        }
        self.retire_fd(entry);
        Err(ProcessError::FdTableFull)
    }
    pub fn duplicate_from(&mut self, oldfd: u32, minimum: u32, cloexec: bool) -> Result<u32, ProcessError> {
        let mut entry = self.fd_table.get(&oldfd).ok_or(ProcessError::InvalidFd)?.clone();
        entry.cloexec = cloexec;
        let target = entry.target;
        let fd = self.allocate_fd_from(entry, minimum)?;
        self.retain_target(target);
        Ok(fd)
    }
    pub fn bind_terminal(&mut self, endpoint: Arc<crate::pty::Endpoint>) {
        for entry in self.fd_table.values_mut() {
            if matches!(entry.target, FdTarget::Stdin | FdTarget::Stdout | FdTarget::Stderr) {
                if let Some(old) = entry.terminal.replace(endpoint.clone()) {
                    self.retired_resources.endpoints.push(old);
                }
            }
        }
        // Also defer the argument: all stdio slots may have been closed, so no
        // installed descriptor necessarily owns it when this method returns.
        self.retired_resources.endpoints.push(endpoint);
    }
    pub fn snapshot_fds(&self) -> Vec<(u32, FdEntry)> {
        self.fd_table.iter().map(|(&fd, entry)| (fd, entry.clone())).collect()
    }
    pub fn set_cloexec(&mut self, fd: u32, value: bool) -> Result<(), ProcessError> {
        self.fd_table.get_mut(&fd).ok_or(ProcessError::InvalidFd)?.cloexec = value;
        Ok(())
    }
    pub fn install_fd(&mut self, fd: u32, mut entry: FdEntry) -> Result<FdCleanup, ProcessError> {
        if fd >= MAX_PROCESS_FD {
            self.retire_fd(entry);
            return Err(ProcessError::InvalidFd);
        }
        self.retain_target(entry.target);
        let cleanup = match self.fd_table.remove(&fd) {
            Some(old) => {
                let cleanup = self.release_target(old.target);
                self.retire_fd(old);
                cleanup
            }
            None => FdCleanup::None,
        };
        entry.identity = NEXT_FD_IDENTITY.fetch_add(1, Ordering::Relaxed);
        self.fd_table.insert(fd, entry);
        self.fd_wakeup_pending = true;
        Ok(cleanup)
    }
    /// Adds a freshly opened VFS object at an exact descriptor, consuming its
    /// existing global reference (unlike an inherited reference).
    pub fn place_open_vfs_fd(&mut self, fd: u32, entry: FdEntry) -> Result<FdCleanup, ProcessError> {
        self.install_fd(fd, entry)
    }
    pub fn get_fd(&self, fd: ProcessFd) -> Option<&FdEntry> {
        self.fd_table.get(&fd)
    }

    pub fn close_fd(&mut self, fd: ProcessFd) -> Result<FdEntry, ProcessError> {
        let entry = self.fd_table.remove(&fd).ok_or(ProcessError::InvalidFd)?;
        self.fd_wakeup_pending = true;
        Ok(entry)
    }

}
