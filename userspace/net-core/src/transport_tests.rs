use super::dns::*;
use super::udp::*;
use super::*;
const IA: Ip = [192, 0, 2, 1];
const IB: Ip = [192, 0, 2, 2];
fn peer() -> Peer {
    Peer::new([2, 0, 0, 0, 0, 1], IA, [255, 255, 255, 0], MTU).unwrap()
}
fn address(ip: Ip, port: u16) -> Address {
    Address { ip, port }
}
fn deliver(peer: &mut Peer, port: u16, data: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    let udp = Udp::encode(address(IB, 53), address(IA, port), data).unwrap();
    let ip = Ipv4::encode_protocol(IB, IA, 17, &udp).unwrap();
    peer.receive(
        &Ethernet::encode([2, 0, 0, 0, 0, 1], [2, 0, 0, 0, 0, 2], 0x0800, &ip).unwrap(),
        0,
    )
}
#[test]
fn shared_udp_dns_diagnostic() {
    assert_eq!(
        transport_diagnostic::run().unwrap(),
        transport_diagnostic::Report {
            datagrams: 19,
            dns_answers: 4,
            rejected: 8
        }
    );
}
#[test]
fn udp_checksum_lengths_zero_and_odd_bytes() {
    for size in [0, 1, 2, MAX_PAYLOAD] {
        let data = alloc::vec![0xab; size];
        let mut b = Udp::encode(address(IA, 7), address(IB, 9), &data).unwrap();
        assert_ne!(word(&b, 6), 0);
        assert_eq!(Udp::parse(IA, IB, &b).unwrap().data, data);
        assert!(matches!(
            Udp::parse([192, 0, 2, 3], IA, &b),
            Err(Error::Checksum)
        ));
        if size != 0 {
            b[8] ^= 1;
            assert!(matches!(Udp::parse(IA, IB, &b), Err(Error::Checksum)));
        }
        put(&mut b, 6, 0);
        assert!(Udp::parse(IA, IB, &b).is_ok());
        let len = b.len();
        put(&mut b, 4, (len - 1) as u16);
        assert!(matches!(Udp::parse(IA, IB, &b), Err(Error::Length)));
    }
    for len in 0..8 {
        assert!(Udp::parse(IA, IB, &alloc::vec![0;len]).is_err());
    }
    assert_eq!(
        Udp::encode(address(IA, 1), address(IB, 0), b"").unwrap_err(),
        Error::Address
    );
    assert_eq!(
        Udp::encode(
            address(IA, 1),
            address(IB, 1),
            &alloc::vec![0;MAX_PAYLOAD+1]
        )
        .unwrap_err(),
        Error::Length
    );
    // Find the one's-complement negative-zero case; encode must use 0xffff.
    let mut found = false;
    for n in 0..=u16::MAX {
        let b = Udp::encode(address(IA, 7), address(IB, 9), &n.to_be_bytes()).unwrap();
        if word(&b, 6) == 0xffff {
            assert!(Udp::parse(IA, IB, &b).is_ok());
            found = true;
            break;
        }
    }
    assert!(found);
}
#[test]
fn udp_binding_queue_connect_and_close() {
    let mut p = peer();
    let port = p.udp_bind(0).unwrap();
    assert!(port >= 49152);
    assert_eq!(p.udp_bind(port), Err(Error::PortInUse));
    for n in 1..ENDPOINTS {
        p.udp_bind(n as u16).unwrap();
    }
    assert_eq!(p.udp_bind(0), Err(Error::Capacity));
    assert_eq!(p.udp_recv(0), Err(Error::NotBound));
    for n in 0..QUEUE_DEPTH {
        deliver(&mut p, port, &[n as u8]).unwrap();
    }
    assert_eq!(deliver(&mut p, port, b"x"), Err(Error::Capacity));
    assert!(p.udp_ready(port).unwrap());
    for n in 0..QUEUE_DEPTH {
        assert_eq!(p.udp_recv(port).unwrap().data, [n as u8]);
    }
    assert_eq!(p.udp_recv(port), Err(Error::WouldBlock));
    deliver(&mut p, port, b"old").unwrap();
    p.udp_connect(port, Some(address(IB, 54))).unwrap();
    assert!(!p.udp_ready(port).unwrap());
    assert_eq!(
        deliver(&mut p, port, b"other port"),
        Err(Error::Unsolicited)
    );
    assert_eq!(
        p.udp_send(port, Some(address(IB, 53)), b"", 0),
        Err(Error::Address)
    );
    assert_eq!(
        p.udp_connect(port, Some(address([198, 51, 100, 1], 53))),
        Err(Error::NoRoute)
    );
    p.udp_connect(port, None).unwrap();
    deliver(&mut p, port, b"abc").unwrap();
    assert_eq!(p.udp_recv_into(port, &mut []).unwrap().original_len, 3);
    p.udp_close(port).unwrap();
    assert_eq!(p.udp_recv(port), Err(Error::NotBound));
    p.udp_bind(port).unwrap();
    assert!(!p.udp_ready(port).unwrap());
    assert_eq!(deliver(&mut p, 999, b"x"), Err(Error::NotBound));
}
#[test]
fn udp_route_mtu_errors_precede_arp() {
    let mut p = Peer::new([2, 0, 0, 0, 0, 1], IA, [255, 255, 255, 0], 68).unwrap();
    let port = p.udp_bind(7).unwrap();
    assert_eq!(
        p.udp_send(port, Some(address(IB, 9)), &[0; 41], 0),
        Err(Error::Length)
    );
    assert_eq!(p.neighbors.state(IB, 0).unwrap(), None);
    assert_eq!(
        p.udp_send(port, Some(address([198, 51, 100, 1], 9)), b"", 0),
        Err(Error::NoRoute)
    );
    assert!(matches!(
        p.udp_send(port, Some(address(IB, 9)), &[0; 40], 0).unwrap(),
        Send::Neighbor(_)
    ));
    assert_eq!(
        p.udp_send(port, Some(address(IB, 9)), b"", 1),
        Ok(Send::Waiting)
    );
}
fn question() -> Question {
    Question::new(42, b"www.example.test", Kind::A).unwrap()
}
fn answer() -> Answer {
    Answer {
        address: AddressData::V4([203, 0, 113, 1]),
        ttl: 30,
    }
}
#[test]
fn dns_validation_truncation_ids_names_and_counts() {
    let q = question();
    let b = q.response(&answer()).unwrap();
    assert_eq!(Question::parse(&q.encode().unwrap()).unwrap(), q);
    assert_eq!(parse_response(&b, &q).unwrap(), answer());
    for len in 0..b.len() {
        assert!(parse_response(&b[..len], &q).is_err(), "len {len}");
    }
    for (offset, value, error) in [
        (0, 1, Error::Unsolicited),
        (2, 0x83, Error::Unsupported),
        (3, 0xc0, Error::Header),
        (7, 33, Error::Capacity),
        (12, 0xc0, Error::Header),
    ] {
        let mut bad = b.clone();
        bad[offset] = value;
        assert_eq!(
            parse_response(&bad, &q).unwrap_err(),
            error,
            "offset {offset}"
        );
    }
    let mut bad = b.clone();
    bad[13] = b'x';
    assert_eq!(parse_response(&bad, &q), Err(Error::Unsolicited));
    let mut nx = q.encode().unwrap();
    put(&mut nx, 2, 0x8183);
    assert_eq!(parse_response(&nx, &q), Err(Error::NameNotFound));
    nx.push(0);
    assert_eq!(parse_response(&nx, &q), Err(Error::Length));
    for name in [b"".as_slice(), b"a..b", b"bad space", b"."] {
        assert!(Question::new(1, name, Kind::A).is_err());
    }
    assert!(Question::new(1, &[b'a'; 64], Kind::A).is_err());
    let q6 = Question::new(5, b"ipv6.test", Kind::Aaaa).unwrap();
    let a6 = Answer {
        address: AddressData::V6([0x20; 16]),
        ttl: 0,
    };
    assert_eq!(parse_response(&q6.response(&a6).unwrap(), &q6), Ok(a6));
    assert_eq!(
        q.response(&Answer {
            address: AddressData::V6([0; 16]),
            ttl: 1
        }),
        Err(Error::Protocol)
    );
}
fn rr(b: &mut Vec<u8>, owner: &[u8], kind: u16, ttl: u32, data: &[u8]) {
    b.extend_from_slice(owner);
    b.extend_from_slice(&kind.to_be_bytes());
    b.extend_from_slice(&1u16.to_be_bytes());
    b.extend_from_slice(&ttl.to_be_bytes());
    b.extend_from_slice(&(data.len() as u16).to_be_bytes());
    b.extend_from_slice(data);
}
#[test]
fn dns_cname_compression_ttl_chains_and_loops() {
    let q = question();
    let mut b = q.encode().unwrap();
    put(&mut b, 2, 0x8180);
    put(&mut b, 6, 2);
    let alias_offset = b.len() + 12;
    rr(&mut b, &[0xc0, 12], 5, 7, b"\x05alias\xc0\x10");
    rr(
        &mut b,
        &[0xc0, alias_offset as u8],
        1,
        30,
        &[203, 0, 113, 1],
    );
    assert_eq!(parse_response(&b, &q), Ok(Answer { ttl: 7, ..answer() }));
    let mut looped = q.encode().unwrap();
    put(&mut looped, 2, 0x8180);
    put(&mut looped, 6, 1);
    rr(&mut looped, &[0xc0, 12], 5, 8, &[0xc0, 12]);
    assert_eq!(parse_response(&looped, &q), Err(Error::Header));
    // Name consists of a label followed by a pointer back to itself.
    let mut cyclic = q.encode().unwrap();
    put(&mut cyclic, 2, 0x8180);
    put(&mut cyclic, 6, 1);
    let offset = cyclic.len();
    rr(
        &mut cyclic,
        &[1, b'x', 0xc0, offset as u8],
        1,
        1,
        &[1, 2, 3, 4],
    );
    assert!(parse_response(&cyclic, &q).is_err());
    // Additional-section address cannot satisfy an answer.
    let mut additional = q.response(&answer()).unwrap();
    put(&mut additional, 6, 0);
    put(&mut additional, 10, 1);
    assert_eq!(parse_response(&additional, &q), Err(Error::NameNotFound));
}
fn warm(p: &mut Peer) {
    assert_eq!(p.neighbors.resolve(IB, 0).unwrap(), Resolution::Request);
    p.neighbors
        .accept(
            Arp {
                operation: 2,
                sender_mac: [2, 0, 0, 0, 0, 2],
                sender_ip: IB,
                target_mac: [2, 0, 0, 0, 0, 1],
                target_ip: IA,
            },
            [2, 0, 0, 0, 0, 2],
            [2, 0, 0, 0, 0, 1],
            IA,
            0,
        )
        .unwrap();
}
fn query_frame(resolver: &mut Resolver, p: &mut Peer, now: u64) -> (u16, Question) {
    let frame = match resolver.poll(p, now).unwrap() {
        Poll::Frame(f) => f,
        _ => panic!("expected query"),
    };
    let eth = Ethernet::parse(&frame).unwrap();
    let ip = Ipv4::parse(eth.payload).unwrap();
    let udp = Udp::parse(ip.source, ip.destination, ip.payload).unwrap();
    (udp.source, Question::parse(udp.data).unwrap())
}
#[test]
fn dns_transaction_wrong_id_retries_ttl_nxdomain_and_close() {
    let mut p = peer();
    warm(&mut p);
    let mut resolver = Resolver::new(&mut p, address(IB, 53)).unwrap();
    assert_eq!(resolver.start(b"www.example.test", Kind::A, 0), Ok(None));
    assert_eq!(
        resolver.start(b"other.test", Kind::A, 0),
        Err(Error::Capacity)
    );
    let (port, q) = query_frame(&mut resolver, &mut p, 0);
    assert_eq!(resolver.poll(&mut p, 999), Ok(Poll::Waiting));
    let mut wrong = q.response(&answer()).unwrap();
    put(&mut wrong, 0, q.id.wrapping_add(1));
    deliver(&mut p, port, &wrong).unwrap();
    assert_eq!(resolver.poll(&mut p, 999), Err(Error::Unsolicited));
    let (_, retry) = query_frame(&mut resolver, &mut p, 1000);
    assert_eq!(retry, q);
    deliver_at(&mut p, port, &q.response(&answer()).unwrap(), 1000).unwrap();
    assert_eq!(resolver.poll(&mut p, 1000), Ok(Poll::Answer(answer())));
    assert_eq!(
        resolver.start(b"WWW.EXAMPLE.TEST.", Kind::A, 2000),
        Ok(Some(Answer {
            ttl: 29,
            ..answer()
        }))
    );
    assert_eq!(
        resolver.start(b"www.example.test", Kind::A, 31000),
        Ok(None)
    );
    let arp = match resolver.poll(&mut p, 31000).unwrap() {
        Poll::Frame(f) => f,
        _ => panic!("ARP"),
    };
    assert_eq!(Ethernet::parse(&arp).unwrap().kind, 0x0806);
    p.neighbors
        .accept(
            Arp {
                operation: 2,
                sender_mac: [2, 0, 0, 0, 0, 2],
                sender_ip: IB,
                target_mac: [2, 0, 0, 0, 0, 1],
                target_ip: IA,
            },
            [2, 0, 0, 0, 0, 2],
            [2, 0, 0, 0, 0, 1],
            IA,
            31000,
        )
        .unwrap();
    let (_, next) = query_frame(&mut resolver, &mut p, 31000);
    assert_ne!(next.id, q.id);
    query_frame(&mut resolver, &mut p, 32000);
    query_frame(&mut resolver, &mut p, 33000);
    // Even an otherwise valid response at the deadline must time out.
    deliver_at(&mut p, port, &next.response(&answer()).unwrap(), 34000).unwrap();
    assert_eq!(resolver.poll(&mut p, 34000), Err(Error::Timeout));
    assert_eq!(resolver.poll(&mut p, 34000), Err(Error::NotBound));
    // A queued old response cannot satisfy the next query before transmission.
    resolver.start(b"missing.test", Kind::A, 34000).unwrap();
    assert_eq!(resolver.poll(&mut p, 34000), Err(Error::Unsolicited));
    let (_, missing) = query_frame(&mut resolver, &mut p, 34000);
    let mut nx = missing.encode().unwrap();
    put(&mut nx, 2, 0x8183);
    deliver_at(&mut p, port, &nx, 34000).unwrap();
    assert_eq!(resolver.poll(&mut p, 34000), Err(Error::NameNotFound));
    assert_eq!(
        resolver.start(b"missing.test", Kind::A, 33999),
        Err(Error::Header)
    );
    resolver.close(&mut p).unwrap();
    assert_eq!(p.udp_ready(port), Err(Error::NotBound));
}
fn deliver_at(peer: &mut Peer, port: u16, data: &[u8], now: u64) -> Result<Option<Vec<u8>>, Error> {
    let udp = Udp::encode(address(IB, 53), address(IA, port), data).unwrap();
    let ip = Ipv4::encode_protocol(IB, IA, 17, &udp).unwrap();
    peer.receive(
        &Ethernet::encode([2, 0, 0, 0, 0, 1], [2, 0, 0, 0, 0, 2], 0x0800, &ip).unwrap(),
        now,
    )
}

