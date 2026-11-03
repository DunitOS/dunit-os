use gui_protocol_v1::transfer::{
    broker::{Body, Broker, Effect},
    *,
};
fn offer() -> Offer {
    Offer {
        formats: vec![Format::Text],
        actions: 3,
        preferred: Action::Copy,
    }
}
fn text() -> Bundle {
    Bundle {
        representations: vec![Representation {
            format: Format::Text,
            data: "привет 👋".as_bytes().to_vec(),
        }],
    }
}
fn msg(kind: Kind, request: u64, id: u64, a: u32, b: u32) -> Message {
    Message {
        kind,
        request,
        transfer: id,
        arg0: a,
        arg1: b,
    }
}
fn packets(out: &[Effect], kind: Kind) -> Vec<(u32, Message)> {
    out.iter()
        .filter_map(|e| match e {
            Effect::Send { to, message, .. } if message.kind == kind => Some((*to, *message)),
            _ => None,
        })
        .collect()
}
fn status(out: &[Effect]) -> Status {
    Status::from_u32(packets(out, Kind::Result)[0].1.arg0).unwrap()
}
fn setup() -> Broker {
    let mut b = Broker::new(100);
    for pid in 1..=3 {
        b.connect(pid);
    }
    b.focus(Some(1));
    b
}
fn publish(b: &mut Broker, request: u64) -> u64 {
    let out = b.request(
        1,
        msg(Kind::ClipboardOffer, request, 0, 0, 0),
        Some(Body::Offer(offer())),
        0,
        false,
    );
    assert!(packets(&out, Kind::Result).is_empty());
    let id = packets(&out, Kind::RequestData)[0].1.transfer;
    let out = b.request(
        1,
        msg(Kind::ProvideData, request + 1, id, 0, 0),
        Some(Body::Data(text())),
        1,
        false,
    );
    assert_eq!(packets(&out, Kind::Result).len(), 2);
    id
}
#[test]
fn strict_codecs_and_negotiation_reject_ambiguous_or_unbounded_data() {
    let m = msg(Kind::DragOffer, 7, 0, 0, 0);
    assert_eq!(Message::decode(&m.encode()), Ok(m));
    for n in 0..32 {
        assert!(Message::decode(&m.encode()[..n]).is_err());
    }
    let mut bad = m.encode();
    bad[6] = 1;
    assert!(Message::decode(&bad).is_err());
    assert!(Message::decode(&msg(Kind::Cancel, 0, 0, 0, 0).encode()).is_err());
    for name in [
        "/plain",
        "text/",
        "text//plain",
        "Text/plain",
        "text/plain;foo",
        "text/plain;=bar",
        "text/plain;x=",
    ] {
        assert!(Format::parse(name).is_err(), "{name}");
    }
    for bundle in [
        Bundle {
            representations: vec![],
        },
        Bundle {
            representations: vec![Representation {
                format: Format::Text,
                data: vec![0xff],
            }],
        },
        Bundle {
            representations: vec![Representation {
                format: Format::Text,
                data: vec![0],
            }],
        },
    ] {
        assert!(bundle.encode().is_err());
    }
    let mut encoded = text().encode().unwrap();
    encoded.push(0);
    assert!(Bundle::decode(&encoded).is_err());
    assert_eq!(Bundle::decode(&text().encode().unwrap()), Ok(text()));
    assert_eq!(Offer::decode(&offer().encode().unwrap()), Ok(offer()));
    let mut target = offer();
    target.formats = vec![Format::Rgba, Format::Text];
    assert_eq!(
        negotiate(&offer(), &target, Action::Copy),
        Some(Negotiated {
            format: 0,
            action: Action::Copy
        })
    );
    assert_eq!(negotiate(&offer(), &target, Action::Link), None);
    for (width, height, pixels) in [
        (0, 1, vec![]),
        (1, 1, vec![0; 3]),
        (u32::MAX, u32::MAX, vec![]),
    ] {
        assert!(Rgba {
            width,
            height,
            pixels
        }
        .encode()
        .is_err());
    }
    let rgba = Rgba {
        width: 2,
        height: 1,
        pixels: vec![1; 8],
    };
    assert_eq!(Rgba::decode(&rgba.encode().unwrap()), Ok(rgba));
}
#[test]
fn clipboard_is_acknowledged_only_after_complete_snapshot_and_survives_source_exit() {
    let mut b = setup();
    let id = publish(&mut b, 1);
    let out = b.disconnect(1);
    assert!(!out.contains(&Effect::Retire(id)));
    b.focus(Some(2));
    let out = b.request(
        2,
        msg(Kind::ClipboardRead, 1, 0, 0, 0),
        Some(Body::Offer(offer())),
        2,
        false,
    );
    let data = packets(&out, Kind::Data);
    assert_eq!(data.len(), 1);
    let read = data[0].1.transfer;
    assert_ne!(id, read);
    assert!(out.iter().any(|e|matches!(e,Effect::Send{body:Some(Body::Data(bundle)),custody:Some(c),..} if bundle==&text()&&*c==id)));
    assert_eq!(
        status(&b.request(2, msg(Kind::Complete, 2, read, 0, 0), None, 3, false)),
        Status::Ok
    );
    b.focus(Some(3));
    assert_eq!(
        packets(
            &b.request(
                3,
                msg(Kind::ClipboardRead, 1, 0, 0, 0),
                Some(Body::Offer(offer())),
                4,
                false
            ),
            Kind::Data
        )
        .len(),
        1
    );
}
#[test]
fn pending_publication_failure_keeps_previous_snapshot_and_cleans_escrow() {
    let mut b = setup();
    let old = publish(&mut b, 1);
    let out = b.request(
        1,
        msg(Kind::ClipboardOffer, 3, 0, 0, 0),
        Some(Body::Offer(offer())),
        10,
        false,
    );
    let pending = packets(&out, Kind::RequestData)[0].1.transfer;
    let out = b.tick(110);
    assert!(out.contains(&Effect::Retire(pending)));
    assert!(!out.contains(&Effect::Retire(old)));
    b.focus(Some(2));
    let out = b.request(2, msg(Kind::ClipboardQuery, 1, 0, 0, 0), None, 111, false);
    assert_eq!(packets(&out, Kind::Offer)[0].1.transfer, old);
}
#[test]
fn replaced_snapshot_retires_after_last_reader_and_timeout() {
    let mut b = setup();
    let old = publish(&mut b, 1);
    b.focus(Some(2));
    let out = b.request(
        2,
        msg(Kind::ClipboardRead, 1, 0, 0, 0),
        Some(Body::Offer(offer())),
        10,
        false,
    );
    let read = packets(&out, Kind::Data)[0].1.transfer;
    b.focus(Some(1));
    let new = publish(&mut b, 3);
    assert_ne!(new, old);
    let out = b.tick(110);
    assert!(out.contains(&Effect::Retire(old)));
    assert!(!out.contains(&Effect::Retire(new)));
    assert_eq!(
        status(&b.request(2, msg(Kind::Complete, 2, read, 0, 0), None, 111, false)),
        Status::Stale
    );
}
fn drag(b: &mut Broker) -> u64 {
    b.request(
        2,
        msg(Kind::RegisterTarget, 1, 0, 0, 0),
        Some(Body::Offer(offer())),
        0,
        false,
    );
    let out = b.request(
        1,
        msg(Kind::DragOffer, 1, 0, 0, 0),
        Some(Body::Offer(offer())),
        0,
        true,
    );
    packets(&out, Kind::Result)[0].1.transfer
}
#[test]
fn drag_is_lazy_requires_acceptance_and_can_cross_keyboard_focus() {
    let mut b = setup();
    let id = drag(&mut b);
    assert!(b.can_grant_source(1, id) == false);
    let out = b.pointer(Some((2, 11, 22)), Some(Action::Copy), 1);
    assert_eq!(packets(&out, Kind::DragEnter).len(), 1);
    assert!(packets(&out, Kind::RequestData).is_empty());
    b.focus(Some(2));
    assert_eq!(
        status(&b.request(2, msg(Kind::DragAccept, 2, id, 0, 1), None, 2, false)),
        Status::Ok
    );
    let out = b.drop_pointer(3);
    assert_eq!(packets(&out, Kind::RequestData).len(), 1);
    assert!(b.can_grant_source(1, id));
    assert!(!b.can_grant_source(3, id));
    let out = b.request(
        1,
        msg(Kind::ProvideData, 2, id, 0, 0),
        Some(Body::Data(text())),
        4,
        false,
    );
    assert_eq!(packets(&out, Kind::Data)[0].0, 2);
    assert!(!b.can_grant_source(1, id));
    let out = b.request(2, msg(Kind::Complete, 3, id, 0, 0), None, 5, false);
    assert_eq!(packets(&out, Kind::Finished).len(), 2);
    assert!(out.contains(&Effect::Retire(id)));
}
#[test]
fn wrong_target_forgery_replay_and_late_provider_cannot_deliver() {
    let mut b = setup();
    let id = drag(&mut b);
    b.pointer(Some((2, 1, 1)), Some(Action::Copy), 1);
    assert_eq!(
        status(&b.request(3, msg(Kind::DragAccept, 1, id, 0, 1), None, 2, false)),
        Status::Stale
    );
    assert_eq!(
        status(&b.request(2, msg(Kind::DragAccept, 2, id, 0, 2), None, 2, false)),
        Status::Denied
    );
    assert_eq!(
        status(&b.request(2, msg(Kind::DragAccept, 2, id, 0, 1), None, 2, false)),
        Status::Stale
    );
    let out = b.drop_pointer(3);
    assert!(packets(&out, Kind::RequestData).is_empty());
    let out = b.tick(103);
    assert!(out.contains(&Effect::Retire(id)));
    assert_eq!(
        status(&b.request(
            1,
            msg(Kind::ProvideData, 2, id, 0, 0),
            Some(Body::Data(text())),
            4,
            false
        )),
        Status::Stale
    );
}
#[test]
fn unsupported_target_escape_death_and_timeout_never_deliver_data() {
    for reason in 0..4 {
        let mut b = setup();
        let id = drag(&mut b);
        b.pointer(Some((2, 1, 1)), Some(Action::Copy), 1);
        let out = match reason {
            0 => {
                b.pointer(Some((3, 0, 0)), Some(Action::Copy), 2);
                b.drop_pointer(3)
            }
            1 => b.cancel_drag(Status::Cancelled),
            2 => b.disconnect(2),
            _ => b.tick(101),
        };
        assert!(packets(&out, Kind::Data).is_empty());
        assert!(packets(&out, Kind::RequestData).is_empty());
        assert!(out.contains(&Effect::Retire(id)));
        assert_eq!(b.active_drag(), None);
    }
}
#[test]
fn format_changes_and_modifiers_require_fresh_target_acceptance() {
    let mut b = setup();
    let id = drag(&mut b);
    b.pointer(Some((2, 1, 1)), Some(Action::Copy), 1);
    b.request(2, msg(Kind::DragAccept, 2, id, 0, 1), None, 2, false);
    let out = b.pointer(Some((2, 1, 1)), Some(Action::Move), 3);
    assert_eq!(packets(&out, Kind::DragEnter).len(), 1);
    assert_eq!(b.active_drag().unwrap().3, None);
    let out = b.drop_pointer(4);
    assert!(packets(&out, Kind::RequestData).is_empty());
    let out = b.tick(104);
    assert!(out.contains(&Effect::Retire(id)));
}
#[test]
fn focus_and_capture_gate_exports_and_clipboard_reads() {
    let mut b = setup();
    assert_eq!(
        status(&b.request(
            2,
            msg(Kind::ClipboardOffer, 1, 0, 0, 0),
            Some(Body::Offer(offer())),
            0,
            false
        )),
        Status::Denied
    );
    assert_eq!(
        status(&b.request(
            1,
            msg(Kind::DragOffer, 1, 0, 0, 0),
            Some(Body::Offer(offer())),
            0,
            false
        )),
        Status::Busy
    );
    assert_eq!(
        status(&b.request(
            2,
            msg(Kind::ClipboardRead, 2, 0, 0, 0),
            Some(Body::Offer(offer())),
            0,
            false
        )),
        Status::Denied
    );
}
#[test]
fn file_move_completion_requires_native_success_for_every_item() {
    let mut b = setup();
    let files = FileList {
        items: vec![
            FileItem {
                path: "/tmp/a".into(),
                capability: 41,
                kind: FileKind::File,
            },
            FileItem {
                path: "/tmp/b".into(),
                capability: 42,
                kind: FileKind::Directory,
            },
        ],
    };
    let offer = Offer {
        formats: vec![Format::Files],
        actions: 3,
        preferred: Action::Move,
    };
    let out = b.request(
        1,
        msg(Kind::ClipboardOffer, 1, 0, 0, 0),
        Some(Body::Offer(offer.clone())),
        0,
        false,
    );
    let custody = packets(&out, Kind::RequestData)[0].1.transfer;
    let data = Bundle {
        representations: vec![Representation {
            format: Format::Files,
            data: files.encode().unwrap(),
        }],
    };
    b.request(
        1,
        msg(Kind::ProvideData, 2, custody, 0, 0),
        Some(Body::Data(data)),
        1,
        false,
    );
    b.focus(Some(2));
    let out = b.request(
        2,
        msg(Kind::ClipboardRead, 1, 0, 0, 0),
        Some(Body::Offer(offer)),
        2,
        false,
    );
    let id = packets(&out, Kind::Data)[0].1.transfer;
    assert_eq!(
        status(&b.request(2, msg(Kind::Complete, 2, id, 0, 0), None, 3, false)),
        Status::Busy
    );
    assert!(b.can_grant_destination(2, id));
    assert!(!b.can_grant_destination(3, id));
    for item in 0..2 {
        let movement = FileMove {
            item,
            destination: 55,
            leaf: format!("leaf-{item}"),
        };
        assert!(
            matches!(b.request(2,msg(Kind::FileMove,3+u64::from(item),id,0,0),Some(Body::Move(movement)),4,false)[0],Effect::Move{custody:c,..} if c==custody)
        );
        let effect = b.moved(2, 3 + u64::from(item), id, item, Status::Ok);
        assert_eq!(status(&[effect]), Status::Ok);
        let effect = b.moved(2, 8, id, item, Status::Ok);
        assert_eq!(status(&[effect]), Status::Stale);
    }
    let out = b.request(2, msg(Kind::Complete, 5, id, 0, 0), None, 6, false);
    assert_eq!(status(&out), Status::Ok);
    assert!(out.contains(&Effect::Retire(custody)));
    assert_eq!(
        status(&b.request(2, msg(Kind::ClipboardQuery, 6, 0, 0, 0), None, 7, false)),
        Status::Stale
    );
}

