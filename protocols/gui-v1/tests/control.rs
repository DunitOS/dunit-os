//! Golden bytes for the live control channel, independent of DGUI framing.
use gui_protocol_v1::control::*;

#[test]
fn capability_encoding_preserves_the_existing_little_endian_layout() {
    let message = Capability {
        handle: 0x12345678,
        advertised_bytes: 0xffff_ffff,
        object: 5,
    };
    let bytes = message.encode();
    assert_eq!(
        bytes,
        [b'C', b'A', b'P', b'1', 0x78, 0x56, 0x34, 0x12, 255, 255, 255, 255, 5, 0, 0, 0]
    );
    assert_eq!(Capability::decode(&bytes), Some(message));
    for len in 0..CAPABILITY_LEN {
        assert_eq!(Capability::decode(&bytes[..len]), None);
    }
    // The parser conveys a client's claim; it never authorizes a mapping size.
    assert_eq!(
        Capability::decode(&bytes).unwrap().advertised_bytes,
        u32::MAX
    );
}

#[test]
fn input_preserves_negative_coordinates_modifiers_and_reserved_bytes() {
    for kind in [
        IN_MOVE, IN_DOWN, IN_UP, IN_LEAVE, IN_KEY, IN_SCROLL, IN_QUIT,
    ] {
        let input = Input {
            kind,
            x: -7,
            y: 0x123456,
            value: 0x80,
        };
        let bytes = input.encode();
        assert_eq!(&bytes[..4], b"INP1");
        assert_eq!(&bytes[5..8], &[0, 0, 0]);
        assert_eq!(&bytes[8..12], &(-7i32).to_le_bytes());
        assert_eq!(Input::decode(&bytes), Some(input));
    }
}

#[test]
fn short_payloads_cannot_supply_stale_key_or_pointer_fields() {
    for kind in [IN_MOVE, IN_DOWN, IN_UP, IN_KEY, IN_SCROLL] {
        let bytes = Input {
            kind,
            x: 9,
            y: 8,
            value: 7,
        }
        .encode();
        for len in 0..INPUT_LEN {
            assert_eq!(Input::decode(&bytes[..len]), None);
        }
    }
    for kind in [IN_QUIT, IN_LEAVE] {
        let bytes = Input {
            kind,
            x: 9,
            y: 8,
            value: 7,
        }
        .encode();
        assert_eq!(
            Input::decode(&bytes[..8]),
            Some(Input {
                kind,
                x: 0,
                y: 0,
                value: 0
            })
        );
    }
    let mut bytes = Input {
        kind: IN_KEY,
        x: 0,
        y: 0,
        value: 0,
    }
    .encode();
    bytes[4] = 255;
    assert_eq!(Input::decode(&bytes), None);
}

#[test]
fn reload_accepts_both_existing_forms_without_reading_configuration() {
    assert_eq!(&RELOAD[..4], b"RLD1");
    assert!(is_reload(&RELOAD));
    assert!(is_reload(b"RLD1"));
    for bytes in [&b"RLD"[..], &b"INP1"[..], &b""[..]] {
        assert!(!is_reload(bytes));
    }
}

#[test]
fn notification_sender_preserves_utf8_at_the_byte_budget() {
    let text = "x".repeat(63) + "я";
    let mut bytes = [0; NOTIFY_LEN];
    let len = notification(&text, &mut bytes);
    assert_eq!(len, 8 + 63);
    assert_eq!(notification_text(&bytes[..len]), Some(&text[..63]));
    let len = notification("hello", &mut bytes);
    assert_eq!(&bytes[..len], b"NOT1\x05\0\0\0hello");
}

#[test]
fn notification_receiver_clamps_untrusted_lengths_and_rejects_invalid_utf8() {
    assert_eq!(
        notification_text(b"NOT1\xff\xff\xff\xffhello"),
        Some("hello")
    );
    assert_eq!(notification_text(b"NOT1\x01\0\0\0\xff"), None);
    assert!(is_notification(b"NOT1\x01\0\0\0\xff"));
    assert!(is_notification(b"NOT1"));
    assert_eq!(notification_text(b"NOT1"), None);
    assert_eq!(notification_text(b"NOT1\0\0\0\0"), None);
}

#[test]
fn arbitrary_bounded_payloads_never_panic_or_decode_across_families() {
    let mut state = 0x12345678u32;
    let mut bytes = [0u8; 256];
    for len in 0..=bytes.len() {
        for byte in &mut bytes {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *byte = (state >> 24) as u8;
        }
        let input = &bytes[..len];
        let _ = (
            Capability::decode(input),
            Input::decode(input),
            is_reload(input),
            notification_text(input),
        );
    }
    assert_eq!(
        Input::decode(
            &Capability {
                handle: 1,
                advertised_bytes: 2,
                object: 3
            }
            .encode()
        ),
        None
    );
    assert_eq!(
        Capability::decode(
            &Input {
                kind: IN_KEY,
                x: 1,
                y: 2,
                value: 3
            }
            .encode()
        ),
        None
    );
}


#[test]
fn bootstrap_preserves_existing_bytes_and_rejects_partial_or_trailing_data() {
    use gui_protocol_v1::control::Bootstrap;
    let intro = Bootstrap { compositor: 0x04030201, client: 0x08070605 };
    assert_eq!(intro.encode(), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(Bootstrap::decode(&intro.encode()), Some(intro));
    for len in 0..8 { assert_eq!(Bootstrap::decode(&intro.encode()[..len]), None); }
    assert_eq!(Bootstrap::decode(&[0; 9]), None);
}
