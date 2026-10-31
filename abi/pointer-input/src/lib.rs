#![no_std]
//! ReadPointerEvent v1: fixed little-endian records, independent of Rust padding.
pub const RECORD_LEN: usize = 32;
pub const LOST_BEFORE: u32 = 1;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerEvent {
    pub x: u32,
    pub y: u32,
    pub buttons: u32,
    pub wheel: i32,
    pub sequence: u64,
    pub flags: u32,
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
}