#[test]
fn uri_text_paths_preserve_utf8_and_cannot_reference_remote_or_injected_names() {
    let paths = PathList {
        paths: vec![
            "/tmp/a b'я".into(),
            "/tmp/$HOME;$(x)".into(),
            "/tmp/percent%#?".into(),
        ],
    };
    let bytes = paths.encode().unwrap();
    assert_eq!(PathList::decode(&bytes), Ok(paths));
    for bytes in [
        b"file://remote/tmp/a".as_slice(),
        b"file:///tmp/%0Aevil",
        b"file:///tmp/%00",
        b"file:///tmp/%FF",
        b"file:///tmp/%",
        b"file:///tmp/%GG",
        b"file:///tmp/../escape",
        b"file:///tmp/a?query",
        b"file:///tmp/a#fragment",
    ] {
        assert!(PathList::decode(bytes).is_err());
    }
    let bundle = Bundle {
        representations: vec![Representation {
            format: Format::Custom(URI_LIST_MIME.into()),
            data: b"file://remote/tmp/a".to_vec(),
        }],
    };
    assert!(bundle.encode().is_err());
    let bytes = encode_blob(42, 7, b"content").unwrap();
    assert_eq!(decode_blob(&bytes), Ok((42, 7, b"content".as_slice())));
    let mut bad = bytes.clone();
    bad[0] = 0;
    assert!(decode_blob(&bad).is_err());
}

