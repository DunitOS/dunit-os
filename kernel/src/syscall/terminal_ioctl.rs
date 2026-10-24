//! Bounded native terminal records. No POSIX ioctl numbers enter the kernel.
use super::*;

/// Ioctl(fd, op, record, len): 0 probe; 1 get dimensions; 2 set dimensions;
/// 3 bytes of pending input. Dimensions are four little-endian u16 values.
pub(super) fn ioctl(fd: u32, op: u32, record: *mut u8, len: usize) -> i64 {
    let target = match crate::process::get_fd(fd) {
        Some(entry) => entry.target,
        None => return EBADF,
    };
    if !matches!(
        target,
        crate::process::FdTarget::Stdin
            | crate::process::FdTarget::Stdout
            | crate::process::FdTarget::Stderr
    ) {
        return -25; // ENOTTY: a valid descriptor that is not a terminal.
    }
    let pid = match crate::process::get_fd(fd).and_then(|entry| entry.terminal.as_ref()) {
        Some(endpoint) => endpoint.id,
        None => return -25,
    };
    match op {
        0 if len == 0 => 0,
        1 if len == 8 => {
            let dimensions = match crate::pty::slave_dimensions(pid) {
                Some(value) => value,
                None => return -25,
            };
            copy_buffer_to_user(record, &dimensions).map_or_else(|error| error, |_| 0)
        }
        2 if len == 8 => {
            let bytes = match copy_buffer_from_user(record, len) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let mut dimensions = [0u8; 8];
            dimensions.copy_from_slice(&bytes);
            if crate::pty::set_slave_dimensions(pid, dimensions) {
                0
            } else {
                -25
            }
        }
        3 if len == 4 => {
            let count = crate::pty::slave_pending_input(pid).unwrap_or(0) as u32;
            copy_buffer_to_user(record, &count.to_le_bytes()).map_or_else(|error| error, |_| 0)
        }
        0..=3 => EINVAL,
        _ => EOPNOTSUPP,
    }
}
