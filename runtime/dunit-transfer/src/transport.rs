//! Authenticated attachment transport shared by clients and the broker.
use super::*;
use alloc::collections::VecDeque;
/// A pending send owns its attachment until the kernel has either delivered it
/// atomically or rejected the send. EAGAIN retains it for the next pump.
pub struct Packet {
    pub to: u32,
    pub message: Message,
    attachment: Option<(OwnedHandle, u32)>,
}
impl Packet {
    pub fn control(to: u32, message: Message) -> Self {
        Self {
            to,
            message,
            attachment: None,
        }
    }
    pub fn handle(to: u32, message: Message, handle: OwnedHandle, rights: u32) -> Self {
        Self {
            to,
            message,
            attachment: Some((handle, rights)),
        }
    }
    pub fn blob(to: u32, mut message: Message, bytes: &[u8]) -> NativeResult<Self> {
        let body = encode_blob(message.arg0, message.arg1, bytes).map_err(invalid)?;
        let bytes = body.as_slice();
        let handle = OwnedHandle(checked(libdunit::handle_create_shared(bytes.len()))? as u32);
        let mapped = checked(libdunit::handle_map(handle.raw(), 0, bytes.len()))?;
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped as *mut u8, bytes.len());
        }
        let unmapped = libdunit::vm_unmap(mapped, (bytes.len() + 4095) & !4095);
        checked(unmapped)?;
        message.arg0 = 0;
        message.arg1 = bytes.len() as u32;
        Ok(Self::handle(
            to,
            message,
            handle,
            libdunit::RIGHT_READ | libdunit::RIGHT_MAP,
        ))
    }
    fn send(&self) -> isize {
        match &self.attachment {
            Some((handle, rights)) => libdunit::ipc_send_handle(
                self.to,
                handle.raw(),
                *rights,
                &self.message.encode(),
                24,
            ),
            None => libdunit::ipc_send(self.to, &self.message.encode()),
        }
    }
}
#[derive(Default)]
pub struct Outbox {
    queue: VecDeque<Packet>,
}
impl Outbox {
    pub fn push(&mut self, packet: Packet) -> NativeResult<()> {
        if self.queue.len() >= 1024 {
            return Err(Error(libdunit::ENOBUFS));
        }
        self.queue.push_back(packet);
        Ok(())
    }
    /// Does not wait or receive. Caller invokes it before sleeping and after
    /// processing input. Failed recipient sends are returned with correlation.
    pub fn flush(&mut self) -> Vec<(u32, Message, Error)> {
        let mut failed = Vec::new();
        // Different peers have independent kernel queues. A stalled peer must
        // not hold up every other transfer; one pass preserves per-peer order.
        let mut blocked = Vec::new();
        let count = self.queue.len();
        for _ in 0..count {
            let packet = self.queue.pop_front().unwrap();
            if blocked.contains(&packet.to) {
                self.queue.push_back(packet);
                continue;
            }
            let result = packet.send();
            if result == libdunit::EAGAIN || result == libdunit::ENOBUFS {
                blocked.push(packet.to);
                self.queue.push_back(packet);
            } else if result < 0 {
                failed.push((packet.to, packet.message, Error(result)));
            }
        }
        failed
    }
    pub fn extend(&mut self, packets: Vec<Packet>) -> NativeResult<()> {
        if self.queue.len().saturating_add(packets.len()) > 1024 {
            return Err(Error(libdunit::ENOBUFS));
        }
        self.queue.extend(packets);
        Ok(())
    }
    pub fn discard_data(&mut self, id: u64) {
        self.queue.retain(|p| {
            p.message.transfer != id
                || matches!(
                    p.message.kind,
                    Kind::Result | Kind::Cancelled | Kind::Finished | Kind::ClipboardChanged
                )
        });
    }
    pub fn discard_peer(&mut self, to: u32) {
        self.queue.retain(|p| p.to != to);
    }
    pub fn discard_transfer(&mut self, id: u64) {
        self.queue.retain(|p| p.message.transfer != id);
    }
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}
/// The caller must supply the actual handle returned by IpcReceiveHandle.
/// A forged number in XFR1 alone never becomes an owner or gets closed.
pub fn read_blob(
    message: Message,
    attachment: Option<OwnedHandle>,
) -> NativeResult<(Message, Vec<u8>)> {
    read_blob_limited(message, attachment, MAX_BLOB_BYTES)
}
pub fn read_blob_limited(
    mut message: Message,
    attachment: Option<OwnedHandle>,
    limit: usize,
) -> NativeResult<(Message, Vec<u8>)> {
    let handle = attachment.ok_or(Error(libdunit::EACCES))?;
    let len = message.arg1 as usize;
    if handle.raw() != message.arg0 || len == 0 || len > limit.min(MAX_BLOB_BYTES) {
        return Err(Error(libdunit::EINVAL));
    }
    let real = checked(libdunit::handle_shared_len(handle.raw()))?;
    if len > real {
        return Err(Error(libdunit::EINVAL));
    }
    let mapped = checked(libdunit::handle_map(handle.raw(), 0, len))?;
    let mut bytes = Vec::with_capacity(len);
    // Copy before decoding: the provider may retain a writable alias. Parsing
    // and the durable clipboard always use this private, bounded snapshot.
    for i in 0..len {
        bytes.push(unsafe { core::ptr::read_volatile((mapped as *const u8).add(i)) });
    }
    checked(libdunit::vm_unmap(mapped, (len + 4095) & !4095))?;
    let (arg0, arg1, _) = decode_blob(&bytes).map_err(invalid)?;
    message.arg0 = arg0;
    message.arg1 = arg1;
    Ok((message, bytes.split_off(BLOB_HEADER_LEN)))
}