#[test]
fn natural_drag_selects_supported_target_action_without_downgrading_explicit_move() {
    let mut b = setup();
    let mut copy_only = offer();
    copy_only.actions = 1;
    b.request(
        2,
        msg(Kind::RegisterTarget, 1, 0, 0, 0),
        Some(Body::Offer(copy_only)),
        0,
        false,
    );
    let mut source = offer();
    source.preferred = Action::Move;
    let out = b.request(
        1,
        msg(Kind::DragOffer, 1, 0, 0, 0),
        Some(Body::Offer(source)),
        0,
        true,
    );
    let id = packets(&out, Kind::Result)[0].1.transfer;
    let out = b.pointer(Some((2, 1, 1)), None, 1);
    assert_eq!(
        packets(&out, Kind::DragEnter)[0].1.arg1,
        Action::Copy as u32
    );
    b.request(2, msg(Kind::DragAccept, 2, id, 0, 1), None, 2, false);
    let out = b.pointer(Some((2, 1, 1)), Some(Action::Move), 3);
    assert_eq!(packets(&out, Kind::DragLeave).len(), 1);
    assert!(packets(&out, Kind::DragEnter).is_empty());
    assert!(packets(&b.drop_pointer(4), Kind::RequestData).is_empty());
}
#[test]
fn an_existing_pointer_capture_can_start_a_drag_across_workspace_focus_change() {
    let mut b = setup();
    b.focus(Some(2));
    let out = b.request(
        1,
        msg(Kind::DragOffer, 1, 0, 0, 0),
        Some(Body::Offer(offer())),
        0,
        true,
    );
    assert_eq!(status(&out), Status::Ok);
    assert_eq!(
        status(&b.request(
            1,
            msg(Kind::ClipboardOffer, 2, 0, 0, 0),
            Some(Body::Offer(offer())),
            1,
            false
        )),
        Status::Denied
    );
}

