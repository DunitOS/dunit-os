use gui_protocol_v1::{control::*, wire::*};
#[test]
fn requests_and_events_require_exact_negotiated_contract() {
    let r = PointerRequest {
        request: 7,
        mode: PointerMode::Locked,
    };
    assert_eq!(PointerRequest::decode(&r.encode()), Some(r));
    let mut b = r.encode();
    b[12] = 1;
    assert_eq!(PointerRequest::decode(&b), None);
    assert_eq!(PointerRequest::decode(&r.encode()[..12]), None);
    for bad in [0, u32::MAX] {
        let mut b = r.encode();
        b[4..8].copy_from_slice(&bad.to_le_bytes());
        assert_eq!(PointerRequest::decode(&b), None);
    }
    let mut b = r.encode();
    b[8] = 3;
    assert_eq!(PointerRequest::decode(&b), None);
    let f = FEATURE_INPUT_LIFECYCLE | FEATURE_POINTER_MODE;
    for input in [
        Input {
            kind: IN_RELATIVE,
            x: -999,
            y: 123,
            value: pointer_value(3, 2).unwrap(),
        },
        Input {
            kind: IN_POINTER_MODE,
            x: 7,
            y: 0,
            value: 2,
        },
        Input {
            kind: IN_POINTER_MODE,
            x: 0,
            y: -4,
            value: 0,
        },
    ] {
        let bytes = input.encode();
        assert_eq!(Input::decode_for_features(&bytes, f), Some(input));
        assert_eq!(
            Input::decode_for_features(&bytes, FEATURE_INPUT_LIFECYCLE),
            None
        );
        assert_eq!(input.encode_for_features(0), None);
        assert_eq!(Input::decode(&bytes), None);
    }
    assert!(Input {
        kind: IN_POINTER_MODE,
        x: 1,
        y: 0,
        value: 3
    }
    .encode_for_features(f)
    .is_none());
}

#[test]
fn pointer_feature_does_not_require_unrelated_lifecycle_negotiation() {
    let input = Input {
        kind: IN_RELATIVE,
        x: -5,
        y: 9,
        value: 3,
    };
    assert_eq!(
        Input::decode_for_features(&input.encode(), FEATURE_POINTER_MODE),
        Some(input)
    );
    let focus = Input {
        kind: IN_FOCUS_OUT,
        x: 0,
        y: 0,
        value: 0,
    };
    assert_eq!(
        Input::decode_for_features(&focus.encode(), FEATURE_POINTER_MODE),
        None
    );
}
