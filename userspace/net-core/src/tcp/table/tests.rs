use super::super::PSH;
use super::*;
use alloc::vec;

const A: Ip = [192, 0, 2, 10];
const B: Ip = [192, 0, 2, 11];
const C: Ip = [192, 0, 2, 12];
const PORT: u16 = 8000;

fn endpoint(ip: Ip, port: u16) -> Endpoint {
    Endpoint { ip, port }
}

fn new(ip: Ip) -> Table {
    Table::new(ip, MTU, 9).unwrap()
}

fn packet(table: &mut Table, now: u64) -> Outbound {
    table.poll(now).unwrap().expect("expected TCP segment")
}

fn dispatch(dst: &mut Table, outgoing: &Outbound, now: u64) -> Ingress {
    dst.receive(
        outgoing.source.ip,
        outgoing.destination.ip,
        &outgoing.bytes,
        now,
    )
    .unwrap()
}

fn transfer(src: &mut Table, dst: &mut Table, now: u64) -> Ingress {
    dispatch(dst, &packet(src, now), now)
}

fn wire(
    source: Endpoint,
    destination: Endpoint,
    sequence: u32,
    ack: u32,
    flags: u8,
    data: &[u8],
) -> Outbound {
    Outbound {
        source,
        destination,
        bytes: Segment {
            sequence,
            acknowledgement: ack,
            flags,
            window: 4096,
            mss: None,
            data,
        }
        .encode(source, destination)
        .unwrap(),
    }
}

#[test]
fn delayed_egress_replaces_retries_and_rejects_acknowledged_packets() {
    let mut client = new(A);
    let remote = endpoint(B, PORT);
    let id = client.connect(remote, 0).unwrap();
    let original = packet(&mut client, 0);
    let retry = packet(&mut client, 1000);
    assert!(retry.same_pending_slot(&original));
    assert!(client.outbound_current(&retry));
    let local = retry.source;
    let syn = Segment::parse(local, remote, &retry.bytes).unwrap();
    let start = syn.sequence.wrapping_add(1);
    dispatch(&mut client, &wire(remote, local, 500, start, SYN | ACK, b""), 1001);
    assert!(!client.outbound_current(&retry));
    let ack = packet(&mut client, 1001);
    assert!(client.outbound_current(&ack));
    client.send(id, b"abc").unwrap();
    let data = packet(&mut client, 1001);
    assert!(!data.same_pending_slot(&ack));
    assert!(client.outbound_current(&data));
    dispatch(&mut client, &wire(remote, local, 501, start.wrapping_add(1), ACK, b"x"), 1002);
    assert!(!client.outbound_current(&data));
    assert!(!client.outbound_current(&ack));
    let new_ack = packet(&mut client, 1002);
    assert!(new_ack.same_pending_slot(&ack));
    assert!(client.outbound_current(&new_ack));
}

#[test]
fn deferred_egress_starts_rto_and_rtt_at_the_actual_handoff() {
    let mut client = new(A);
    let remote = endpoint(B, PORT);
    let id = client.connect(remote, 0).unwrap();
    let pending = client.poll_deferred(0).unwrap().unwrap();
    assert_eq!(client.deadline(), Some(super::super::EGRESS_WAIT_MS));
    assert_eq!(client.poll_deferred(4000), Ok(None));
    client.complete_egress(&pending, 4001, Ok(())).unwrap();
    assert_eq!(client.deadline(), Some(5001));
    assert_eq!(client.poll_deferred(5000), Ok(None));
    let local = pending.source;
    let syn = Segment::parse(local, remote, &pending.bytes).unwrap();
    dispatch(&mut client, &wire(remote, local, 500, syn.sequence.wrapping_add(1), SYN | ACK, b""), 5000);
    assert_eq!(client.state(id), Ok(State::Established));
    assert_eq!(client.stream(id).unwrap().retransmission.estimate, Some((999, 499)));
}

#[test]
fn deferred_retransmit_waits_for_handoff_or_a_terminal_route_error() {
    let mut client = new(A);
    let id = client.connect(endpoint(B, PORT), 0).unwrap();
    let syn = client.poll_deferred(0).unwrap().unwrap();
    client.complete_egress(&syn, 2500, Ok(())).unwrap();
    let retry = client.poll_deferred(3500).unwrap().unwrap();
    assert_eq!(retry, syn);
    assert_eq!(client.deadline(), Some(3500 + super::super::EGRESS_WAIT_MS));
    assert_eq!(client.poll_deferred(6500), Ok(None));
    client.complete_egress(&retry, 6500, Err(Error::Timeout)).unwrap();
    assert_eq!(client.failure(id), Ok(Some(StreamError::Packet(Error::Timeout))));
    assert_eq!(client.deadline(), None);
}

