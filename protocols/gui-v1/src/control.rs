//! Existing live IPC control messages used alongside DGUI packets.
//!
//! This module centralizes their established little-endian layout. It does not
//! introduce DGUI opcodes, kernel rights or a new wire version. Optional input
//! lifecycle and pointer modes are sent only after their features are negotiated.

pub const CTRL_MAGIC: u32 = u32::from_le_bytes(*b"CAP1");
pub const POINTER_MAGIC: u32 = u32::from_le_bytes(*b"PTR1");
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
pub const IN_KEY_UP: u8 = 7;
pub const IN_FOCUS_IN: u8 = 8;
pub const IN_QUIT: u8 = 9;
pub const IN_FOCUS_OUT: u8 = 10;
/// x/y are device-relative movement, value packs buttons and modifiers.
pub const IN_RELATIVE: u8 = 11;
/// x=request (zero for revocation), y=negative errno or zero, value=effective mode.
pub const IN_POINTER_MODE: u8 = 12;
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerMode {
    #[default]
    Normal = 0,
    Confined = 1,
    Locked = 2,
}
impl PointerMode {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Normal),
            1 => Some(Self::Confined),
            2 => Some(Self::Locked),
            _ => None,
        }
    }
}
/// Negotiated sideband request, authenticated by its IPC sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerRequest {
    pub request: u32,
    pub mode: PointerMode,
}
impl PointerRequest {
    pub fn is_message(b: &[u8]) -> bool {
        has_magic(b, POINTER_MAGIC)
    }
    pub fn encode(self) -> [u8; 16] {
        let mut b = [0; 16];
        b[..4].copy_from_slice(&POINTER_MAGIC.to_le_bytes());
        b[4..8].copy_from_slice(&self.request.to_le_bytes());
        b[8..12].copy_from_slice(&(self.mode as u32).to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != 16 || !Self::is_message(b) || b[12..] != [0; 4] {
            return None;
        }
        let request = word(b, 4);
        if request == 0 || request > i32::MAX as u32 {
            return None;
        }
        Some(Self {
            request,
            mode: PointerMode::from_u32(word(b, 8))?,
        })
    }
}
/// Native Shift/Ctrl/Alt/Super, in that order.
pub const INPUT_MODIFIERS_MASK: u8 = 0x0f;
const POINTER_VALUE_MASK: u32 = 0x0fff;

/// Canonical lifecycle pointer payload: button in low byte, modifiers above it.
/// The legacy projection strips modifiers without changing the button byte.
pub fn pointer_value(button: u8, modifiers: u8) -> Option<u32> {
    (modifiers & !INPUT_MODIFIERS_MASK == 0).then_some(button as u32 | ((modifiers as u32) << 8))
}

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
        (bytes.len() == 8).then(|| Self {
            compositor: word(bytes, 0),
            client: word(bytes, 4),
        })
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
    /// For IN_SCROLL, negative is up and positive is down.
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
    /// Low-byte button. A feature-aware decode validates the packed value.
    pub const fn pointer_button(self) -> u8 {
        self.value as u8
    }
    /// Shift/Ctrl/Alt/Super from a negotiated pointer payload.
    pub const fn pointer_modifiers(self) -> u8 {
        ((self.value >> 8) & INPUT_MODIFIERS_MASK as u32) as u8
    }
    pub const fn is_pointer(self) -> bool {
        matches!(
            self.kind,
            IN_RELATIVE | IN_MOVE | IN_DOWN | IN_UP | IN_LEAVE | IN_SCROLL
        )
    }
    fn lifecycle_valid(self) -> bool {
        match self.kind {
            IN_RELATIVE | IN_MOVE | IN_DOWN | IN_UP | IN_LEAVE | IN_SCROLL => {
                self.value & !POINTER_VALUE_MASK == 0
            }
            IN_KEY => self.y >= 0 && self.y & !(INPUT_MODIFIERS_MASK as i32) == 0,
            IN_KEY_UP => {
                self.y >= 0 && self.y & !(INPUT_MODIFIERS_MASK as i32) == 0 && self.value == 0
            }
            IN_FOCUS_IN | IN_FOCUS_OUT => self.x == 0 && self.y == 0 && self.value == 0,
            IN_POINTER_MODE => {
                self.x >= 0 && self.y <= 0 && PointerMode::from_u32(self.value).is_some()
            }
            IN_QUIT => true,
            _ => false,
        }
    }
    /// Project a canonical compositor event onto one connection's negotiated
    /// contract. Legacy clients receive no new kinds and no packed modifiers.
    pub fn for_features(mut self, features: u64) -> Option<Self> {
        if matches!(self.kind, IN_RELATIVE | IN_POINTER_MODE)
            && features & crate::wire::FEATURE_POINTER_MODE == 0
        {
            return None;
        }
        if !self.lifecycle_valid() {
            return None;
        }
        if features & crate::wire::FEATURE_INPUT_LIFECYCLE == 0 {
            if matches!(self.kind, IN_KEY_UP | IN_FOCUS_IN | IN_FOCUS_OUT) {
                return None;
            }
            if self.is_pointer() {
                self.value = self.pointer_button() as u32;
            }
        }
        Some(self)
    }
    pub fn encode_for_features(self, features: u64) -> Option<[u8; INPUT_LEN]> {
        Some(self.for_features(features)?.encode())
    }
    /// Decode exactly the negotiated contract. With no lifecycle feature the
    /// historical decoder retains its existing payload-free leave/quit forms.
    pub fn decode_for_features(bytes: &[u8], features: u64) -> Option<Self> {
        if features & (crate::wire::FEATURE_INPUT_LIFECYCLE | crate::wire::FEATURE_POINTER_MODE)
            == 0
        {
            return Self::decode(bytes);
        }
        if bytes.len() != INPUT_LEN || !has_magic(bytes, INPUT_MAGIC) || bytes[5..8] != [0, 0, 0] {
            return None;
        }
        let input = Self {
            kind: bytes[4],
            x: word(bytes, 8) as i32,
            y: word(bytes, 12) as i32,
            value: word(bytes, 16),
        };
        input.for_features(features)
    }
    /// Legacy decoder. New kinds remain unrecognized unless negotiated.
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