#[test]
fn dns_negative_cache_uses_soa_minimum_and_question_scope() {
    for nxdomain in [true, false] {
        let mut peer = peer();
        warm(&mut peer);
        let mut resolver = Resolver::new(&mut peer, address(IB, 53)).unwrap();
        resolver.start(b"missing.test", Kind::A, 0).unwrap();
        let (port, question) = query_frame(&mut resolver, &mut peer, 0);
        let mut negative = question.encode().unwrap();
        put(&mut negative, 2, if nxdomain { 0x8183 } else { 0x8180 });
        put(&mut negative, 8, 1);
        put(&mut negative, 10, 1);
        // SOA owner "test", two compressed names, TTL 30 and MINIMUM 2.
        negative.extend_from_slice(&[0xc0, 20, 0, 6, 0, 1, 0, 0, 0, 30, 0, 24]);
        negative.extend_from_slice(&[0xc0, 20, 0xc0, 20]);
        negative.extend_from_slice(&[0; 19]);
        negative.push(2);
        // EDNS OPT uses a root owner and does not contribute cache policy.
        negative.extend_from_slice(&[0, 0, 41, 2, 0, 0, 0, 0, 0, 0, 0]);
        let mut malformed = negative.clone();
        malformed[question.encode().unwrap().len() + 11] = 23;
        deliver_at(&mut peer, port, &malformed, 1).unwrap();
        assert_eq!(resolver.poll(&mut peer, 1), Err(Error::Length));
        deliver_at(&mut peer, port, &negative, 2).unwrap();
        assert_eq!(resolver.poll(&mut peer, 2), Err(Error::NameNotFound));
        assert_eq!(resolver.start(b"MISSING.TEST.", Kind::A, 2001), Err(Error::NameNotFound));
        if nxdomain {
            assert_eq!(resolver.start(b"missing.test", Kind::Aaaa, 2001), Err(Error::NameNotFound));
            assert_eq!(resolver.start(b"missing.test", Kind::A, 2002), Ok(None));
        } else {
            assert_eq!(resolver.start(b"missing.test", Kind::Aaaa, 2001), Ok(None));
        }
    }
}

