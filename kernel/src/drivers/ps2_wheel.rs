//! Fourth byte of the negotiated IntelliMouse ID 3 packet.
/// ID 3 carries a signed eight-bit wheel delta. The four-bit wheel field of
/// Explorer ID 4 is a different protocol and is not negotiated by this driver.
pub fn intellimouse_wheel(byte: u8) -> i32 {
    byte as i8 as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id3_preserves_direction_and_magnitude_across_nibble_boundaries() {
        for (packet, delta) in [
            (0, 0),
            (1, 1),
            (7, 7),
            (8, 8),
            (15, 15),
            (24, 24),
            (127, 127),
            (128, -128),
            (232, -24),
            (248, -8),
            (255, -1),
        ] {
            assert_eq!(intellimouse_wheel(packet), delta, "packet={packet:#x}");
        }
    }
    #[test]
    fn aggregated_and_individual_detents_have_the_same_total() {
        for packets in [&[24][..], &[8, 8, 8][..], &[7, 7, 7, 3][..]] {
            assert_eq!(
                packets.iter().copied().map(intellimouse_wheel).sum::<i32>(),
                24
            );
        }
        assert_eq!(
            [232, 248].into_iter().map(intellimouse_wheel).sum::<i32>(),
            -32
        );
    }
}
