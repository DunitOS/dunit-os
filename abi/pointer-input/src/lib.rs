#![no_std]
//! ReadPointerEvent: fixed little-endian records, independent of Rust padding.
pub const RECORD_LEN: usize = 32;
pub const RECORD_WITH_MODIFIERS_LEN: usize = 40;
pub const LOST_BEFORE: u32 = 1;
pub const MODIFIER_MASK: u32 = 0x0f;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerEvent {
    pub x: u32,
    pub y: u32,
    pub buttons: u32,
    pub wheel: i32,
    pub sequence: u64,
    pub flags: u32,
}
/// Explicitly requested v2 record. The original v1 projection stays byte-exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerEventWithModifiers {
    pub event: PointerEvent,
    pub modifiers: u32,
}
/// Explicit v3 record: device motion is retained before display clamping.
pub const RECORD_WITH_MOTION_LEN: usize = 56;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerEventWithMotion {
    pub event: PointerEvent,
    pub modifiers: u32,
    pub dx: i32,
    pub dy: i32,
    pub relative: bool,
}
impl PointerEventWithMotion {
    pub const EMPTY: Self = Self {
        event: PointerEvent::EMPTY,
        modifiers: 0,
        dx: 0,
        dy: 0,
        relative: false,
    };
    pub fn legacy(self) -> PointerEventWithModifiers {
        PointerEventWithModifiers {
            event: self.event,
            modifiers: self.modifiers,
        }
    }
    pub fn encode(self) -> [u8; RECORD_WITH_MOTION_LEN] {
        let mut b = [0; RECORD_WITH_MOTION_LEN];
        b[..40].copy_from_slice(&self.legacy().encode());
        b[40..44].copy_from_slice(&self.dx.to_le_bytes());
        b[44..48].copy_from_slice(&self.dy.to_le_bytes());
        b[48..52].copy_from_slice(&(self.relative as u32).to_le_bytes());
        b
    }
    pub fn decode(b: &[u8; RECORD_WITH_MOTION_LEN]) -> Option<Self> {
        let old = PointerEventWithModifiers::decode(b[..40].try_into().ok()?)?;
        let source = u32::from_le_bytes(b[48..52].try_into().ok()?);
        let dx = i32::from_le_bytes(b[40..44].try_into().ok()?);
        let dy = i32::from_le_bytes(b[44..48].try_into().ok()?);
        if source > 1 || b[52..] != [0; 4] || (source == 0 && (dx != 0 || dy != 0)) {
            return None;
        }
        Some(Self {
            event: old.event,
            modifiers: old.modifiers,
            dx,
            dy,
            relative: source == 1,
        })
    }
}
impl PointerEventWithModifiers {
    pub const EMPTY: Self = Self {
        event: PointerEvent::EMPTY,
        modifiers: 0,
    };
    pub fn encode(self) -> [u8; RECORD_WITH_MODIFIERS_LEN] {
        let mut bytes = [0; RECORD_WITH_MODIFIERS_LEN];
        bytes[..RECORD_LEN].copy_from_slice(&self.event.encode());
        bytes[32..36].copy_from_slice(&self.modifiers.to_le_bytes());
        bytes
    }
    pub fn decode(bytes: &[u8; RECORD_WITH_MODIFIERS_LEN]) -> Option<Self> {
        let modifiers = u32::from_le_bytes(bytes[32..36].try_into().ok()?);
        if modifiers & !MODIFIER_MASK != 0 || bytes[36..40] != [0; 4] {
            return None;
        }
        Some(Self {
            event: PointerEvent::decode(bytes[..RECORD_LEN].try_into().ok()?)?,
            modifiers,
        })
    }
}
impl PointerEvent {
    pub const EMPTY: Self = Self {
        x: 0,
        y: 0,
        buttons: 0,
        wheel: 0,
        sequence: 0,
        flags: 0,
    };
    pub fn encode(self) -> [u8; RECORD_LEN] {
        let mut b = [0; RECORD_LEN];
        b[0..4].copy_from_slice(&self.x.to_le_bytes());
        b[4..8].copy_from_slice(&self.y.to_le_bytes());
        b[8..12].copy_from_slice(&self.buttons.to_le_bytes());
        b[12..16].copy_from_slice(&self.wheel.to_le_bytes());
        b[16..24].copy_from_slice(&self.sequence.to_le_bytes());
        b[24..28].copy_from_slice(&self.flags.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8; RECORD_LEN]) -> Option<Self> {
        let flags = u32::from_le_bytes(b[24..28].try_into().ok()?);
        if flags & !LOST_BEFORE != 0 || b[28..32] != [0; 4] {
            return None;
        }
        Some(Self {
            x: u32::from_le_bytes(b[0..4].try_into().ok()?),
            y: u32::from_le_bytes(b[4..8].try_into().ok()?),
            buttons: u32::from_le_bytes(b[8..12].try_into().ok()?),
            wheel: i32::from_le_bytes(b[12..16].try_into().ok()?),
            sequence: u64::from_le_bytes(b[16..24].try_into().ok()?),
            flags,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_is_little_endian_and_reserved_is_zero() {
        let e = PointerEvent {
            x: 0x12345678,
            y: 900,
            buttons: 5,
            wheel: -3,
            sequence: 0x123456789abcdef0,
            flags: LOST_BEFORE,
        };
        let b = e.encode();
        assert_eq!(&b[..4], &[0x78, 0x56, 0x34, 0x12]);
        assert_eq!(&b[28..], &[0; 4]);
        assert_eq!(PointerEvent::decode(&b), Some(e));
        let mut bad = b;
        bad[31] = 1;
        assert_eq!(PointerEvent::decode(&bad), None);
        bad = b;
        bad[24] = 2;
        assert_eq!(PointerEvent::decode(&bad), None);
    }
    #[test]
    fn modifier_record_preserves_v1_projection_and_validates_reserved_fields() {
        let old = PointerEvent {
            x: 12,
            y: 34,
            buttons: 3,
            wheel: -2,
            sequence: 0x123456789abc,
            flags: LOST_BEFORE,
        };
        let event = PointerEventWithModifiers {
            event: old,
            modifiers: 15,
        };
        let bytes = event.encode();
        assert_eq!(&bytes[..32], &old.encode());
        assert_eq!(&bytes[32..], &[15, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(PointerEventWithModifiers::decode(&bytes), Some(event));
        for at in [28, 31, 36, 39] {
            let mut bad = bytes;
            bad[at] = 1;
            assert_eq!(PointerEventWithModifiers::decode(&bad), None);
        }
        for modifiers in [16u32, 256, u32::MAX] {
            let mut bad = bytes;
            bad[32..36].copy_from_slice(&modifiers.to_le_bytes());
            assert_eq!(PointerEventWithModifiers::decode(&bad), None);
        }
    }
}
#[cfg(test)]
mod motion_tests {
    use super::*;
    #[test]
    fn motion_record_preserves_legacy_prefix_and_validates_source() {
        let mut e = PointerEventWithMotion {
            event: PointerEvent {
                x: 99,
                y: 0,
                sequence: 1,
                ..PointerEvent::EMPTY
            },
            modifiers: 3,
            dx: 5000,
            dy: -4000,
            relative: true,
        };
        let b = e.encode();
        assert_eq!(&b[..40], &e.legacy().encode());
        assert_eq!(PointerEventWithMotion::decode(&b), Some(e));
        let mut bad = b;
        bad[48] = 2;
        assert_eq!(PointerEventWithMotion::decode(&bad), None);
        bad = b;
        bad[52] = 1;
        assert_eq!(PointerEventWithMotion::decode(&bad), None);
        bad = b;
        bad[48] = 0;
        assert_eq!(PointerEventWithMotion::decode(&bad), None);
        e.relative = false;
        e.dx = 0;
        e.dy = 0;
        assert_eq!(PointerEventWithMotion::decode(&e.encode()), Some(e));
    }
}
