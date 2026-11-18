//! Native copied-packet queue v1 wire records.
#![no_std]

pub const VERSION: u16 = 1;
pub const MIN_FRAME_LEN: usize = 60;
pub const MAX_FRAME_LEN: usize = 1514;
pub const MAX_RECORDS: usize = 32;
pub const MAX_BATCH_LEN: usize = 48 * 1024;
pub const FRAME_HEADER_LEN: usize = 8;
pub const QUEUE_OPTIONS_LEN: usize = 16;
pub const DEVICE_INFO_LEN: usize = 64;
pub const DEVICE_CONTROL_LEN: usize = 16;

const LINK_UP: u16 = 1;
const RESET: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    Invalid,
    Unsupported,
    BufferTooSmall,
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn zero(bytes: &[u8]) -> bool {
    bytes.iter().all(|&byte| byte == 0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueOptions;

impl QueueOptions {
    pub fn encode(self) -> [u8; QUEUE_OPTIONS_LEN] {
        let mut bytes = [0; QUEUE_OPTIONS_LEN];
        bytes[..2].copy_from_slice(&VERSION.to_le_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != QUEUE_OPTIONS_LEN || le_u16(bytes) != VERSION || !zero(&bytes[2..]) {
            return Err(WireError::Invalid);
        }
        Ok(Self)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeviceInfo {
    pub link_up: bool,
    pub mtu: u16,
    pub next_rx_record_len: u16,
    pub mac: [u8; 6],
    pub rx_frames: u64,
    pub rx_bytes: u64,
    pub rx_dropped: u64,
    pub tx_frames: u64,
    pub tx_bytes: u64,
    pub tx_errors: u64,
}

impl DeviceInfo {
    pub fn encode(self) -> [u8; DEVICE_INFO_LEN] {
        let mut bytes = [0; DEVICE_INFO_LEN];
        bytes[..2].copy_from_slice(&VERSION.to_le_bytes());
        let flags = if self.link_up { LINK_UP } else { 0 };
        bytes[2..4].copy_from_slice(&flags.to_le_bytes());
        bytes[4..6].copy_from_slice(&self.mtu.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.next_rx_record_len.to_le_bytes());
        bytes[8..14].copy_from_slice(&self.mac);
        let counters = [
            self.rx_frames,
            self.rx_bytes,
            self.rx_dropped,
            self.tx_frames,
            self.tx_bytes,
            self.tx_errors,
        ];
        for (out, counter) in bytes[16..].chunks_exact_mut(8).zip(counters) {
            out.copy_from_slice(&counter.to_le_bytes());
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != DEVICE_INFO_LEN
            || le_u16(bytes) != VERSION
            || le_u16(&bytes[2..]) & !LINK_UP != 0
            || !zero(&bytes[14..16])
        {
            return Err(WireError::Invalid);
        }
        let counter = |offset| {
            let mut value = [0; 8];
            value.copy_from_slice(&bytes[offset..offset + 8]);
            u64::from_le_bytes(value)
        };
        let mut mac = [0; 6];
        mac.copy_from_slice(&bytes[8..14]);
        Ok(Self {
            link_up: le_u16(&bytes[2..]) & LINK_UP != 0,
            mtu: le_u16(&bytes[4..]),
            next_rx_record_len: le_u16(&bytes[6..]),
            mac,
            rx_frames: counter(16),
            rx_bytes: counter(24),
            rx_dropped: counter(32),
            tx_frames: counter(40),
            tx_bytes: counter(48),
            tx_errors: counter(56),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceControl {
    Reset,
}

impl DeviceControl {
    pub fn encode(self) -> [u8; DEVICE_CONTROL_LEN] {
        let mut bytes = [0; DEVICE_CONTROL_LEN];
        bytes[..2].copy_from_slice(&VERSION.to_le_bytes());
        let operation = match self {
            Self::Reset => RESET,
        };
        bytes[2..4].copy_from_slice(&operation.to_le_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != DEVICE_CONTROL_LEN || le_u16(bytes) != VERSION || !zero(&bytes[4..]) {
            return Err(WireError::Invalid);
        }
        match le_u16(&bytes[2..]) {
            RESET => Ok(Self::Reset),
            0 => Err(WireError::Invalid),
            _ => Err(WireError::Unsupported),
        }
    }
}

fn aligned_len(frame_len: usize) -> usize {
    FRAME_HEADER_LEN + ((frame_len + 7) & !7)
}

pub fn record_len(frame_len: usize) -> Result<usize, WireError> {
    if !(MIN_FRAME_LEN..=MAX_FRAME_LEN).contains(&frame_len) {
        return Err(WireError::Invalid);
    }
    Ok(aligned_len(frame_len))
}

/// Encode a complete frame. On error the output stays unchanged.
pub fn encode_record(frame: &[u8], output: &mut [u8]) -> Result<usize, WireError> {
    let len = record_len(frame.len())?;
    let output = output.get_mut(..len).ok_or(WireError::BufferTooSmall)?;
    output[..FRAME_HEADER_LEN].fill(0);
    output[..2].copy_from_slice(&(frame.len() as u16).to_le_bytes());
    let frame_end = FRAME_HEADER_LEN + frame.len();
    output[FRAME_HEADER_LEN..frame_end].copy_from_slice(frame);
    output[frame_end..].fill(0);
    Ok(len)
}

/// Borrow a validated, nonempty batch. Immutable bytes allow a complete check
/// before yielding any frame, so the writer can publish a valid prefix safely.
#[derive(Clone, Copy, Debug)]
pub struct Records<'a> {
    remaining: &'a [u8],
    count: usize,
}

impl<'a> Records<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, WireError> {
        if bytes.is_empty() || bytes.len() > MAX_BATCH_LEN {
            return Err(WireError::Invalid);
        }
        let mut remaining = bytes;
        let mut count = 0;
        while !remaining.is_empty() {
            if count == MAX_RECORDS || remaining.len() < FRAME_HEADER_LEN {
                return Err(WireError::Invalid);
            }
            let frame_len = usize::from(le_u16(remaining));
            let len = record_len(frame_len)?;
            if remaining.len() < len
                || !zero(&remaining[2..FRAME_HEADER_LEN])
                || !zero(&remaining[FRAME_HEADER_LEN + frame_len..len])
            {
                return Err(WireError::Invalid);
            }
            remaining = &remaining[len..];
            count += 1;
        }
        Ok(Self {
            remaining: bytes,
            count,
        })
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }
        let frame_len = usize::from(le_u16(self.remaining));
        let frame = &self.remaining[FRAME_HEADER_LEN..FRAME_HEADER_LEN + frame_len];
        self.remaining = &self.remaining[aligned_len(frame_len)..];
        self.count -= 1;
        Some(frame)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.count, Some(self.count))
    }
}

impl ExactSizeIterator for Records<'_> {}
impl core::iter::FusedIterator for Records<'_> {}

#[cfg(test)]
mod tests;
