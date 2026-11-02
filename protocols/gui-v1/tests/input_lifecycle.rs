//! Negotiated live-control extension: old peers retain their exact contract.
mod common;
use common::*;
use gui_protocol_v1::{control::*, server::Server, wire::*, Request};

#[test]
fn lifecycle_negotiation_is_opt_in_and_does_not_change_advertised_formats() {
    let mut server = Server::new();
    for features in [
        0,
        FEATURE_ARGB8888,
        FEATURE_INPUT_LIFECYCLE,
        FEATURE_ARGB8888 | FEATURE_INPUT_LIFECYCLE,
    ] {
        let conn = server.connect().unwrap();
        assert_eq!(server.negotiated_features(conn), None);
        let request = Request::Hello {
            min_minor: 0,
            max_minor: 0,
            offered_features: features | (1 << 63),
            required_features: features,
        }
        .encode(0, 1);
        let reply = server.deliver(conn, &request, None);
        assert_eq!(u64at(&reply[0], 48), features);
        assert_eq!(server.negotiated_features(conn), Some(features));
        let formats = FORMATS_XRGB8888
            | if features & FEATURE_ARGB8888 != 0 {
                FORMATS_ARGB8888
            } else {
                0
            };
        assert_eq!(u32at(&reply[0], 56), formats);
        server.deliver(conn, &Request::Disconnect.encode(0, 2), None);
        assert_eq!(server.negotiated_features(conn), None);
    }
    assert_eq!(server.negotiated_features(u32::MAX), None);
}

#[test]
fn unnegotiated_lifecycle_never_produces_new_kinds_or_pointer_bytes() {
    for kind in [IN_KEY_UP, IN_FOCUS_IN, IN_FOCUS_OUT] {
        let message = Input {
            kind,
            x: 0,
            y: 0,
            value: 0,
        };
        assert_eq!(message.encode_for_features(0), None);
        assert_eq!(message.encode_for_features(FEATURE_ARGB8888), None);
        assert_eq!(Input::decode(&message.encode()), None);
        assert_eq!(Input::decode_for_features(&message.encode(), 0), None);
    }
    for kind in [IN_MOVE, IN_DOWN, IN_UP, IN_LEAVE, IN_SCROLL] {
        for modifiers in 0..16 {
            for button in [0, 1, 2, 255] {
                let message = Input {
                    kind,
                    x: -12,
                    y: 43,
                    value: pointer_value(button, modifiers).unwrap(),
                };
                let legacy = Input {
                    value: button as u32,
                    ..message
                };
                assert_eq!(message.encode_for_features(0), Some(legacy.encode()));
                assert_eq!(
                    message.encode_for_features(FEATURE_ARGB8888),
                    Some(legacy.encode())
                );
                assert_eq!(
                    Input::decode(&message.encode_for_features(0).unwrap()),
                    Some(legacy)
                );
                let bytes = message
                    .encode_for_features(FEATURE_INPUT_LIFECYCLE)
                    .unwrap();
                let received = Input::decode_for_features(&bytes, FEATURE_INPUT_LIFECYCLE).unwrap();
                assert_eq!(received, message);
                assert_eq!(received.pointer_button(), button);
                assert_eq!(received.pointer_modifiers(), modifiers);
            }
        }
    }
}

