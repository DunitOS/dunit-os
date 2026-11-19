use super::*;
use crate::tcp::{Connection, Endpoint, State, StreamError};
use crate::udp::Udp;

const LOCAL_IP: Ip = [192, 0, 2, 2];
const SERVER_IP: Ip = [192, 0, 2, 53];
const LOCAL_MAC: Mac = [2, 0, 0, 0, 0, 2];
const SERVER_MAC: Mac = [2, 0, 0, 0, 0, 53];
const SERVER: Address = Address {
    ip: SERVER_IP,
    port: 53,
};
const NAME: &[u8] = b"stream.example.test";

fn expected() -> Answer {
    Answer {
        address: AddressData::V4([192, 0, 2, 8]),
        ttl: 60,
    }
}

fn setup() -> (Peer, Resolver, Address, Question) {
    let mut peer = Peer::new(LOCAL_MAC, LOCAL_IP, [255, 255, 255, 0], MTU).unwrap();
    assert_eq!(
        peer.neighbors.resolve(SERVER_IP, 0),
        Ok(Resolution::Request)
    );
    peer.neighbors
        .accept(
            Arp {
                operation: 2,
                sender_mac: SERVER_MAC,
                sender_ip: SERVER_IP,
                target_mac: LOCAL_MAC,
                target_ip: LOCAL_IP,
            },
            SERVER_MAC,
            LOCAL_MAC,
            LOCAL_IP,
            0,
        )
        .unwrap();
    let mut resolver = Resolver::new_with_tcp(&mut peer, SERVER).unwrap();
    assert_eq!(resolver.start(NAME, Kind::A, 0), Ok(None));
    let Poll::Frame(frame) = resolver.poll(&mut peer, 0).unwrap() else {
        panic!("expected UDP query");
    };
    let frame = Ethernet::parse(&frame).unwrap();
    let ipv4 = Ipv4::parse(frame.payload).unwrap();
    let udp = Udp::parse(ipv4.source, ipv4.destination, ipv4.payload).unwrap();
    let from = Address {
        ip: LOCAL_IP,
        port: udp.source,
    };
    (peer, resolver, from, Question::parse(udp.data).unwrap())
}

fn deliver_udp(peer: &mut Peer, client: Address, body: &[u8], now: u64) {
    let udp = Udp::encode(SERVER, client, body).unwrap();
    let packet = Ipv4::encode_protocol(SERVER_IP, LOCAL_IP, 17, &udp).unwrap();
    let frame = Ethernet::encode(LOCAL_MAC, SERVER_MAC, 0x0800, &packet).unwrap();
    assert_eq!(peer.receive(&frame, now), Ok(None));
}

fn truncated(question: &Question) -> Vec<u8> {
    let mut body = question.encode().unwrap();
    put(&mut body, 2, 0x8380);
    put(&mut body, 6, 4); // Contents after the question are absent under TC.
    body
}

fn start_fallback(
    peer: &mut Peer,
    resolver: &mut Resolver,
    client: Address,
    question: &Question,
) -> Vec<u8> {
    deliver_udp(peer, client, &truncated(question), 2);
    let Poll::TcpFallback {
        server,
        query,
        deadline_ms,
    } = resolver.poll(peer, 2).unwrap()
    else {
        panic!("matched truncation must request a stream");
    };
    assert_eq!(server, SERVER);
    assert_eq!(deadline_ms, 2 + TCP_TIMEOUT_MS);
    assert_eq!(
        u16::from_be_bytes(query[..2].try_into().unwrap()) as usize,
        query.len() - 2
    );
    assert_eq!(Question::parse(&query[2..]).unwrap(), *question);
    assert_eq!(resolver.poll(peer, 3), Ok(Poll::Waiting));
    query
}

#[test]
fn framing_accepts_every_split_and_exact_size_only() {
    let payload = [0x42; TCP_DNS_LIMIT];
    let frame = stream::encode(&payload).unwrap();
    for split in 0..frame.len() {
        let mut reader = stream::ResponseFrame::default();
        assert_eq!(reader.feed(&frame[..split]), Ok(None));
        assert_eq!(reader.feed(&frame[split..]), Ok(Some(payload.as_slice())));
    }
    let mut oversized = stream::ResponseFrame::default();
    assert_eq!(oversized.feed(&[0x10]), Ok(None));
    assert_eq!(oversized.feed(&[0x01]), Err(Error::Length));
    assert_eq!(
        stream::ResponseFrame::default().feed(&[0, 11]),
        Err(Error::Length)
    );
    let mut trailing = frame;
    trailing.push(0xff);
    assert_eq!(
        stream::ResponseFrame::default().feed(&trailing),
        Err(Error::Length)
    );
}

#[test]
fn matched_truncation_and_connected_stream_are_required() {
    let (mut peer, mut resolver, client, q) = setup();
    let mut wrong = truncated(&q);
    put(&mut wrong, 0, q.id.wrapping_add(1));
    deliver_udp(&mut peer, client, &wrong, 1);
    assert_eq!(resolver.poll(&mut peer, 1), Err(Error::Unsolicited));
    assert_eq!(resolver.receive_tcp(SERVER, &[], 1), Err(Error::NotBound));

    start_fallback(&mut peer, &mut resolver, client, &q);
    assert_eq!(
        resolver.start(b"other.test", Kind::A, 3),
        Err(Error::Capacity)
    );
    let other = Address { port: 54, ..SERVER };
    assert_eq!(
        resolver.receive_tcp(other, &[0xff], 3),
        Err(Error::Unsolicited)
    );
    let body = q.response(&expected()).unwrap();
    let frame = stream::encode(&body).unwrap();
    assert_eq!(resolver.receive_tcp(SERVER, &frame[..1], 3), Ok(None));
    assert_eq!(resolver.receive_tcp(SERVER, &frame[1..27], 4), Ok(None));
    assert_eq!(
        resolver.receive_tcp(SERVER, &frame[27..], 5),
        Ok(Some(expected()))
    );
    assert_eq!(
        resolver.receive_tcp(SERVER, &frame, 6),
        Err(Error::NotBound)
    );
    assert_eq!(
        resolver.start(NAME, Kind::A, 1_005),
        Ok(Some(Answer {
            ttl: 59,
            ..expected()
        }))
    );
}