#[test]
fn target_can_reject_a_dropped_transfer_before_provider_data_arrives() {
    let mut b = setup();
    let id = drag(&mut b);
    b.pointer(Some((2, 1, 1)), Some(Action::Copy), 1);
    b.request(2, msg(Kind::DragAccept, 2, id, 0, 1), None, 2, false);
    assert_eq!(packets(&b.drop_pointer(3), Kind::RequestData).len(), 1);
    assert_eq!(
        status(&b.request(
            2,
            msg(Kind::Complete, 3, id, Status::Ok as u32, 0),
            None,
            4,
            false
        )),
        Status::Stale
    );
    let out = b.request(
        2,
        msg(Kind::Complete, 4, id, Status::Cancelled as u32, 0),
        None,
        5,
        false,
    );
    assert_eq!(status(&out), Status::Ok);
    assert!(out.contains(&Effect::Retire(id)));
    assert_eq!(packets(&out, Kind::Finished).len(), 2);
    let late = b.request(
        1,
        msg(Kind::ProvideData, 2, id, 0, 0),
        Some(Body::Data(text())),
        6,
        false,
    );
    assert_eq!(status(&late), Status::Stale);
    assert!(packets(&late, Kind::Data).is_empty());
}

#[test]
fn dock_launch_waits_for_drop_and_the_exact_new_peers_opener_registration() {
    let mut b = setup();
    let id = drag(&mut b);
    let opening = Offer {
        formats: vec![Format::Text],
        actions: 1,
        preferred: Action::Copy,
    };
    let out = b.pointer_opener(8, &opening, None, 1);
    assert!(packets(&out, Kind::RequestData).is_empty());
    assert_eq!(b.active_opener(), Some(8));
    assert!(!b.can_grant_source(1, id));
    let out = b.drop_pointer(2);
    assert_eq!(
        out,
        vec![Effect::Launch {
            opener: 8,
            transfer: id
        }]
    );
    assert!(!b.can_grant_source(1, id));
    assert!(b.expect_opener(id, 4, 3));
    b.connect(4);
    let wrong = b.request(
        3,
        msg(Kind::RegisterOpener, 1, 0, 0, 0),
        Some(Body::Offer(opening.clone())),
        4,
        false,
    );
    assert!(packets(&wrong, Kind::RequestData).is_empty());
    assert!(!b.can_grant_source(1, id));
    let ready = b.request(
        4,
        msg(Kind::RegisterOpener, 1, 0, 0, 0),
        Some(Body::Offer(opening)),
        5,
        false,
    );
    assert_eq!(packets(&ready, Kind::OpenOffer)[0].0, 4);
    assert_eq!(packets(&ready, Kind::RequestData)[0].0, 1);
    assert!(b.can_grant_source(1, id));
    let data = b.request(
        1,
        msg(Kind::ProvideData, 2, id, 0, 0),
        Some(Body::Data(text())),
        6,
        false,
    );
    assert_eq!(packets(&data, Kind::OpenData)[0].0, 4);
    assert!(packets(&data, Kind::Data).is_empty());
    let done = b.request(
        4,
        msg(Kind::Complete, 2, id, Status::Ok as u32, 0),
        None,
        7,
        false,
    );
    assert_eq!(packets(&done, Kind::Finished).len(), 2);
    assert!(done.contains(&Effect::Retire(id)));
}
#[test]
fn dock_hover_escape_failed_launch_and_missing_opt_in_never_request_data() {
    for cause in 0..4 {
        let mut b = setup();
        let id = drag(&mut b);
        b.pointer_opener(8, &offer(), None, 1);
        let out = if cause == 0 {
            b.cancel_drag(Status::Cancelled)
        } else {
            b.drop_pointer(2);
            match cause {
                1 => b.fail_opener(id, Status::TargetGone),
                2 => {
                    b.expect_opener(id, 4, 3);
                    b.disconnect(4)
                }
                _ => b.tick(103),
            }
        };
        assert!(packets(&out, Kind::RequestData).is_empty());
        assert!(out.contains(&Effect::Retire(id)));
        assert!(!b.can_grant_source(1, id));
        assert_eq!(b.active_drag(), None);
    }
}
#[test]
fn dock_cannot_turn_explicit_move_or_unsupported_types_into_document_open() {
    for action in [Some(Action::Move), Some(Action::Link)] {
        let mut b = setup();
        let id = drag(&mut b);
        b.pointer_opener(8, &offer(), action, 1);
        assert_eq!(b.active_opener(), None);
        assert!(b.drop_pointer(2).contains(&Effect::Retire(id)));
    }
    let mut b = setup();
    let id = drag(&mut b);
    let unsupported = Offer {
        formats: vec![Format::Files],
        actions: 1,
        preferred: Action::Copy,
    };
    b.pointer_opener(8, &unsupported, None, 1);
    assert!(b.drop_pointer(2).contains(&Effect::Retire(id)));
}

