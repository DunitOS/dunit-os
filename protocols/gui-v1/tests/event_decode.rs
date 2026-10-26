use gui_protocol_v1::{Event, Header, Opcode, HEADER_LEN};

#[test]
fn every_server_event_roundtrips_and_rejects_truncation_and_trailing_data() {
    let events = vec![
        Event::Welcome {
            request: 1,
            client_id: 2,
            features: 1,
            formats: 3,
        },
        Event::Result { request: 2 },
        Event::Error {
            request: 3,
            code: 4,
            fatal: 1,
        },
        Event::Configure {
            width: 640,
            height: 480,
            scale: 1,
            state: 2,
        },
        Event::RequestClose,
        Event::BufferRelease { commit: 4 },
        Event::FrameDone {
            commit: 5,
            time_ns: 123,
            status: 0,
        },
        Event::PointerEnter { x: -4, y: 7 },
        Event::PointerLeave,
        Event::PointerMotion {
            time_ns: 6,
            x: -3,
            y: -2,
        },
        Event::PointerButton {
            time_ns: 7,
            button: 1,
            pressed: 1,
        },
        Event::PointerAxis {
            time_ns: 8,
            dx: -1,
            dy: 3,
        },
        Event::KeyEnter { modifiers: 4 },
        Event::KeyLeave,
        Event::Key {
            time_ns: 9,
            usage: 12,
            pressed: 1,
            modifiers: 3,
            repeat: 0,
        },
        Event::TextInput {
            time_ns: 10,
            text: "Привет 🦀".into(),
        },
    ];
    for event in events {
        let packet = event.encode(1, 5);
        let header = Header::decode(&packet).unwrap();
        let opcode = Opcode::from_u16(header.opcode_raw).unwrap();
        let payload = &packet[HEADER_LEN..];
        assert_eq!(Event::decode(opcode, payload).unwrap(), event);
        for end in 0..payload.len() {
            assert!(Event::decode(opcode, &payload[..end]).is_err());
        }
        let mut extra = payload.to_vec();
        extra.push(0);
        assert!(Event::decode(opcode, &extra).is_err());
    }
}
#[test]
fn rejects_reserved_fields_invalid_text_and_wrong_direction() {
    for event in [
        Event::Welcome {
            request: 1,
            client_id: 1,
            features: 0,
            formats: 1,
        },
        Event::FrameDone {
            commit: 2,
            time_ns: 3,
            status: 0,
        },
        Event::KeyEnter { modifiers: 0 },
    ] {
        let mut packet = event.encode(0, 1);
        *packet.last_mut().unwrap() = 1;
        assert!(Event::decode(event.opcode(), &packet[HEADER_LEN..]).is_err());
    }
    let mut packet = Event::TextInput {
        time_ns: 1,
        text: "a".into(),
    }
    .encode(1, 1);
    *packet.last_mut().unwrap() = 0xff;
    assert!(Event::decode(Opcode::TextInput, &packet[HEADER_LEN..]).is_err());
    *packet.last_mut().unwrap() = 0;
    assert!(Event::decode(Opcode::TextInput, &packet[HEADER_LEN..]).is_err());
    assert!(Event::decode(Opcode::Disconnect, &[]).is_err());
}
