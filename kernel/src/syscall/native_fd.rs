//! Native descriptor control and wait-queue-backed readiness. POSIX encodings
//! belong in the musl adapter; these records and flags are Dunit contracts.
use super::*;

const READ: u32 = 1;
const WRITE: u32 = 2;
const APPEND: u32 = 16;
const NONBLOCK: u32 = 64;
const POLL_READ: u16 = 1;
const POLL_WRITE: u16 = 2;
const POLL_HUP: u16 = 4;
const POLL_ERROR: u16 = 8;
const POLL_INVALID: u16 = 16;

/// Commands: 0 get descriptor flags, 1 set descriptor flags, 2 get status,
/// 3 set APPEND/NONBLOCK, 4 duplicate from minimum, 5 duplicate with CLOEXEC,
/// 6 atomic pipe creation (fd=flags, arg=pointer to two u32 descriptors).
pub(super) fn fcntl(fd: u32, command: u32, arg: u64) -> i64 {
    if command == 6 {
        return sys_pipe(arg as *mut u32, fd);
    }
    let entry = match crate::process::get_fd(fd) {
        Some(entry) => entry.clone(),
        None => return EBADF,
    };
    match command {
        0 => i64::from(entry.cloexec),
        1 => {
            if arg & !1 != 0 {
                return EINVAL;
            }
            crate::process::current_process_mut()
                .ok_or(EINVAL)
                .and_then(|process| {
                    process
                        .set_cloexec(fd, arg != 0)
                        .map_err(process_error_to_errno)
                })
                .map(|_| 0)
                .unwrap_or_else(|error| error)
        }
        2 => entry.status() as i64,
        3 => {
            if arg & !((READ | WRITE | APPEND | NONBLOCK) as u64) != 0 {
                return EOPNOTSUPP;
            }
            let status = (entry.status() & (READ | WRITE)) | (arg as u32 & (APPEND | NONBLOCK));
            if let crate::process::FdTarget::Vfs(vfs_fd) = entry.target {
                if let Err(error) = crate::fs::vfs::get_vfs().ok_or(EIO).and_then(|vfs| {
                    vfs.set_append(vfs_fd, status & APPEND != 0)
                        .map_err(vfs_error_to_errno)
                }) {
                    return error;
                }
            } else if status & APPEND != 0 {
                return EOPNOTSUPP;
            }
            entry.set_status(status);
            0
        }
        4 | 5 => {
            if arg >= crate::process::MAX_PROCESS_FD as u64 {
                return EINVAL;
            }
            crate::process::current_process_mut()
                .ok_or(EINVAL)
                .and_then(|process| {
                    process
                        .duplicate_from(fd, arg as u32, command == 5)
                        .map_err(process_error_to_errno)
                })
                .map(|fd| fd as i64)
                .unwrap_or_else(|error| error)
        }
        _ => EOPNOTSUPP,
    }
}

pub(super) fn readiness(fd: i32, events: u16) -> u16 {
    if fd < 0 {
        return 0;
    }
    let Some(entry) = crate::process::get_fd(fd as u32) else {
        return POLL_INVALID;
    };
    let mut result = 0;
    use crate::process::FdTarget;
    match entry.target {
        FdTarget::Vfs(_) => result |= events & (POLL_READ | POLL_WRITE),
        FdTarget::Stdout | FdTarget::Stderr => {
            let readiness = entry
                .terminal
                .as_ref()
                .and_then(|endpoint| crate::pty::slave_output_readiness(endpoint.id));
            match readiness {
                Some((writable, closed)) => {
                    if writable {
                        result |= events & POLL_WRITE;
                    }
                    if closed {
                        result |= POLL_ERROR | POLL_HUP;
                    }
                }
                None => result |= events & POLL_WRITE,
            }
        }
        FdTarget::Stdin => {
            if crate::process::current_process().is_some() {
                if let Some((readable, closed)) = entry
                    .terminal
                    .as_ref()
                    .and_then(|endpoint| crate::pty::slave_readiness(endpoint.id))
                {
                    if readable {
                        result |= events & POLL_READ;
                    }
                    if closed {
                        result |= POLL_HUP;
                    }
                } else if crate::process::terminal_input_ready() {
                    result |= events & POLL_READ;
                } else {
                    let _ = crate::process::request_terminal_stdin_for_current();
                }
            }
        }
        FdTarget::Pipe { id, write } => {
            let (ready, closed) = crate::pipe::readiness(id, write);
            if ready {
                result |= events & if write { POLL_WRITE } else { POLL_READ };
            }
            if closed {
                result |= if write { POLL_ERROR } else { POLL_HUP };
            }
        }
    }
    result
}