#[test]
fn deferred_egress_expires_when_the_transmit_queue_stalls() {
    let mut client = new(A);
    let id = client.connect(endpoint(B, PORT), 0).unwrap();
    let pending = client.poll_deferred(0).unwrap().unwrap();
    let expires = client.deadline().unwrap();
    assert_eq!(client.poll_deferred(expires - 1), Ok(None));
    assert!(client.outbound_current(&pending));
    assert_eq!(client.poll_deferred(expires), Ok(None));
    assert_eq!(client.failure(id), Ok(Some(StreamError::Timeout)));
    assert!(!client.outbound_current(&pending));
}

fn passive_handshake(
    server: &mut Table,
    remote: Endpoint,
    local: Endpoint,
    seq: u32,
    now: u64,
) -> (u32, u32) {
    assert_eq!(
        dispatch(server, &wire(remote, local, seq, 0, SYN, b""), now),
        Ingress::Admitted
    );
    let reply = packet(server, now);
    let synack = Segment::parse(reply.source, reply.destination, &reply.bytes).unwrap();
    assert_eq!(synack.flags, SYN | ACK);
    assert_eq!(synack.acknowledgement, seq.wrapping_add(1));
    let local_sequence = synack.sequence.wrapping_add(1);
    assert_eq!(
        dispatch(
            server,
            &wire(remote, local, seq.wrapping_add(1), local_sequence, ACK, b""),
            now
        ),
        Ingress::Delivered
    );
    (seq.wrapping_add(1), local_sequence)
}

#[test]
fn independently_demultiplex_full_duplex_streams_and_half_close() {
    let mut client = new(A);
    let mut server = new(B);
    let listener = server.listen(PORT, 2).unwrap();
    let one = client.connect(endpoint(B, PORT), 0).unwrap();
    let two = client.connect(endpoint(B, PORT), 0).unwrap();
    assert_ne!(
        client.endpoints(one).unwrap().0,
        client.endpoints(two).unwrap().0
    );
    assert_eq!(transfer(&mut client, &mut server, 0), Ingress::Admitted);
    assert_eq!(transfer(&mut client, &mut server, 0), Ingress::Admitted);
    assert_eq!(server.accept(listener), Err(StreamError::WouldBlock));
    for _ in 0..2 {
        assert_eq!(transfer(&mut server, &mut client, 1), Ingress::Delivered);
        assert_eq!(transfer(&mut client, &mut server, 1), Ingress::Delivered);
    }

    let one_server = server.accept(listener).unwrap();
    let two_server = server.accept(listener).unwrap();
    assert_eq!(server.listener_ready(listener), Ok(false));
    assert_eq!(
        server.endpoints(one_server).unwrap().1,
        client.endpoints(one).unwrap().0
    );
    assert_eq!(
        server.endpoints(two_server).unwrap().1,
        client.endpoints(two).unwrap().0
    );
    server.close_listener(listener).unwrap();
    assert_eq!(server.listener_ready(listener), Err(StreamError::Closed));

    let mut data = [0; 16];
    assert_eq!(
        server.read(one_server, &mut data),
        Err(StreamError::WouldBlock)
    );
    assert_eq!(client.send(one, b"first"), Ok(5));
    assert_eq!(client.send(two, b"second"), Ok(6));
    assert_eq!(transfer(&mut client, &mut server, 2), Ingress::Delivered);
    assert_eq!(transfer(&mut client, &mut server, 2), Ingress::Delivered);
    assert_eq!(server.read(one_server, &mut data), Ok(5));
    assert_eq!(&data[..5], b"first");
    assert_eq!(server.read(two_server, &mut data), Ok(6));
    assert_eq!(&data[..6], b"second");
    for _ in 0..2 {
        transfer(&mut server, &mut client, 3);
    }

    client.shutdown_write(one).unwrap();
    transfer(&mut client, &mut server, 4);
    let ready = server.readiness(one_server).unwrap();
    assert!(ready.readable && ready.writable && !ready.hangup && !ready.error);
    assert_eq!(server.read(one_server, &mut data), Ok(0));
    assert_eq!(server.state(two_server), Ok(State::Established));
    assert_eq!(server.send(one_server, b"after EOF"), Ok(9));
    transfer(&mut server, &mut client, 5);
    assert_eq!(client.read(one, &mut data), Ok(9));
    assert_eq!(&data[..9], b"after EOF");
    assert_eq!(client.state(one), Ok(State::FinWait2));
    transfer(&mut client, &mut server, 6);
    server.shutdown_write(one_server).unwrap();
    transfer(&mut server, &mut client, 7);
    transfer(&mut client, &mut server, 8);
    assert_eq!(server.state(one_server), Ok(State::Closed));
    assert!(server.readiness(one_server).unwrap().hangup);
    assert_eq!(server.read(one_server, &mut data), Ok(0));
    server.close(one_server).unwrap();
    assert_eq!(server.readiness(one_server), Err(StreamError::Closed));
}

