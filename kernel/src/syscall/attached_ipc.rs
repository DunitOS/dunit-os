//! Atomic capability delivery with kernel-authenticated attachment provenance.
use super::*;
pub(super) fn send(
    target: u64,
    handle: u64,
    rights: u64,
    message: u64,
    len: u64,
    offset: u64,
) -> i64 {
    let _irq = crate::sync::InterruptGuard::new();
    if target == 0
        || target > u32::MAX as u64
        || handle > u32::MAX as u64
        || rights > u32::MAX as u64
        || rights as u32 & !crate::handle::RIGHTS_ALL != 0
    {
        return EINVAL;
    }
    if len == 0 || len > crate::ipc::MAX_MESSAGE_SIZE as u64 {
        return EMSGSIZE;
    }
    if offset % 4 != 0 || offset.checked_add(4).is_none_or(|end| end > len) {
        return EINVAL;
    }
    let mut bytes = match copy_buffer_from_user(message as *const u8, len as usize) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let target = crate::process::ProcessId(target);
    match crate::process::with_process_mut(target, |p| Ok(p.ipc_handles_enabled)) {
        Ok(true) => {}
        Ok(false) => return EOPNOTSUPP,
        Err(_) => return ENOENT,
    }
    let sender = match crate::process::with_current_process(|p| p.pid) {
        Some(value) => value,
        None => return EINVAL,
    };
    let object = match crate::process::with_current_process_mut(
        |process| -> Result<crate::handle::HandleObject, i64> {
            let held = process
                .handle_rights(handle as u32)
                .map_err(handle_error_to_errno)?;
            if held & crate::handle::RIGHT_TRANSFER == 0 {
                return Err(EACCES);
            }
            let duplicate = process
                .handle_dup(handle as u32, rights as u32 | crate::handle::RIGHT_TRANSFER)
                .map_err(handle_error_to_errno)?;
            match process.handle_take_for_transfer(duplicate) {
                Ok((object, _)) => Ok(object),
                Err(error) => {
                    let _ = process.handle_close(duplicate);
                    Err(handle_error_to_errno(error))
                }
            }
        },
    ) {
        Some(Ok(object)) => object,
        Some(Err(error)) => return error,
        None => return EINVAL,
    };
    // Target handle insertion and queue publication share the target task
    // scope. The IPC guard ends before rollback or readiness notification.
    let result = crate::process::with_process_mut(target, |process| {
        if !process.ipc_handles_enabled {
            return Ok(Err(EOPNOTSUPP));
        }
        if matches!(
            process.state,
            crate::process::ProcessState::Dead | crate::process::ProcessState::Reaped
        ) {
            return Ok(Err(ENOENT));
        }
        let received = process.handle_receive(object, rights as u32);
        bytes[offset as usize..offset as usize + 4].copy_from_slice(&received.to_le_bytes());
        match crate::ipc::send_attached(sender, process, &bytes, received) {
            Ok(()) => Ok(Ok(len as i64)),
            Err(error) => {
                let _ = process.handle_close(received);
                Ok(Err(ipc_error_to_errno(error)))
            }
        }
    });
    match result {
        Ok(Ok(len)) => {
            crate::process::wake_ipc_waiters(target);
            len
        }
        Ok(Err(error)) => error,
        Err(error) => process_error_to_errno(error),
    }
}
pub(super) fn receive(message: u64, len: u64, sender: u64, handle: u64) -> i64 {
    let _irq = crate::sync::InterruptGuard::new();
    if len == 0 || len > crate::ipc::MAX_MESSAGE_SIZE as u64 {
        return EMSGSIZE;
    }
    for (pointer, size) in [(message, len as usize), (sender, 4), (handle, 4)] {
        if let Err(e) = validate_user_mapping(pointer, size, true) {
            return e;
        }
    }
    // Opt in only after every destination has been validated. Even EAGAIN
    // establishes support, allowing peers to send before the next poll.
    let pid = match crate::process::with_current_process_mut(|p| {
        p.ipc_handles_enabled = true;
        p.pid
    }) {
        Some(value) => value,
        None => return EINVAL,
    };
    let packet = match crate::ipc::peek_attached(pid) {
        Ok(p) => p,
        Err(e) => return ipc_error_to_errno(e),
    };
    if packet.len > len as usize {
        return EMSGSIZE;
    }
    for (pointer, data) in [
        (message, &packet.data[..packet.len]),
        (sender, &(packet.sender.0 as u32).to_le_bytes()),
        (handle, &packet.attachment.to_le_bytes()),
    ] {
        if let Err(e) = copy_buffer_to_user(pointer as *mut u8, data) {
            return e;
        }
    }
    match crate::ipc::consume_attached(pid, &packet) {
        Ok(()) => packet.len as i64,
        Err(e) => ipc_error_to_errno(e),
    }
}