/// Array records: {fd:i32, requested:u16, returned:u16}, all little-endian.
/// deadline_ns: 0=snapshot, MAX=infinite; otherwise absolute MONOTONIC ns.
pub(super) fn poll(records: *mut u8, count: usize, deadline_ns: u64) -> i64 {
    if count > 1024 {
        return EINVAL;
    }
    let _guard = crate::sync::InterruptGuard::new();
    let length = count * 8;
    if let Err(error) = validate_user_mapping(records as u64, length, true) {
        return error;
    }
    let mut data = match copy_buffer_from_user(records, length) {
        Ok(data) => data,
        Err(error) => return error,
    };
    let mut ready = 0;
    for record in data.chunks_exact_mut(8) {
        let fd = i32::from_le_bytes(record[..4].try_into().unwrap());
        let events = u16::from_le_bytes(record[4..6].try_into().unwrap());
        if events & !(POLL_READ | POLL_WRITE) != 0 {
            return EINVAL;
        }
        let returned = readiness(fd, events);
        record[6..8].copy_from_slice(&returned.to_le_bytes());
        if returned != 0 {
            ready += 1;
        }
    }
    if ready != 0 || deadline_ns == 0 || crate::clock::monotonic_ns() >= deadline_ns {
        return match copy_buffer_to_user(records, &data) {
            Ok(()) => ready,
            Err(error) => error,
        };
    }
    match crate::process::poll_block_current(deadline_ns) {
        Ok(()) => USER_CONTEXT_RETURN_MAGIC,
        Err(error) => process_error_to_errno(error),
    }
}

/// Eager private file snapshot, preserving the shared open-description offset.
/// Pages after the final partial file page cannot be represented without
/// SIGBUS/demand paging, so such ranges explicitly fail rather than fake them.
pub(super) fn map_file(
    addr: usize,
    length: usize,
    prot: u32,
    flags: u32,
    fd: i64,
    offset: u64,
) -> i64 {
    if flags != 2 || offset & 4095 != 0 || offset > i64::MAX as u64 {
        return EINVAL;
    }
    if fd < 0 || fd >= crate::process::MAX_PROCESS_FD as i64 {
        return EBADF;
    }
    let entry = match crate::process::get_fd(fd as u32) {
        Some(entry) => entry.clone(),
        None => return EBADF,
    };
    if entry.status() & READ == 0 {
        return EACCES;
    }
    let vfs_fd = match entry.target {
        crate::process::FdTarget::Vfs(fd) => fd,
        _ => return EOPNOTSUPP,
    };
    let stat = match crate::fs::vfs::get_vfs()
        .ok_or(EIO)
        .and_then(|vfs| vfs.fstat(vfs_fd).map_err(vfs_error_to_errno))
    {
        Ok(stat) => stat,
        Err(error) => return error,
    };
    if stat.file_type != crate::fs::vfs::FileType::File {
        return EISDIR;
    }
    let rounded_size = match stat.size.checked_add(4095) {
        Some(size) => size & !4095,
        None => return EINVAL,
    };
    let end = match offset.checked_add(length as u64) {
        Some(end) => end,
        None => return EINVAL,
    };
    if offset >= stat.size as u64 || end > rounded_size as u64 {
        return EINVAL;
    }
    let rounded_length = match length.checked_add(4095) {
        Some(size) => size & !4095,
        None => return EINVAL,
    };
    let mapped = match crate::process::current_process_mut() {
        Some(process) => match process.map_anonymous(addr, length, true, false, false, true) {
            Ok(mapped) => mapped,
            Err(error) => return process_error_to_errno(error),
        },
        None => return EINVAL,
    };
    let result = (|| -> Result<(), i64> {
        let available = (stat.size as u64 - offset).min(length as u64) as usize;
        let mut done = 0;
        let mut scratch = alloc::vec![0u8; available.min(16 * 1024)];
        while done < available {
            let want = scratch.len().min(available - done);
            let read = crate::fs::vfs::get_vfs()
                .ok_or(EIO)?
                .read_at(vfs_fd, offset + done as u64, &mut scratch[..want])
                .map_err(vfs_error_to_errno)?;
            if read == 0 {
                break;
            }
            copy_buffer_to_user((mapped + done) as *mut u8, &scratch[..read])?;
            done += read;
        }
        crate::process::current_process_mut()
            .ok_or(EINVAL)?
            .protect_range(
                mapped,
                rounded_length,
                prot & 2 != 0,
                prot & 4 != 0,
                prot != 0,
            )
            .map_err(process_error_to_errno)?;
        Ok(())
    })();
    match result {
        Ok(()) => mapped as i64,
        Err(error) => {
            if let Some(process) = crate::process::current_process_mut() {
                let _ = process.unmap_range(mapped, rounded_length);
            }
            error
        }
    }
}