#[test]
fn pointer_up_before_target_round_trip_freezes_target_and_waits_without_requesting_data() {
    let mut b = setup();
    let id = drag(&mut b);
    b.pointer(Some((2, 11, 22)), Some(Action::Copy), 1);
    assert!(b.drop_pointer(2).is_empty());
    assert!(!b.dragging());
    assert!(!b.can_grant_source(1, id));
    assert!(b
        .pointer(Some((3, 33, 44)), Some(Action::Copy), 3)
        .is_empty());
    let out = b.request(2, msg(Kind::DragAccept, 2, id, 0, 1), None, 4, false);
    assert_eq!(packets(&out, Kind::DragDrop)[0].0, 2);
    assert_eq!(packets(&out, Kind::RequestData)[0].0, 1);
    assert!(b.can_grant_source(1, id));
}
#[test]
fn rejecting_a_released_target_cleans_up_without_contacting_provider() {
    let mut b = setup();
    let id = drag(&mut b);
    b.pointer(Some((2, 1, 1)), Some(Action::Copy), 1);
    assert!(b.drop_pointer(2).is_empty());
    let out = b.request(
        2,
        msg(Kind::DragAccept, 2, id, ALL_FORMATS, 0),
        None,
        3,
        false,
    );
    assert!(packets(&out, Kind::RequestData).is_empty());
    assert!(out.contains(&Effect::Retire(id)));
}