#[test]
fn lifecycle_kinds_keep_twenty_byte_envelope_and_have_golden_payloads() {
    let keyup = Input {
        kind: IN_KEY_UP,
        x: 0x39,
        y: 3,
        value: 0,
    };
    let bytes = keyup.encode_for_features(FEATURE_INPUT_LIFECYCLE).unwrap();
    assert_eq!(
        bytes,
        [b'I', b'N', b'P', b'1', 7, 0, 0, 0, 0x39, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        Input::decode_for_features(&bytes, FEATURE_INPUT_LIFECYCLE),
        Some(keyup)
    );
    for kind in [IN_FOCUS_IN, IN_FOCUS_OUT] {
        let focus = Input {
            kind,
            x: 0,
            y: 0,
            value: 0,
        };
        let bytes = focus.encode_for_features(FEATURE_INPUT_LIFECYCLE).unwrap();
        assert_eq!(bytes.len(), INPUT_LEN);
        assert_eq!(&bytes[..4], b"INP1");
        assert_eq!(bytes[4], kind);
        assert_eq!(&bytes[5..], &[0; 15]);
        assert_eq!(
            Input::decode_for_features(&bytes, FEATURE_INPUT_LIFECYCLE),
            Some(focus)
        );
    }
}

#[test]
fn negotiated_payload_validation_rejects_reserved_bits_modifiers_and_partial_data() {
    let valid = Input {
        kind: IN_KEY_UP,
        x: 0x1c,
        y: 15,
        value: 0,
    };
    let bytes = valid.encode();
    for len in 0..INPUT_LEN {
        assert_eq!(
            Input::decode_for_features(&bytes[..len], FEATURE_INPUT_LIFECYCLE),
            None
        );
    }
    let mut extra = bytes.to_vec();
    extra.push(0);
    assert_eq!(
        Input::decode_for_features(&extra, FEATURE_INPUT_LIFECYCLE),
        None
    );
    for offset in 5..8 {
        let mut invalid = bytes;
        invalid[offset] = 1;
        assert_eq!(
            Input::decode_for_features(&invalid, FEATURE_INPUT_LIFECYCLE),
            None
        );
    }
    for invalid in [
        Input { kind: 255, ..valid },
        Input {
            kind: IN_KEY,
            y: -1,
            ..valid
        },
        Input {
            kind: IN_KEY,
            y: 16,
            ..valid
        },
        Input { y: 16, ..valid },
        Input { value: 1, ..valid },
        Input {
            kind: IN_FOCUS_IN,
            x: 1,
            y: 0,
            value: 0,
        },
        Input {
            kind: IN_FOCUS_OUT,
            x: 0,
            y: 1,
            value: 0,
        },
        Input {
            kind: IN_FOCUS_OUT,
            x: 0,
            y: 0,
            value: 1,
        },
        Input {
            kind: IN_DOWN,
            x: 0,
            y: 0,
            value: 1 << 12,
        },
        Input {
            kind: IN_MOVE,
            x: 0,
            y: 0,
            value: 1 << 31,
        },
    ] {
        assert_eq!(
            Input::decode_for_features(&invalid.encode(), FEATURE_INPUT_LIFECYCLE),
            None
        );
        assert_eq!(invalid.encode_for_features(FEATURE_INPUT_LIFECYCLE), None);
    }
    for modifiers in 16..=255 {
        assert_eq!(pointer_value(0, modifiers), None);
    }
}

#[test]
fn fallback_decode_retains_historical_short_signals_and_unconstrained_legacy_fields() {
    for kind in [IN_QUIT, IN_LEAVE] {
        let input = Input {
            kind,
            x: 1,
            y: 2,
            value: 3,
        };
        assert_eq!(
            Input::decode_for_features(&input.encode()[..8], 0),
            Some(Input {
                kind,
                x: 0,
                y: 0,
                value: 0
            })
        );
        assert_eq!(
            Input::decode_for_features(&input.encode()[..8], FEATURE_INPUT_LIFECYCLE),
            None
        );
    }
    let old = Input {
        kind: IN_KEY,
        x: -7,
        y: 0x123456,
        value: 0x80,
    };
    assert_eq!(Input::decode_for_features(&old.encode(), 0), Some(old));
    assert_eq!(
        Input::decode_for_features(&old.encode(), FEATURE_ARGB8888),
        Some(old)
    );
}

#[test]
fn lifecycle_decoder_is_total_for_arbitrary_bounded_packets() {
    let mut state = 0x105afeu32;
    let mut bytes = [0u8; 256];
    for len in 0..=bytes.len() {
        for byte in &mut bytes {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *byte = (state >> 24) as u8;
        }
        let _ = Input::decode_for_features(&bytes[..len], FEATURE_INPUT_LIFECYCLE);
    }
    // Ensure every kind is exercised with a valid family header too.
    for kind in 0..=255 {
        let input = Input {
            kind,
            x: 0,
            y: 0,
            value: 0,
        };
        if let Some(decoded) = Input::decode_for_features(&input.encode(), FEATURE_INPUT_LIFECYCLE)
        {
            assert!(matches!(
                kind,
                IN_MOVE
                    | IN_DOWN
                    | IN_UP
                    | IN_LEAVE
                    | IN_KEY
                    | IN_SCROLL
                    | IN_KEY_UP
                    | IN_FOCUS_IN
                    | IN_QUIT
                    | IN_FOCUS_OUT
            ));
            assert_eq!(decoded, input);
        }
    }
}