#[test]
fn backlog_limits_duplicate_syns_stale_handles_and_elapsed_accept() {
    let mut server = new(B);
    let listener = server.listen(PORT, 1).unwrap();
    let other = server.listen(PORT + 1, MAX_PENDING).unwrap();
    let local = endpoint(B, PORT);
    let remote = endpoint(A, 5567);
    let first = wire(remote, local, 5, 0, SYN, b"");
    assert_eq!(dispatch(&mut server, &first, 10), Ingress::Admitted);
    assert_eq!(dispatch(&mut server, &first, 11), Ingress::Delivered);
    assert_eq!(
        dispatch(
            &mut server,
            &wire(endpoint(C, remote.port), local, 6, 0, SYN, b""),
            11
        ),
        Ingress::BacklogFull
    );
    for port in 5570..5573 {
        assert_eq!(
            dispatch(
                &mut server,
                &wire(endpoint(A, port), endpoint(B, PORT + 1), 7, 0, SYN, b""),
                11
            ),
            Ingress::Admitted
        );
    }
    assert_eq!(
        dispatch(
            &mut server,
            &wire(endpoint(A, 5573), endpoint(B, PORT + 1), 8, 0, SYN, b""),
            11
        ),
        Ingress::BacklogFull
    );
    assert_eq!(server.deadline(), Some(10 + ADMISSION_MS));
    assert_eq!(server.poll(11 + ADMISSION_MS), Ok(None));
    assert!(!server.listener_ready(other).unwrap());
    assert_eq!(
        dispatch(&mut server, &first, 11 + ADMISSION_MS),
        Ingress::Admitted
    );
    server.close_listener(listener).unwrap();
    let replacement = server.listen(PORT, 1).unwrap();
    assert_ne!(replacement, listener);
    assert!(server.accept(listener).is_err());

    let (remote_seq, local_seq) =
        passive_handshake(&mut server, remote, local, 34, 11 + ADMISSION_MS);
    assert_eq!(server.listener_ready(replacement), Ok(true));
    assert_eq!(
        dispatch(
            &mut server,
            &wire(
                remote,
                local,
                remote_seq,
                local_seq,
                ACK | PSH | FIN,
                b"queued"
            ),
            11 + ADMISSION_MS
        ),
        Ingress::Delivered
    );
    let stream = server.accept(replacement).unwrap();
    let mut data = [0; 8];
    assert_eq!(server.state(stream), Ok(State::CloseWait));
    assert_eq!(server.read(stream, &mut data), Ok(6));
    assert_eq!(&data[..6], b"queued");
    assert_eq!(server.read(stream, &mut data), Ok(0));
}

#[test]
fn pending_acknowledgement_order_and_malformed_packets_preserve_admission() {
    let mut server = new(B);
    let listener = server.listen(PORT, 2).unwrap();
    let local = endpoint(B, PORT);
    let first = endpoint(A, 7000);
    let second = endpoint(C, 7000);
    let mut bad = wire(first, local, 0xffff_ffff, 0, SYN, b"");
    bad.bytes[16] ^= 1;
    assert_eq!(
        server.receive(A, B, &bad.bytes, 7),
        Err(Error::Checksum.into())
    );
    assert_eq!(server.poll(7), Ok(None));
    assert_eq!(
        dispatch(
            &mut server,
            &wire(first, endpoint(B, PORT + 2), 1, 0, SYN, b""),
            7
        ),
        Ingress::Unmatched
    );
    assert_eq!(
        dispatch(&mut server, &wire(first, local, u32::MAX, 0, SYN, b""), 7),
        Ingress::Admitted
    );
    let first_synack = packet(&mut server, 7);
    let first_synack = Segment::parse(
        first_synack.source,
        first_synack.destination,
        &first_synack.bytes,
    )
    .unwrap();
    passive_handshake(&mut server, second, local, 200, 8);
    let accepted = server.accept(listener).unwrap();
    assert_eq!(server.endpoints(accepted).unwrap().1, second);
    assert!(!server.listener_ready(listener).unwrap());
    assert_eq!(
        dispatch(
            &mut server,
            &wire(
                first,
                local,
                0,
                first_synack.sequence.wrapping_add(1),
                ACK,
                b""
            ),
            9
        ),
        Ingress::Delivered
    );
    let accepted = server.accept(listener).unwrap();
    assert_eq!(server.endpoints(accepted).unwrap().1, first);
}