#[test]
fn dns_failover_on_timeout_ignores_primary_queue_and_restarts_with_primary() {
    let mut peer = peer();
    warm(&mut peer);
    let primary = address(IB, 53);
    let backup = address(IB, 5353);
    let mut resolver = Resolver::with_secondary(&mut peer, primary, backup, false).unwrap();
    resolver.start(b"missing.test", Kind::A, 0).unwrap();
    let (port, original) = query_frame(&mut resolver, &mut peer, 0);
    for now in [1000, 2000] {
        assert_eq!(query_frame(&mut resolver, &mut peer, now), (port, original.clone()));
    }
    deliver_at(&mut peer, port, &original.response(&answer()).unwrap(), 3000).unwrap();
    let (new_port, retry) = query_frame(&mut resolver, &mut peer, 3000);
    assert_eq!(new_port, port);
    assert_ne!(retry.id, original.id);
    assert_eq!(retry.name, original.name);
    assert_eq!(
        deliver_at(&mut peer, port, &retry.response(&answer()).unwrap(), 3000),
        Err(Error::Unsolicited)
    );
    assert_eq!(resolver.poll(&mut peer, 3000), Ok(Poll::Waiting));
    query_frame(&mut resolver, &mut peer, 4000);
    query_frame(&mut resolver, &mut peer, 5000);
    assert_eq!(resolver.poll(&mut peer, 6000), Err(Error::Timeout));
    resolver.start(b"other.test", Kind::A, 6000).unwrap();
    let (_, current) = query_frame(&mut resolver, &mut peer, 6000);
    deliver_at(&mut peer, port, &current.response(&answer()).unwrap(), 6000).unwrap();
    assert_eq!(resolver.poll(&mut peer, 6000), Ok(Poll::Answer(answer())));
}

