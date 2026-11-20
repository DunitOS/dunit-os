use super::*;
use crate::{checksum, put};

const A: Endpoint = Endpoint {
    ip: [192, 0, 2, 10],
    port: 4321,
};
const B: Endpoint = Endpoint {
    ip: [192, 0, 2, 11],
    port: 8000,
};

fn packet(seq: u32, ack: u32, flags: u8, window: u16, data: &[u8]) -> Vec<u8> {
    Segment {
        sequence: seq,
        acknowledgement: ack,
        flags,
        window,
        mss: None,
        data,
    }
    .encode(B, A)
    .unwrap()
}

fn repair_checksum(data: &mut [u8]) {
    put(data, 16, 0);
    let mut pseudo = Vec::from(B.ip);
    pseudo.extend_from_slice(&A.ip);
    pseudo.extend_from_slice(&[0, 6]);
    pseudo.extend_from_slice(&(data.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(data);
    put(data, 16, checksum(&pseudo));
}

fn pair() -> (Connection, Connection) {
    let mut client = Connection::connect(A, B, 100, MTU, 0).unwrap();
    let syn = client.poll(0).unwrap().unwrap();
    let mut server = Connection::accept(B, A, 500, MTU, 0, &syn).unwrap();
    pass(&mut server, &mut client, 0);
    pass(&mut client, &mut server, 0);
    (client, server)
}

fn pass(from: &mut Connection, to: &mut Connection, now: u64) {
    let (local, remote) = from.endpoints();
    let bytes = from.poll(now).unwrap().unwrap();
    to.receive(local.ip, remote.ip, &bytes, now).unwrap();
}

#[test]
fn checksum_header_flags_lengths_and_mss() {
    let payload = packet(50, 60, ACK | PSH, 4096, b"abcde");
    assert_eq!(Segment::parse(B, A, &payload).unwrap().data, b"abcde");
    for size in 0..wire::HEADER {
        assert_eq!(Segment::parse(B, A, &payload[..size]), Err(Error::Length));
    }
    assert_eq!(Segment::parse(A, B, &payload), Err(Error::Address));
    let wrong_ip = Endpoint {
        ip: [192, 0, 2, 20],
        ..B
    };
    assert_eq!(Segment::parse(wrong_ip, A, &payload), Err(Error::Checksum));
    let mut corrupted = payload;
    corrupted[23] ^= 128;
    assert_eq!(Segment::parse(B, A, &corrupted), Err(Error::Checksum));
    corrupted[12] = 0x51;
    repair_checksum(&mut corrupted);
    assert_eq!(Segment::parse(B, A, &corrupted), Err(Error::Header));
    for options in [
        [2, 1, 0, 1, 0, 0, 0, 0],
        [2, 4, 0, 0, 0, 0, 0, 0],
        [2, 4, 0, 8, 2, 4, 0, 8],
        [2, 9, 0, 8, 1, 1, 1, 1],
        [0, 1, 0, 0, 0, 0, 0, 0],
    ] {
        let mut bytes = packet(0, 0, SYN, 64, b"");
        bytes[12] = 0x70;
        bytes.extend_from_slice(&options);
        repair_checksum(&mut bytes);
        assert_eq!(Segment::parse(B, A, &bytes), Err(Error::Header));
    }
    let mut bytes = packet(0, 0, SYN, 64, b"");
    bytes[13] |= FIN;
    repair_checksum(&mut bytes);
    assert_eq!(Segment::parse(B, A, &bytes), Err(Error::Header));
}

#[test]
fn handshake_data_retries_wrap_and_half_close_large_stream() {
    assert_eq!(
        diagnostic::run().unwrap(),
        diagnostic::Report {
            bytes: 8192,
            retries: 2
        }
    );
    assert_eq!(
        diagnostic::exercise(2 * 1024 * 1024).unwrap().bytes,
        2 * 1024 * 1024
    );
}

#[test]
fn gaps_and_overlap_keep_delivery_order() {
    let (mut client, _) = pair();
    client
        .receive(B.ip, A.ip, &packet(503, 101, ACK, 64, b"cd"), 1)
        .unwrap();
    let reply = client.poll(1).unwrap().unwrap();
    assert_eq!(Segment::parse(A, B, &reply).unwrap().acknowledgement, 501);
    assert_eq!(client.read(&mut [0; 8]), Err(StreamError::WouldBlock));
    client
        .receive(B.ip, A.ip, &packet(501, 101, ACK, 64, b"abX"), 2)
        .unwrap();
    let reply = client.poll(2).unwrap().unwrap();
    assert_eq!(Segment::parse(A, B, &reply).unwrap().acknowledgement, 505);
    assert_eq!(client.receive.len(), 4);
    client
        .receive(B.ip, A.ip, &packet(503, 101, ACK, 64, b"cdef"), 3)
        .unwrap();
    let mut output = [0; 8];
    assert_eq!(client.read(&mut output).unwrap(), 6);
    assert_eq!(&output[..6], b"abcdef");
    let reply = client.poll(3).unwrap().unwrap();
    assert_eq!(Segment::parse(A, B, &reply).unwrap().acknowledgement, 507);
}

#[test]
fn reordering_across_wrap_defers_fin_and_rejects_data_outside_window() {
    let (mut client, _) = pair();
    client.receive_next = u32::MAX - 1;
    let outside = client.receive_next.wrapping_add(QUEUE_BYTES as u32 - 1);
    client.receive(B.ip, A.ip, &packet(outside, 101, ACK, 64, b"xx"), 1).unwrap();
    assert!(client.out_of_order.is_none());
    client.receive(B.ip, A.ip, &packet(0, 101, ACK | FIN, 64, b"cd"), 2).unwrap();
    client.receive(B.ip, A.ip, &packet(0, 101, ACK | FIN, 64, b"XX"), 2).unwrap();
    let ack = client.poll(2).unwrap().unwrap();
    let ack = Segment::parse(A, B, &ack).unwrap();
    assert_eq!(ack.acknowledgement, u32::MAX - 1);
    assert_eq!(ack.window as usize, QUEUE_BYTES);
    assert_eq!(client.read(&mut [0; 4]), Err(StreamError::WouldBlock));
    client.receive(B.ip, A.ip, &packet(u32::MAX - 1, 101, ACK, 64, b"ab"), 3).unwrap();
    assert_eq!(client.state(), State::CloseWait);
    let mut output = [0; 4];
    assert_eq!(client.read(&mut output), Ok(4));
    assert_eq!(&output, b"abcd");
    assert_eq!(client.read(&mut output), Ok(0));
    let ack = client.poll(3).unwrap().unwrap();
    assert_eq!(Segment::parse(A, B, &ack).unwrap().acknowledgement, 3);
}

#[test]
fn partial_and_invalid_ack_accounting() {
    let (mut client, _) = pair();
    assert_eq!(client.send(&[5; QUEUE_BYTES + 1]).unwrap(), QUEUE_BYTES);
    assert_eq!(client.send(b"busy"), Err(StreamError::WouldBlock));
    let bytes = client.poll(1).unwrap().unwrap();
    let first = Segment::parse(A, B, &bytes).unwrap();
    assert_eq!(first.data.len(), MAX_DATA);
    assert_eq!(
        client.receive(
            B.ip,
            A.ip,
            &packet(501, 101 + MAX_DATA as u32 + 1, ACK, 4096, b""),
            2
        ),
        Err(StreamError::Packet(Error::Unsolicited))
    );
    assert_eq!(client.send(b"busy"), Err(StreamError::WouldBlock));
    client
        .receive(B.ip, A.ip, &packet(501, 104, ACK, 4096, b""), 3)
        .unwrap();
    assert_eq!(client.send(b"again").unwrap(), 3);
    client
        .receive(B.ip, A.ip, &packet(501, 104, ACK, 4096, b""), 4)
        .unwrap();
    assert_eq!(client.send(b"busy"), Err(StreamError::WouldBlock));
    let bytes = client.poll(1003).unwrap().unwrap();
    let second = Segment::parse(A, B, &bytes).unwrap();
    assert_eq!(second.sequence, 104);
    assert_eq!(second.data, &first.data[3..]);
}

#[test]
fn zero_window_probe_and_bounded_growth() {
    let (mut client, mut server) = pair();
    let chunk = [3u8; QUEUE_BYTES];
    client.send(&chunk).unwrap();
    for ms in 1..=3 {
        pass(&mut client, &mut server, ms);
        pass(&mut server, &mut client, ms);
    }
    assert_eq!(server.receive.len(), QUEUE_BYTES);
    client.send(b"next").unwrap();
    assert!(client.poll(4).unwrap().is_none());
    let deadline = client.deadline().unwrap();
    assert_eq!(deadline, 1004);
    pass(&mut client, &mut server, deadline);
    assert_eq!(server.receive.len(), QUEUE_BYTES);
    pass(&mut server, &mut client, deadline);
    assert_eq!(server.read(&mut [0; QUEUE_BYTES]).unwrap(), QUEUE_BYTES);
    pass(&mut server, &mut client, deadline);
    let probe_retry = client.deadline().unwrap();
    assert_eq!(probe_retry, deadline);
    pass(&mut client, &mut server, probe_retry);
    pass(&mut server, &mut client, probe_retry);
    pass(&mut client, &mut server, probe_retry);
    let mut output = [0u8; 4];
    assert_eq!(server.read(&mut output).unwrap(), 4);
    assert_eq!(&output, b"next");
}

#[test]
fn answered_zero_window_probes_do_not_exhaust_retries() {
    let mut client = Connection::connect(A, B, 100, MTU, 0).unwrap();
    client.poll(0).unwrap();
    client.receive(B.ip, A.ip, &packet(500, 101, SYN | ACK, 0, b""), 1).unwrap();
    client.send(b"pending").unwrap();
    client.poll(1).unwrap();

    let mut last_delay = 0;
    for _ in 0..(MAX_RETRANSMITS + 4) {
        let now = client.deadline().unwrap();
        let probe = client.poll(now).unwrap().unwrap();
        let segment = Segment::parse(A, B, &probe).unwrap();
        assert_eq!((segment.sequence, segment.data), (101, b"p".as_slice()));
        client.receive(B.ip, A.ip, &packet(501, 101, ACK, 0, b""), now).unwrap();
        let delay = client.deadline().unwrap() - now;
        assert!(delay >= last_delay && delay <= MAX_RTO);
        last_delay = delay;
    }
    let reopen = client.now + 1;
    client.receive(B.ip, A.ip, &packet(501, 101, ACK, 4096, b""), reopen).unwrap();
    assert_eq!(client.deadline(), Some(reopen));
    let retry = client.poll(reopen).unwrap().unwrap();
    let segment = Segment::parse(A, B, &retry).unwrap();
    assert_eq!((segment.sequence, segment.data), (101, b"p".as_slice()));
    client.receive(B.ip, A.ip, &packet(501, 102, ACK, 4096, b""), reopen).unwrap();
    let rest = client.poll(reopen).unwrap().unwrap();
    let segment = Segment::parse(A, B, &rest).unwrap();
    assert_eq!((segment.sequence, segment.data), (102, b"ending".as_slice()));
}

#[test]
fn unanswered_zero_window_probes_time_out() {
    let (mut client, _) = pair();
    client.remote_window = 0;
    client.send(b"pending").unwrap();
    assert_eq!(client.poll(2).unwrap(), None);
    for _ in 0..=MAX_RETRANSMITS {
        let deadline = client.deadline().unwrap();
        assert!(client.poll(deadline).unwrap().is_some());
    }
    let deadline = client.deadline().unwrap();
    assert_eq!(client.poll(deadline), Err(StreamError::Timeout));
    assert_eq!(client.state(), State::Closed);
}

#[test]
fn simultaneous_open_and_simultaneous_close() {
    let mut a = Connection::connect(A, B, 100, MTU, 0).unwrap();
    let mut b = Connection::connect(B, A, 500, MTU, 0).unwrap();
    let sa = a.poll(0).unwrap().unwrap();
    pass(&mut b, &mut a, 0);
    b.receive(A.ip, B.ip, &sa, 0).unwrap();
    let sa = a.poll(0).unwrap().unwrap();
    pass(&mut b, &mut a, 0);
    b.receive(A.ip, B.ip, &sa, 0).unwrap();
    pass(&mut a, &mut b, 1);
    assert_eq!(
        (a.state(), b.state()),
        (State::Established, State::Established)
    );
    a.shutdown_write().unwrap();
    b.shutdown_write().unwrap();
    let fa = a.poll(2).unwrap().unwrap();
    pass(&mut b, &mut a, 2);
    b.receive(A.ip, B.ip, &fa, 2).unwrap();
    assert_eq!((a.state(), b.state()), (State::Closing, State::Closing));
    let aa = a.poll(3).unwrap().unwrap();
    pass(&mut b, &mut a, 3);
    b.receive(A.ip, B.ip, &aa, 3).unwrap();
    assert_eq!((a.state(), b.state()), (State::TimeWait, State::TimeWait));
}

#[test]
fn reset_requires_the_expected_sequence_and_syn_ack() {
    let (mut client, _) = pair();
    client
        .receive(B.ip, A.ip, &packet(502, 0, RST, 0, b""), 1)
        .unwrap();
    assert_eq!(client.state(), State::Established);
    let reply = client.poll(1).unwrap().unwrap();
    assert_eq!(Segment::parse(A, B, &reply).unwrap().acknowledgement, 501);
    client
        .receive(B.ip, A.ip, &packet(501, 0, RST, 0, b""), 2)
        .unwrap();
    assert_eq!(client.failure(), Some(StreamError::Reset));
    assert_eq!(client.read(&mut [0; 1]), Err(StreamError::Reset));
    let mut client = Connection::connect(A, B, 100, MTU, 0).unwrap();
    client.poll(0).unwrap().unwrap();
    client
        .receive(B.ip, A.ip, &packet(500, 0, RST, 0, b""), 1)
        .unwrap();
    assert_eq!(client.state(), State::SynSent);
    client
        .receive(B.ip, A.ip, &packet(500, 101, ACK | RST, 0, b""), 1)
        .unwrap();
    assert_eq!(client.failure(), Some(StreamError::Reset));
}

#[test]
fn timers_bound_retries_and_reject_regressing_clock() {
    let mut client = Connection::connect(A, B, 100, MTU, 100).unwrap();
    let syn = client.poll(100).unwrap().unwrap();
    assert_eq!(client.poll(98), Err(StreamError::Clock));
    for _ in 0..MAX_RETRANSMITS {
        let now = client.deadline().unwrap();
        assert!(client.poll(now - 1).unwrap().is_none());
        assert_eq!(client.poll(now).unwrap().unwrap(), syn);
    }
    let now = client.deadline().unwrap();
    assert_eq!(client.poll(now), Err(StreamError::Timeout));
    assert_eq!(client.state(), State::Closed);
    assert!(client.deadline().is_none());
}
