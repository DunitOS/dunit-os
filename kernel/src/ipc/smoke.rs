//! Real mailbox/target lifetime and receive reservation checks.
use super::*;
pub fn run_ipc_lock_smoke() -> bool {
    fn check() -> Result<(), IpcError> {
        use crate::process::*;
        let before = ipc_stats();
        let target = create_user_process_record(
            alloc::string::String::from("[ipc-lock-proof]"),
            false,
            false,
        )
        .map_err(|_| IpcError::InvalidTarget)?;
        let sender = ProcessId(1);
        send_bytes(sender, target, b"payload")?;
        if recv_bytes(target, &mut [0; 1]) != Err(IpcError::MessageTooLarge) {
            return Err(IpcError::NoMessage);
        }
        let mut bytes = [0; 32];
        if recv_bytes_with_sender(target, &mut bytes)? != (sender, 7) || &bytes[..7] != b"payload" {
            return Err(IpcError::NoMessage);
        }
        with_process_mut(target, |process| {
            Ok(send_attached(sender, process, b"first", 7))
        })
        .map_err(|_| IpcError::InvalidTarget)??;
        with_process_mut(target, |process| {
            Ok(send_attached(sender, process, b"second", 8))
        })
        .map_err(|_| IpcError::InvalidTarget)??;
        let first = peek_attached(target)?;
        consume_attached(target, &first)?;
        if consume_attached(target, &first) != Err(IpcError::NoMessage)
            || peek_attached(target)?.attachment != 8
        {
            return Err(IpcError::NoMessage);
        }
        consume_attached(target, &peek_attached(target)?)?;
        for _ in 0..MAX_QUEUE_MESSAGES {
            send_bytes(sender, target, b"x")?;
        }
        if send_bytes(sender, target, b"x") != Err(IpcError::QueueFull) {
            return Err(IpcError::QueueFull);
        }
        autoreap_process(target, "ipc-lock-proof-done").map_err(|_| IpcError::InvalidTarget)?;
        let after = ipc_stats();
        if after.queue_count != before.queue_count
            || after.queued_messages != before.queued_messages
        {
            return Err(IpcError::NoMessage);
        }
        Ok(())
    }
    if check().is_ok() {
        crate::memory::serial_write(
            "[IPC-LOCK] bounded FIFO, attachment identity and cleanup OK\r\n",
        );
        true
    } else {
        crate::memory::serial_write("[IPC-LOCK] FAIL\r\n");
        false
    }
}