#[test]
fn stream_deadline_eof_and_malformed_messages_retire_pending_question() {
    for case in 0..5 {
        let (mut peer, mut resolver, client, q) = setup();
        start_fallback(&mut peer, &mut resolver, client, &q);
        let good = stream::encode(&q.response(&expected()).unwrap()).unwrap();
        match case {
            0 => {
                assert_eq!(resolver.receive_tcp(SERVER, &good[..3], 4), Ok(None));
                assert_eq!(resolver.tcp_eof(SERVER, 5), Err(Error::Length));
            }
            1 => {
                assert_eq!(
                    resolver.receive_tcp(SERVER, &good, 2 + TCP_TIMEOUT_MS),
                    Err(Error::Timeout)
                );
            }
            2 => {
                let wrong = Question::new(q.id, b"wrong.example.test", Kind::A).unwrap();
                let answer = stream::encode(&wrong.response(&expected()).unwrap()).unwrap();
                assert_eq!(
                    resolver.receive_tcp(SERVER, &answer, 5),
                    Err(Error::Unsolicited)
                );
            }
            3 => {
                let partial = stream::encode(&truncated(&q)).unwrap();
                assert_eq!(
                    resolver.receive_tcp(SERVER, &partial, 5),
                    Err(Error::Unsupported)
                );
            }
            4 => {
                let mut nx = q.encode().unwrap();
                put(&mut nx, 2, 0x8183);
                assert_eq!(
                    resolver.receive_tcp(SERVER, &stream::encode(&nx).unwrap(), 5),
                    Err(Error::NameNotFound)
                );
            }
            _ => unreachable!(),
        }
        assert_eq!(
            resolver.poll(&mut peer, 2 + TCP_TIMEOUT_MS),
            Err(Error::NotBound)
        );
        assert_eq!(resolver.start(NAME, Kind::A, 2 + TCP_TIMEOUT_MS), Ok(None));
    }
}

#[test]
fn real_tcp_segments_reassemble_response_larger_than_udp_limit() {
    let (mut peer, mut resolver, local, q) = setup();
    let query = start_fallback(&mut peer, &mut resolver, local, &q);
    let client_endpoint = Endpoint {
        ip: LOCAL_IP,
        port: local.port,
    };
    let server_endpoint = Endpoint {
        ip: SERVER_IP,
        port: SERVER.port,
    };
    let mut client = Connection::connect(client_endpoint, server_endpoint, 34, 276, 3).unwrap();
    let syn = client.poll(3).unwrap().unwrap();
    let mut server =
        Connection::accept(server_endpoint, client_endpoint, 82, 276, 3, &syn).unwrap();
    client
        .receive(SERVER_IP, LOCAL_IP, &server.poll(3).unwrap().unwrap(), 3)
        .unwrap();
    server
        .receive(LOCAL_IP, SERVER_IP, &client.poll(3).unwrap().unwrap(), 3)
        .unwrap();
    assert_eq!(client.state(), State::Established);
    assert_eq!(server.state(), State::Established);
    assert_eq!(client.send(&query), Ok(query.len()));
    server
        .receive(LOCAL_IP, SERVER_IP, &client.poll(4).unwrap().unwrap(), 4)
        .unwrap();
    let mut query_rx = [0; 512];
    let count = server.read(&mut query_rx).unwrap();
    assert_eq!(&query_rx[..count], &query);

    let mut body = q.response(&expected()).unwrap();
    put(&mut body, 10, 1); // One private-use additional record of 600 bytes.
    body.extend_from_slice(&[0xc0, 12, 0xff, 0x00, 0, 1, 0, 0, 0, 1, 2, 88]);
    body.extend_from_slice(&[0u8; 600]);
    assert_eq!(parse_response(&body, &q), Err(Error::Length));
    let response = stream::encode(&body).unwrap();
    assert_eq!(server.send(&response), Ok(response.len()));

    let mut answer = None;
    for now in 5..15 {
        let Some(segment) = server.poll(now).unwrap() else {
            break;
        };
        client.receive(SERVER_IP, LOCAL_IP, &segment, now).unwrap();
        let mut chunk = [0; 19];
        loop {
            match client.read(&mut chunk) {
                Ok(count) => {
                    let result = resolver.receive_tcp(SERVER, &chunk[..count], now).unwrap();
                    if result.is_some() {
                        answer = result;
                    }
                }
                Err(StreamError::WouldBlock) => break,
                error => panic!("unexpected read result {error:?}"),
            }
        }
        if let Some(ack) = client.poll(now).unwrap() {
            server.receive(LOCAL_IP, SERVER_IP, &ack, now).unwrap();
        }
        if answer.is_some() {
            break;
        }
    }
    assert_eq!(answer, Some(expected()));
}