#[test]
fn capacity_ephemeral_reuse_and_timeout_retains_held_error() {
    let mut client = new(A);
    let listener = client.listen(FIRST_EPHEMERAL, 1).unwrap();
    assert_eq!(
        client.listen(FIRST_EPHEMERAL, 1),
        Err(Error::PortInUse.into())
    );
    client.next_port = u16::MAX;
    let first = client.connect(endpoint(B, PORT), 0).unwrap();
    assert_eq!(client.endpoints(first).unwrap().0.port, u16::MAX);
    let mut active = vec![first];
    for index in 0..MAX_STREAMS - 1 {
        let id = client.connect(endpoint(B, PORT), 0).unwrap();
        assert_eq!(
            client.endpoints(id).unwrap().0.port,
            FIRST_EPHEMERAL + index as u16 + 1
        );
        active.push(id);
    }
    assert_eq!(
        client.connect(endpoint(B, PORT), 0),
        Err(Error::Capacity.into())
    );
    let emitted: Vec<_> = (0..MAX_STREAMS)
        .map(|_| packet(&mut client, 0).source.port)
        .collect();
    assert!(emitted
        .iter()
        .zip(&active)
        .all(|(port, id)| *port == client.endpoints(*id).unwrap().0.port));
    for _ in 0..=super::super::MAX_RETRANSMITS {
        let deadline = client.deadline().unwrap();
        while client.poll(deadline).unwrap().is_some() {}
    }
    assert_eq!(client.deadline(), None);
    let ready = client.readiness(first).unwrap();
    assert!(ready.error && ready.hangup && !ready.writable);
    assert_eq!(client.read(first, &mut [0; 10]), Err(StreamError::Timeout));
    assert_eq!(
        client.connect(endpoint(B, PORT), client.now),
        Err(Error::Capacity.into())
    );
    client.close(first).unwrap();
    let next = client.connect(endpoint(C, PORT), client.now).unwrap();
    assert_ne!(first, next);
    assert_eq!(client.state(first), Err(StreamError::Closed));

    let mut replacement = new(A);
    replacement.listen(FIRST_EPHEMERAL, 1).unwrap();
    replacement.connect(endpoint(C, PORT), 0).unwrap();
    assert_eq!(replacement.state(first), Err(StreamError::Closed));
    assert_eq!(replacement.accept(listener), Err(StreamError::Closed));
    assert_eq!(client.poll(0), Err(StreamError::Clock));
}

#[test]
fn close_detaches_immediately_and_bounds_transport_retirement() {
    let mut server = new(B);
    let local = endpoint(B, PORT);
    let remote = endpoint(A, 9814);
    let listener = server.listen(PORT, 1).unwrap();
    passive_handshake(&mut server, remote, local, 10, 8);
    let stream = server.accept(listener).unwrap();
    server.close(stream).unwrap();
    assert_eq!(server.send(stream, b"stale"), Err(StreamError::Closed));
    assert_eq!(server.readiness(stream), Err(StreamError::Closed));
    assert_eq!(server.deadline(), Some(8 + DETACHED_MS));
    let fin = packet(&mut server, 8);
    assert_eq!(
        Segment::parse(fin.source, fin.destination, &fin.bytes)
            .unwrap()
            .flags,
        FIN | ACK
    );
    assert_eq!(server.poll(8 + DETACHED_MS), Ok(None));
    assert_eq!(server.deadline(), None);
    let syn = wire(remote, local, 30, 0, SYN, b"");
    assert_eq!(
        dispatch(&mut server, &syn, 8 + DETACHED_MS),
        Ingress::Admitted
    );
}
