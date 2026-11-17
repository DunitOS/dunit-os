//! PS/2 packet assembly. Movement uses the header's sign and overflow bits.
#[path = "ps2_wheel.rs"]
mod wheel;

#[derive(Debug, PartialEq, Eq)]
pub struct Packet {
    pub dx: i32,
    pub dy: i32,
    pub buttons: u8,
    pub wheel: i32,
}

pub struct Decoder {
    bytes: [u8; 4],
    index: usize,
    wheel: bool,
}
impl Decoder {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 4],
            index: 0,
            wheel: false,
        }
    }
    pub fn reset(&mut self, wheel: bool) {
        self.index = 0;
        self.wheel = wheel;
    }
    pub fn push(&mut self, byte: u8) -> Option<Packet> {
        if self.index == 0 && byte & 0x08 == 0 {
            return None;
        }
        self.bytes[self.index] = byte;
        self.index += 1;
        if self.index != if self.wheel { 4 } else { 3 } {
            return None;
        }
        self.index = 0;
        let header = self.bytes[0];
        let axis = |byte: u8, sign, overflow| {
            if header & overflow != 0 {
                0
            } else {
                i32::from(byte) - if header & sign != 0 { 256 } else { 0 }
            }
        };
        Some(Packet {
            dx: axis(self.bytes[1], 0x10, 0x40),
            dy: -axis(self.bytes[2], 0x20, 0x80),
            buttons: header & 7,
            wheel: if self.wheel {
                wheel::intellimouse_wheel(self.bytes[3])
            } else {
                0
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(decoder: &mut Decoder, bytes: &[u8]) -> Packet {
        let (last, prefix) = bytes.split_last().unwrap();
        for byte in prefix {
            assert_eq!(decoder.push(*byte), None);
        }
        decoder.push(*last).unwrap()
    }
    #[test]
    fn movement_uses_nine_bit_sign_instead_of_an_i8_cast() {
        let mut decoder = Decoder::new();
        for value in [-255i32, -200, -128, -1, 0, 127, 128, 200, 255] {
            let header = 0x08 | if value < 0 { 0x30 } else { 0 };
            let sample = packet(&mut decoder, &[header, value as u8, value as u8]);
            assert_eq!((sample.dx, sample.dy), (value, -value));
        }
    }
    #[test]
    fn overflow_preserves_buttons_wheel_and_the_other_axis() {
        let mut decoder = Decoder::new();
        decoder.reset(true);
        let sample = packet(&mut decoder, &[0x49, 200, 3, 255]);
        assert_eq!(
            sample,
            Packet {
                dx: 0,
                dy: -3,
                buttons: 1,
                wheel: -1
            }
        );
        let sample = packet(&mut decoder, &[0x8a, 3, 200, 1]);
        assert_eq!(
            sample,
            Packet {
                dx: 3,
                dy: 0,
                buttons: 2,
                wheel: 1
            }
        );
    }
    #[test]
    fn stray_bytes_and_format_reset_cannot_mix_partial_packets() {
        let mut decoder = Decoder::new();
        for byte in [0, 1, 7] {
            assert_eq!(decoder.push(byte), None);
        }
        assert_eq!(decoder.push(0x08), None);
        decoder.reset(true);
        let sample = packet(&mut decoder, &[0x09, 5, 7, 232]);
        assert_eq!(
            sample,
            Packet {
                dx: 5,
                dy: -7,
                buttons: 1,
                wheel: -24
            }
        );
        decoder.reset(false);
        assert_eq!(packet(&mut decoder, &[0x08, 0, 0]).wheel, 0);
    }
}
