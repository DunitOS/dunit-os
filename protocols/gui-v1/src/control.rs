//! Existing live IPC control messages used alongside DGUI packets.
//!
//! This module centralizes their established little-endian layout. It does not
//! introduce opcodes, capabilities, syscall changes or a new wire version.

pub const CTRL_MAGIC: u32 = u32::from_le_bytes(*b"CAP1");
pub const INPUT_MAGIC: u32 = u32::from_le_bytes(*b"INP1");
pub const RELOAD_MAGIC: u32 = u32::from_le_bytes(*b"RLD1");
pub const NOTIFY_MAGIC: u32 = u32::from_le_bytes(*b"NOT1");
pub const CAPABILITY_LEN: usize = 16;
pub const INPUT_LEN: usize = 20;
pub const NOTIFY_TEXT_MAX: usize = 64;
pub const NOTIFY_LEN: usize = 8 + NOTIFY_TEXT_MAX;
pub const IN_MOVE: u8 = 1;
pub const IN_DOWN: u8 = 2;
pub const IN_UP: u8 = 3;
pub const IN_LEAVE: u8 = 4;
pub const IN_KEY: u8 = 5;
pub const IN_SCROLL: u8 = 6;
pub const IN_QUIT: u8 = 9;

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn has_magic(bytes: &[u8], magic: u32) -> bool {
    bytes.len() >= 4 && word(bytes, 0) == magic
}

/// Existing launch introduction: compositor PID followed by client/tint ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bootstrap {
    pub compositor: u32,
    pub client: u32,
}
impl Bootstrap {
    pub fn encode(self) -> [u8; 8] {
        let mut bytes = [0; 8];
        bytes[..4].copy_from_slice(&self.compositor.to_le_bytes());
        bytes[4..].copy_from_slice(&self.client.to_le_bytes());
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        (bytes.len() == 8).then(|| Self { compositor: word(bytes, 0), client: word(bytes, 4) })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capability {
    pub handle: u32,
    /// Advisory only. Receivers must query the backing length from the kernel.
    pub advertised_bytes: u32,
    pub object: u32,
}
impl Capability {
    pub fn encode(self) -> [u8; CAPABILITY_LEN] {
        let mut bytes = [0; CAPABILITY_LEN];
        for (offset, value) in [
            (0, CTRL_MAGIC),
            (4, self.handle),
            (8, self.advertised_bytes),
            (12, self.object),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < CAPABILITY_LEN || !has_magic(bytes, CTRL_MAGIC) {
            return None;
        }
        Some(Self {
            handle: word(bytes, 4),
            advertised_bytes: word(bytes, 8),
            object: word(bytes, 12),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Input {
    pub kind: u8,
    /// Pointer x, wheel delta or key scancode, depending on kind.
    pub x: i32,
    /// Pointer y or keyboard modifiers, depending on kind.
    pub y: i32,
    /// Button or cooked key byte, depending on kind.
    pub value: u32,
}
impl Input {
    pub fn encode(self) -> [u8; INPUT_LEN] {
        let mut bytes = [0; INPUT_LEN];
        bytes[..4].copy_from_slice(&INPUT_MAGIC.to_le_bytes());
        bytes[4] = self.kind;
        for (offset, value) in [(8, self.x as u32), (12, self.y as u32), (16, self.value)] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 8 || !has_magic(bytes, INPUT_MAGIC) {
            return None;
        }
        let kind = bytes[4];
        if !matches!(
            kind,
            IN_MOVE | IN_DOWN | IN_UP | IN_LEAVE | IN_KEY | IN_SCROLL | IN_QUIT
        ) {
            return None;
        }
        // Early clients accepted an eight-byte quit/leave signal. Retain those
        // payload-free forms; coordinates and keys always need their full data.
        if bytes.len() < INPUT_LEN {
            return matches!(kind, IN_QUIT | IN_LEAVE).then_some(Self {
                kind,
                x: 0,
                y: 0,
                value: 0,
            });
        }
        Some(Self {
            kind,
            x: word(bytes, 8) as i32,
            y: word(bytes, 12) as i32,
            value: word(bytes, 16),
        })
    }
}

/// Existing Settings senders use eight bytes; receivers also accept the
/// original four-byte magic-only form. Neither form contains configuration.
pub const RELOAD: [u8; 8] = {
    let magic = RELOAD_MAGIC.to_le_bytes();
    [magic[0], magic[1], magic[2], magic[3], 0, 0, 0, 0]
};
pub fn is_reload(bytes: &[u8]) -> bool {
    has_magic(bytes, RELOAD_MAGIC)
}

/// Encode bounded UTF-8 without splitting a character at the text limit.
pub fn notification(text: &str, bytes: &mut [u8; NOTIFY_LEN]) -> usize {
    let mut len = text.len().min(NOTIFY_TEXT_MAX);
    while !text.is_char_boundary(len) {
        len -= 1;
    }
    bytes[..4].copy_from_slice(&NOTIFY_MAGIC.to_le_bytes());
    bytes[4..8].copy_from_slice(&(len as u32).to_le_bytes());
    bytes[8..8 + len].copy_from_slice(&text.as_bytes()[..len]);
    8 + len
}
/// Recognize the control family even if its payload is malformed, so callers
/// can discard it without interpreting notification text as a DGUI header.
pub fn is_notification(bytes: &[u8]) -> bool {
    has_magic(bytes, NOTIFY_MAGIC)
}

/// Clamp the untrusted length to the received bytes and the established limit.
pub fn notification_text(bytes: &[u8]) -> Option<&str> {
    if bytes.len() < 8 || !has_magic(bytes, NOTIFY_MAGIC) {
        return None;
    }
    let len = (word(bytes, 4) as usize)
        .min(bytes.len() - 8)
        .min(NOTIFY_TEXT_MAX);
    let text = core::str::from_utf8(&bytes[8..8 + len]).ok()?;
    (!text.is_empty()).then_some(text)
}
