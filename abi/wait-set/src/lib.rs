//! Native Dunit wait-set records. One layout shared by the kernel and Rust SDK.
#![no_std]

pub const MAX_INTERESTS: usize = 64;
pub const IPC: u32 = 1;
pub const DESCRIPTOR: u32 = 2;
pub const TIMER: u32 = 3;
pub const PROCESS: u32 = 4;
pub const READ: u32 = 1;
pub const WRITE: u32 = 2;
pub const HANGUP: u32 = 4;
pub const ERROR: u32 = 8;
pub const ELAPSED: u32 = 16;
pub const EXITED: u32 = 32;
pub const INFINITE: u64 = u64::MAX;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interest {
    pub kind: u32,
    pub interest: u32,
    pub object: u64,
    pub token: u64,
    pub deadline_ns: u64,
}
impl Interest {
    pub const fn ipc(token: u64) -> Self {
        Self {
            kind: IPC,
            interest: READ,
            object: 0,
            token,
            deadline_ns: 0,
        }
    }
    pub const fn descriptor(fd: u32, events: u32, token: u64) -> Self {
        Self {
            kind: DESCRIPTOR,
            interest: events,
            object: fd as u64,
            token,
            deadline_ns: 0,
        }
    }
    pub const fn timer(deadline_ns: u64, token: u64) -> Self {
        Self {
            kind: TIMER,
            interest: ELAPSED,
            object: 0,
            token,
            deadline_ns,
        }
    }
    pub const fn process(pid: u32, token: u64) -> Self {
        Self {
            kind: PROCESS,
            interest: EXITED,
            object: pid as u64,
            token,
            deadline_ns: 0,
        }
    }
    pub fn valid(&self) -> bool {
        match self.kind {
            IPC => self.interest == READ && self.object == 0 && self.deadline_ns == 0,
            DESCRIPTOR => {
                self.interest & !(READ | WRITE) == 0
                    && self.object <= u32::MAX as u64
                    && self.deadline_ns == 0
            }
            TIMER => self.interest == ELAPSED && self.object == 0,
            PROCESS => {
                self.interest == EXITED
                    && self.object != 0
                    && self.object <= u32::MAX as u64
                    && self.deadline_ns == 0
            }
            _ => false,
        }
    }
    pub fn decode(bytes: &[u8; 32]) -> Self {
        Self {
            kind: u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            interest: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            object: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            token: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            deadline_ns: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub token: u64,
    pub events: u32,
    pub reserved: u32,
    /// PROCESS: wait kind in the upper u32 and signed exit code in the lower.
    /// Other classes return zero. Observing an exit never reaps the child.
    pub data: u64,
}
impl Event {
    pub fn encode(self) -> [u8; 24] {
        let mut bytes = [0; 24];
        bytes[..8].copy_from_slice(&self.token.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.events.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.reserved.to_le_bytes());
        bytes[16..].copy_from_slice(&self.data.to_le_bytes());
        bytes
    }
}
const _: () = assert!(core::mem::size_of::<Interest>() == 32);
const _: () = assert!(core::mem::size_of::<Event>() == 24);
