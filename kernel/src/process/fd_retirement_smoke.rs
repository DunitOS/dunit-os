//! Real retirement/reentry checks in the existing native boot-smoke profile.
use super::*;

fn arm(child: ProcessId) -> Result<(), ProcessError> {
    with_task_record(child, |process, record| {
        record.state = ProcessState::Blocked;
        process.state = ProcessState::Blocked;
        record.wait_generation = Some(WAIT_QUEUE.lock().push(WaitEntry {
            entity: child, key: WaitKey::WaitSet, deadline: None,
        }));
        Ok(())
    })
}

pub(super) fn check() -> Result<(), ProcessError> {
    use crate::fs::vfs::{get_vfs, OpenFlags};
    let master = current_pid().ok_or(ProcessError::NoCurrentProcess)?;
    let child = create_user_process_record(String::from("[registry-smoke]"), true, false)?;

    // Final endpoint drop executes the actual wake path against a blocked task.
    // A destructor inside its mutation would reacquire the same task mutex.
    let baseline_channels = crate::pty::channel_count();
    for case in 0..6 {
        let id = crate::pty::create(master);
        let endpoint = crate::pty::attach_slave(id, master).ok_or(ProcessError::InvalidFd)?;
        let weak = Arc::downgrade(&endpoint);
        if !crate::pty::close(id, master) { return Err(ProcessError::InvalidFd); }
        arm(child)?;
        with_process_mut(child, |process| {
            let mut entry = FdEntry::stdin();
            entry.terminal = Some(endpoint);
            match case {
                0 => { let fd = process.allocate_fd(entry)?; process.drop_fd(fd)?; }
                1 => { let fd = process.allocate_fd(entry)?; process.dup_fd(1, Some(fd))?; }
                2 => { let fd = process.allocate_fd(entry)?; process.install_fd(fd, FdEntry::stdout())?; }
                3 => {
                    if !matches!(process.install_fd(MAX_PROCESS_FD, entry), Err(ProcessError::InvalidFd)) {
                        return Err(ProcessError::InvalidFd);
                    }
                }
                4 => {
                    if !matches!(process.allocate_fd_from(entry, MAX_PROCESS_FD), Err(ProcessError::InvalidFd)) {
                        return Err(ProcessError::InvalidFd);
                    }
                }
                5 => {
                    for fd in 0..MAX_PROCESS_FD {
                        process.fd_table.entry(fd).or_insert_with(FdEntry::stdout);
                    }
                    if !matches!(process.allocate_fd(entry), Err(ProcessError::FdTableFull)) {
                        return Err(ProcessError::InvalidFd);
                    }
                }
                _ => unreachable!(),
            }
            if weak.upgrade().is_none() { return Err(ProcessError::InvalidFd); }
            Ok(())
        })?;
        if weak.upgrade().is_some() || is_pid_blocked(child) || crate::pty::channel_count() != baseline_channels {
            return Err(ProcessError::InvalidFd);
        }
    }

    // Ring transitions must notify real blocked tasks after releasing PTY_TABLE.
    let id = crate::pty::create(master);
    let endpoint = crate::pty::attach_slave(id, master).ok_or(ProcessError::InvalidFd)?;
    arm(child)?;
    if crate::pty::master_write(id, master, b"input") != Some(5) || is_pid_blocked(child) {
        return Err(ProcessError::InvalidFd);
    }
    arm(child)?;
    let mut input = [0; 8];
    if !matches!(crate::pty::slave_read_stdin(&endpoint, &mut input), Some(crate::pty::SlaveRead::Data(5)))
        || &input[..5] != b"input" || is_pid_blocked(child) {
        return Err(ProcessError::InvalidFd);
    }
    arm(child)?;
    if crate::pty::slave_write_stdout(&endpoint, b"output") != Some(Ok(6)) || is_pid_blocked(child) {
        return Err(ProcessError::InvalidFd);
    }
    arm(child)?;
    if crate::pty::master_read(id, master, &mut input) != Some((6, false))
        || &input[..6] != b"output" || is_pid_blocked(child) {
        return Err(ProcessError::InvalidFd);
    }
    arm(child)?;
    if !crate::pty::close(id, master) || is_pid_blocked(child)
        || !matches!(crate::pty::slave_read_stdin(&endpoint, &mut input), Some(crate::pty::SlaveRead::Eof)) {
        return Err(ProcessError::InvalidFd);
    }
    drop(endpoint);
    if crate::pty::channel_count() != baseline_channels { return Err(ProcessError::InvalidFd); }
    crate::memory::serial_write("[PTY-LOCK] owning I/O, read/write wake, hangup and drain OK\r\n");

    // bind_terminal must defer its consumed argument even with no stdio slots.
    let entries = with_process_mut(child, |process| Ok(core::mem::take(&mut process.fd_table)))?;
    drop(entries);
    let id = crate::pty::create(master);
    let endpoint = crate::pty::attach_slave(id, master).ok_or(ProcessError::InvalidFd)?;
    let weak = Arc::downgrade(&endpoint);
    crate::pty::close(id, master);
    with_process_mut(child, |process| {
        process.bind_terminal(endpoint);
        if weak.upgrade().is_none() { return Err(ProcessError::InvalidFd); }
        Ok(())
    })?;
    if weak.upgrade().is_some() || crate::pty::channel_count() != baseline_channels {
        return Err(ProcessError::InvalidFd);
    }

    let flags = OpenFlags::from_bits(OpenFlags::READ_WRITE.bits() | OpenFlags::CREATE.bits());
    let fd = get_vfs().ok_or(ProcessError::InvalidFd)?.open_at("/", "/tmp/registry-retirement", flags)
        .map_err(|_| ProcessError::InvalidFd)?;
    get_vfs().ok_or(ProcessError::InvalidFd)?.close(fd).map_err(|_| ProcessError::InvalidFd)?;
    let baseline = crate::fs::vfs::root_memfs_stats().open_handles;
    let file = get_vfs().ok_or(ProcessError::InvalidFd)?
        .open_capability("/", "/tmp/registry-retirement", OpenFlags::READ)
        .map_err(|_| ProcessError::InvalidFd)?;
    let weak = Arc::downgrade(&file);
    // An error after close must still flush the retired capability.
    let result: Result<(), ProcessError> = with_process_mut(child, |process| {
        let handle = process.handle_create_file(file, RIGHT_READ);
        process.handle_close(handle).map_err(|_| ProcessError::InvalidFd)?;
        if weak.upgrade().is_none() { return Err(ProcessError::InvalidFd); }
        Err(ProcessError::NotRunnable)
    });
    if result != Err(ProcessError::NotRunnable) || weak.upgrade().is_some()
        || crate::fs::vfs::root_memfs_stats().open_handles != baseline {
        return Err(ProcessError::InvalidFd);
    }
    get_vfs().ok_or(ProcessError::InvalidFd)?.remove_at("/", "/tmp/registry-retirement")
        .map_err(|_| ProcessError::InvalidFd)?;
    kill_process(child, 0)?;
    wait_for_child(child)?;
    crate::memory::serial_write(
        "[FD-RETIREMENT] final PTY/cap drops, fd errors and wake reentry after unlock OK\r\n",
    );
    Ok(())
}
