use crate::process::ProcessId;
use crate::sync::{IrqSafeSpinLock, IrqSafeSpinLockGuard};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};

pub const MAX_MESSAGE_SIZE: usize = 256;
pub const MAX_QUEUE_MESSAGES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SharedMemoryId(pub u64);

pub struct SharedMemoryRegion {
    pub id: SharedMemoryId,
    pub physical_addr: u64,
    pub size: usize,
    pub owners: Vec<ProcessId>,
}

impl SharedMemoryRegion {
    pub fn new(id: SharedMemoryId, physical_addr: u64, size: usize) -> Self {
        Self {
            id,
            physical_addr,
            size,
            owners: Vec::new(),
        }
    }

    pub fn add_owner(&mut self, pid: ProcessId) {
        if !self.owners.contains(&pid) {
            self.owners.push(pid);
        }
    }

    pub fn remove_owner(&mut self, pid: ProcessId) {
        self.owners.retain(|&p| p != pid);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MessageType {
    MouseEvent { x: i32, y: i32, buttons: u8 },
    KeyboardEvent { scancode: u8, pressed: bool },
    RenderFrame { buffer_id: u64 },
    WindowCreate { width: u32, height: u32 },
    WindowClose { window_id: u32 },
}

#[derive(Debug, Clone)]
pub struct Message {
    sequence: u64,
    pub sender: ProcessId,
    pub attachment: u32,
    pub msg_type: MessageType,
    pub len: usize,
    pub data: [u8; MAX_MESSAGE_SIZE],
}

impl Message {
    pub fn new(sender: ProcessId, msg_type: MessageType) -> Self {
        Self {
            sequence: 0,
            sender,
            attachment: 0,
            msg_type,
            len: 0,
            data: [0; MAX_MESSAGE_SIZE],
        }
    }

    pub fn with_data(sender: ProcessId, msg_type: MessageType, data: &[u8]) -> Self {
        let mut msg = Self::new(sender, msg_type);
        let len = data.len().min(MAX_MESSAGE_SIZE);
        msg.data[..len].copy_from_slice(&data[..len]);
        msg.len = len;
        msg
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidTarget,
    MessageTooLarge,
    QueueFull,
    NoMessage,
    Unavailable,
    AttachmentRequired,
}

pub struct IpcManager {
    shared_regions: BTreeMap<SharedMemoryId, SharedMemoryRegion>,
    message_queues: BTreeMap<ProcessId, VecDeque<Message>>,
    next_message_sequence: u64,
    next_shared_id: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct IpcStats {
    pub queue_count: u64,
    pub queued_messages: u64,
    pub shared_regions: u64,
    pub max_queue_messages: u64,
}

impl IpcManager {
    pub const fn new() -> Self {
        Self {
            shared_regions: BTreeMap::new(),
            message_queues: BTreeMap::new(),
            next_message_sequence: 0,
            next_shared_id: 1,
        }
    }

    pub fn create_shared_memory(&mut self, size: usize) -> Option<SharedMemoryId> {
        use crate::memory::pmm::get_pmm;

        let pmm = get_pmm()?;
        let num_pages = (size + 4095) / 4096;

        let mut physical_frames = Vec::new();
        for _ in 0..num_pages {
            match pmm.alloc_frame() {
                Some(frame) => physical_frames.push(frame),
                None => {
                    for frame in physical_frames {
                        pmm.free_frame(frame);
                    }
                    return None;
                }
            }
        }

        let id = SharedMemoryId(self.next_shared_id);
        self.next_shared_id += 1;

        let physical_addr = physical_frames[0].as_usize() as u64;
        let region = SharedMemoryRegion::new(id, physical_addr, size);
        self.shared_regions.insert(id, region);

        Some(id)
    }

    pub fn attach_shared_memory(&mut self, id: SharedMemoryId, pid: ProcessId) -> Option<u64> {
        let region = self.shared_regions.get_mut(&id)?;
        region.add_owner(pid);
        Some(region.physical_addr)
    }

    pub fn detach_shared_memory(&mut self, id: SharedMemoryId, pid: ProcessId) {
        if let Some(region) = self.shared_regions.get_mut(&id) {
            region.remove_owner(pid);

            if region.owners.is_empty() {
                self.shared_regions.remove(&id);
            }
        }
    }

    pub fn get_shared_region(&self, id: SharedMemoryId) -> Option<&SharedMemoryRegion> {
        self.shared_regions.get(&id)
    }

    pub fn send_message(&mut self, target: ProcessId, mut msg: Message) -> Result<(), IpcError> {
        if !crate::process::process_exists(target) {
            return Err(IpcError::InvalidTarget);
        }
        let queue = self
            .message_queues
            .entry(target)
            .or_insert_with(VecDeque::new);
        if queue.len() >= MAX_QUEUE_MESSAGES {
            return Err(IpcError::QueueFull);
        }
        self.next_message_sequence = self
            .next_message_sequence
            .checked_add(1)
            .expect("IPC sequence exhausted");
        msg.sequence = self.next_message_sequence;
        queue.push_back(msg);
        Ok(())
    }

    pub fn receive_message(&mut self, pid: ProcessId) -> Option<Message> {
        let queue = self.message_queues.get_mut(&pid)?;
        if queue.is_empty() {
            None
        } else {
            queue.pop_front()
        }
    }

    pub fn has_messages(&self, pid: ProcessId) -> bool {
        self.message_queues
            .get(&pid)
            .map(|q| !q.is_empty())
            .unwrap_or(false)
    }

    pub fn clear_messages(&mut self, pid: ProcessId) {
        self.message_queues.remove(&pid);
    }

    pub fn stats(&self) -> IpcStats {
        let mut queued_messages = 0u64;
        for queue in self.message_queues.values() {
            queued_messages += queue.len() as u64;
        }
        IpcStats {
            queue_count: self.message_queues.len() as u64,
            queued_messages,
            shared_regions: self.shared_regions.len() as u64,
            max_queue_messages: MAX_QUEUE_MESSAGES as u64,
        }
    }
}

// Registry ownership is bounded by an actual guard; no static mutable reference.
static IPC_MANAGER_INSTANCE: IrqSafeSpinLock<Option<IpcManager>> = IrqSafeSpinLock::new(None);
pub struct IpcGuard(IrqSafeSpinLockGuard<'static, Option<IpcManager>>);
impl Deref for IpcGuard {
    type Target = IpcManager;
    fn deref(&self) -> &IpcManager {
        self.0.as_ref().expect("initialized IPC guard")
    }
}
impl DerefMut for IpcGuard {
    fn deref_mut(&mut self) -> &mut IpcManager {
        self.0.as_mut().expect("initialized IPC guard")
    }
}

/// Initialize the IPC manager and return its measured startup stats. At init
/// there are no message queues (they are created lazily per target PID on the
/// first `send`) and no shared-memory regions, so the boot log can report the
/// real counts instead of claiming queues/shared memory were pre-created.
pub fn init() -> IpcStats {
    {
        let mut manager = IPC_MANAGER_INSTANCE.lock();
        assert!(manager.is_none(), "IPC initialized twice");
        *manager = Some(IpcManager::new());
    }
    let stats = ipc_stats();
    crate::memory::serial_write(
        "[IPC] manager ready: bounded byte message queues (lazy per-PID)\r\n",
    );
    stats
}

pub fn get_ipc_manager() -> Option<IpcGuard> {
    let guard = IPC_MANAGER_INSTANCE.lock();
    if guard.is_none() {
        None
    } else {
        Some(IpcGuard(guard))
    }
}

pub fn send_bytes(sender: ProcessId, target: ProcessId, data: &[u8]) -> Result<(), IpcError> {
    if data.is_empty() || data.len() > MAX_MESSAGE_SIZE {
        return Err(IpcError::MessageTooLarge);
    }
    // The target scope closes kill/admission races. The queue lock additionally
    // orders process_exists with registry removal -> mailbox clearing.
    crate::process::with_process_mut(target, |process| {
        if matches!(
            process.state,
            crate::process::ProcessState::Dead | crate::process::ProcessState::Reaped
        ) {
            return Ok(Err(IpcError::InvalidTarget));
        }
        let result = get_ipc_manager()
            .ok_or(IpcError::Unavailable)
            .and_then(|mut manager| {
                manager.send_message(
                    target,
                    Message::with_data(sender, MessageType::WindowClose { window_id: 0 }, data),
                )
            });
        Ok(result)
    })
    .map_err(|_| IpcError::InvalidTarget)??;
    // No task or IPC guard may span a readiness callback.
    crate::process::wake_ipc_waiters(target);
    Ok(())
}

pub fn recv_bytes(pid: ProcessId, out: &mut [u8]) -> Result<usize, IpcError> {
    recv_bytes_with_sender(pid, out).map(|(_, len)| len)
}

pub fn recv_bytes_with_sender(
    pid: ProcessId,
    out: &mut [u8],
) -> Result<(ProcessId, usize), IpcError> {
    let mut manager = get_ipc_manager().ok_or(IpcError::Unavailable)?;
    let queue = manager
        .message_queues
        .get_mut(&pid)
        .ok_or(IpcError::NoMessage)?;
    if queue.is_empty() {
        return Err(IpcError::NoMessage);
    }
    if queue[0].attachment != 0 {
        return Err(IpcError::AttachmentRequired);
    }
    let len = queue[0].len;
    if out.len() < len {
        return Err(IpcError::MessageTooLarge);
    }
    let message = queue.pop_front().expect("checked nonempty queue");
    out[..len].copy_from_slice(&message.data[..len]);
    Ok((message.sender, len))
}

pub fn ipc_stats() -> IpcStats {
    get_ipc_manager()
        .map(|manager| manager.stats())
        .unwrap_or_default()
}

/// Authenticated attachment metadata is supplied only by the atomic handle syscall.
pub fn send_attached(
    sender: ProcessId,
    target: &mut crate::process::Process,
    data: &[u8],
    attachment: u32,
) -> Result<(), IpcError> {
    if data.is_empty() || data.len() > MAX_MESSAGE_SIZE {
        return Err(IpcError::MessageTooLarge);
    }
    let mut message = Message::with_data(sender, MessageType::WindowClose { window_id: 0 }, data);
    message.attachment = attachment;
    if matches!(
        target.state,
        crate::process::ProcessState::Dead | crate::process::ProcessState::Reaped
    ) {
        return Err(IpcError::InvalidTarget);
    }
    get_ipc_manager()
        .ok_or(IpcError::Unavailable)?
        .send_message(target.pid, message)?;
    Ok(()) // caller notifies only after releasing the target task transaction
}
pub fn peek_attached(pid: ProcessId) -> Result<Message, IpcError> {
    get_ipc_manager()
        .ok_or(IpcError::Unavailable)?
        .message_queues
        .get(&pid)
        .and_then(|q| q.front())
        .cloned()
        .ok_or(IpcError::NoMessage)
}
pub fn consume_attached(pid: ProcessId, packet: &Message) -> Result<(), IpcError> {
    let mut manager = get_ipc_manager().ok_or(IpcError::Unavailable)?;
    let queue = manager
        .message_queues
        .get_mut(&pid)
        .ok_or(IpcError::NoMessage)?;
    if queue
        .front()
        .is_none_or(|front| front.sequence != packet.sequence)
    {
        return Err(IpcError::NoMessage);
    }
    queue.pop_front();
    Ok(())
}

#[cfg(feature = "boot-smoke-tests")]
mod smoke;
#[cfg(feature = "boot-smoke-tests")]
pub use smoke::run_ipc_lock_smoke;