#[test]
fn dns_unresolved_neighbor_does_not_leave_transaction_open() {
    let mut peer = peer();
    let mut resolver = Resolver::new(&mut peer, address(IB, 53)).unwrap();
    resolver.start(b"missing.test", Kind::A, 0).unwrap();
    assert!(matches!(resolver.poll(&mut peer, 0), Ok(Poll::Frame(_))));
    // The caller missed several ARP wakeups. This must still be terminal.
    assert_eq!(resolver.poll(&mut peer, 6000), Err(Error::Timeout));
    assert_eq!(resolver.start(b"other.test", Kind::A, 6000), Ok(None));
}

#[test]
fn dns_bounded_mutation_corpus_never_panics() {
    let q = question();
    let seed = q.response(&answer()).unwrap();
    let mut state = 1u32;
    for size in 0..=600 {
        let mut b = alloc::vec![0;size];
        for v in &mut b {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *v = (state >> 24) as u8;
        }
        let _ = Question::parse(&b);
        let _ = parse_response(&b, &q);
    }
    for offset in 0..seed.len() {
        for value in 0..=255 {
            let mut b = seed.clone();
            b[offset] = value;
            let _ = parse_response(&b, &q);
        }
    }
}

#[test]
fn dns_zero_ttl_and_bounded_fifo_cache() {
    let mut p = peer();
    warm(&mut p);
    let mut resolver = Resolver::new(&mut p, address(IB, 53)).unwrap();
    resolver.start(b"zero.test", Kind::A, 0).unwrap();
    let (port, q) = query_frame(&mut resolver, &mut p, 0);
    let zero = Answer { ttl: 0, ..answer() };
    deliver(&mut p, port, &q.response(&zero).unwrap()).unwrap();
    assert_eq!(resolver.poll(&mut p, 0), Ok(Poll::Answer(zero)));
    assert_eq!(resolver.start(b"zero.test", Kind::A, 0), Ok(None));
    let (port, q) = query_frame(&mut resolver, &mut p, 0);
    deliver(&mut p, port, &q.response(&answer()).unwrap()).unwrap();
    resolver.poll(&mut p, 0).unwrap();
    for n in 0..8 {
        let name = [b'a' + n, b'.', b't'];
        assert_eq!(resolver.start(&name, Kind::A, 0), Ok(None));
        let (port, q) = query_frame(&mut resolver, &mut p, 0);
        deliver(&mut p, port, &q.response(&answer()).unwrap()).unwrap();
        assert_eq!(resolver.poll(&mut p, 0), Ok(Poll::Answer(answer())));
    }
    assert_eq!(resolver.start(b"a.t", Kind::A, 0), Ok(Some(answer())));
    assert_eq!(resolver.start(b"zero.test", Kind::A, 0), Ok(None));
    resolver.close(&mut p).unwrap();
}
#[test]
fn udp_optional_zero_source_port_and_exact_ip_length() {
    let udp = Udp::encode(address(IB, 0), address(IA, 7), b"odd").unwrap();
    assert_eq!(Udp::parse(IB, IA, &udp).unwrap().source, 0);
    let mut p = peer();
    p.udp_bind(7).unwrap();
    let ip = Ipv4::encode_protocol(IB, IA, 17, &udp).unwrap();
    let frame = Ethernet::encode([2, 0, 0, 0, 0, 1], [2, 0, 0, 0, 0, 2], 0x0800, &ip).unwrap();
    p.receive(&frame, 0).unwrap();
    let received = p.udp_recv(7).unwrap();
    assert_eq!(received.source.port, 0);
    assert_eq!(received.data, b"odd");
    // IPv4 payload has an extra byte beyond the UDP length: reject mismatch.
    let mut extra = udp;
    extra.push(0);
    let ip = Ipv4::encode_protocol(IB, IA, 17, &extra).unwrap();
    let frame = Ethernet::encode([2, 0, 0, 0, 0, 1], [2, 0, 0, 0, 0, 2], 0x0800, &ip).unwrap();
    assert_eq!(p.receive(&frame, 0), Err(Error::Length));
    assert_eq!(p.udp_recv(7), Err(Error::WouldBlock));
}
