use super::*;
const A: Mac = [2, 0, 0, 0, 0, 1];
const B: Mac = [2, 0, 0, 0, 0, 2];
const IA: Ip = [192, 0, 2, 1];
const IB: Ip = [192, 0, 2, 2];
fn ip_packet() -> Vec<u8> {
    Ipv4::encode(IA, IB, &Echo::encode(false, 7, 9, b"odd").unwrap()).unwrap()
}
fn repair_ip(b: &mut [u8]) {
    put(b, 10, 0);
    let c = checksum(&b[..20]);
    put(b, 10, c);
}
fn arp_reply() -> Arp {
    Arp {
        operation: 2,
        sender_mac: B,
        sender_ip: IB,
        target_mac: A,
        target_ip: IA,
    }
}
#[test]
fn native_scenario_checks_wire_results() {
    assert_eq!(
        diagnostic::run(),
        Ok(diagnostic::Report {
            echo_replies: 3,
            arp_requests: 5,
            rejected: 5
        })
    );
}
#[test]
fn ethernet_bounds_types_and_zero_padding() {
    let b = Ethernet::encode(B, A, 0x0800, &[7; 20]).unwrap();
    assert_eq!(b.len(), 60);
    assert_eq!(&b[34..], &[0; 26]);
    for size in 0..60 {
        assert_eq!(Ethernet::parse(&b[..size]).unwrap_err(), Error::Length);
    }
    assert_eq!(
        Ethernet::parse(&alloc::vec![0; 1515]).unwrap_err(),
        Error::Length
    );
    let mut b = b;
    put(&mut b, 12, 1500);
    assert_eq!(Ethernet::parse(&b).unwrap_err(), Error::EtherType);
    put(&mut b, 12, 0x8100);
    assert_eq!(Ethernet::parse(&b).unwrap_err(), Error::EtherType);
    put(&mut b, 12, 0x0800);
    b[6] |= 1;
    assert_eq!(Ethernet::parse(&b).unwrap_err(), Error::Address);
    assert_eq!(
        Ethernet::encode(B, A, 0x0800, &alloc::vec![0; 1501]),
        Err(Error::Length)
    );
}
#[test]
fn arp_wire_validation_and_ethernet_padding() {
    let raw = arp_reply().encode().unwrap();
    for n in 0..28 {
        assert_eq!(Arp::parse(&raw[..n]), Err(Error::Length));
    }
    for (offset, value) in [(0, 1), (2, 1), (4, 5), (5, 6), (7, 3)] {
        let mut bad = raw.clone();
        bad[offset] = value;
        assert_eq!(Arp::parse(&bad), Err(Error::Header));
    }
    let b = Ethernet::encode(A, B, 0x0806, &raw).unwrap();
    assert_eq!(
        Arp::parse(Ethernet::parse(&b).unwrap().payload),
        Ok(arp_reply())
    );
    let mut bad = raw;
    bad[8] |= 1;
    assert_eq!(Arp::parse(&bad), Err(Error::Address));
}
#[test]
fn ipv4_rejects_lengths_options_fragments_ttl_protocol_and_addresses() {
    let raw = ip_packet();
    for n in 0..raw.len() {
        assert_eq!(Ipv4::parse(&raw[..n]).unwrap_err(), Error::Length);
    }
    for (offset, value, expected) in [
        (0, 0x65, Error::Header),
        (0, 0x44, Error::Header),
        (0, 0x46, Error::Header),
        (8, 0, Error::Ttl),
        (9, 17, Error::Protocol),
        (12, 0, Error::Address),
        (16, 224, Error::Address),
    ] {
        let mut b = raw.clone();
        b[offset] = value;
        repair_ip(&mut b);
        assert_eq!(Ipv4::parse(&b).unwrap_err(), expected);
    }
    for flags in [0x8000, 0x2000, 1, 0x6000] {
        let mut b = raw.clone();
        put(&mut b, 6, flags);
        repair_ip(&mut b);
        assert_eq!(Ipv4::parse(&b).unwrap_err(), Error::Fragment);
    }
    for length in [0, 19, 1501, 0xffff] {
        let mut b = raw.clone();
        put(&mut b, 2, length);
        repair_ip(&mut b);
        assert_eq!(Ipv4::parse(&b).unwrap_err(), Error::Length);
    }
    let mut bad = raw.clone();
    bad[10] ^= 1;
    assert_eq!(Ipv4::parse(&bad).unwrap_err(), Error::Checksum);
    let mut padded = raw.clone();
    padded.extend_from_slice(&[0xa5; 15]);
    assert_eq!(Ipv4::parse(&padded).unwrap().payload, &raw[20..]);
    assert_eq!(
        Ipv4::encode(IA, IB, &alloc::vec![0; 1481]),
        Err(Error::Length)
    );
}
#[test]
fn icmp_odd_checksum_corruption_and_mtu() {
    assert_eq!(checksum(&[0, 1, 0xf2, 3, 0xf4, 0xf5, 0xf6, 0xf7]), 0x220d);
    for n in [0, 1, 2, 3, 31, 1472] {
        let data: Vec<u8> = (0..n).map(|i| (i * 37) as u8).collect();
        let mut b = Echo::encode(false, 17, 0xffff, &data).unwrap();
        let e = Echo::parse(&b).unwrap();
        assert_eq!(e.data, data);
        assert_eq!(e.sequence, 0xffff);
        b[4] ^= 1;
        assert_eq!(Echo::parse(&b).unwrap_err(), Error::Checksum);
    }
    assert_eq!(
        Echo::encode(false, 1, 1, &alloc::vec![0; 1473]),
        Err(Error::Length)
    );
    for n in 0..8 {
        assert_eq!(Echo::parse(&[0; 8][..n]).unwrap_err(), Error::Length);
    }
    let mut b = Echo::encode(false, 1, 1, &[]).unwrap();
    b[1] = 1;
    assert_eq!(Echo::parse(&b).unwrap_err(), Error::Protocol);
}
#[test]
fn arp_retries_expiry_timeout_and_monotonic_clock() {
    let mut c = ArpCache::new();
    assert_eq!(c.resolve(IB, 0), Ok(Resolution::Request));
    assert_eq!(c.state(IB, 0), Ok(Some(NeighborState::Incomplete)));
    assert_eq!(c.resolve(IB, 999), Ok(Resolution::Wait));
    assert_eq!(c.resolve(IB, 1000), Ok(Resolution::Request));
    c.accept(arp_reply(), B, A, IA, 1001).unwrap();
    assert_eq!(c.resolve(IB, 1002), Ok(Resolution::Ready(B)));
    assert_eq!(c.state(IB, 31001), Ok(Some(NeighborState::Stale)));
    assert_eq!(c.resolve(IB, 31001), Ok(Resolution::Request));
    assert_eq!(c.resolve(IB, 32001), Ok(Resolution::Request));
    assert_eq!(c.resolve(IB, 33001), Ok(Resolution::Request));
    assert_eq!(c.resolve(IB, 34000), Ok(Resolution::Wait));
    assert_eq!(c.resolve(IB, 34001), Err(Error::Timeout));
    assert_eq!(c.state(IB, 34001), Ok(None));
    assert_eq!(c.resolve(IB, 34000), Err(Error::Header));
    assert_eq!(c.resolve(IB, 34001), Ok(Resolution::Request));
}
#[test]
fn arp_capacity_unsolicited_late_and_poisoning_policy() {
    let mut c = ArpCache::new();
    assert_eq!(c.accept(arp_reply(), B, A, IA, 0), Err(Error::Unsolicited));
    c.resolve(IB, 0).unwrap();
    for (mac, local, ip) in [(A, A, IA), (B, B, IA), (B, A, IB)] {
        assert_eq!(
            c.accept(arp_reply(), mac, local, ip, 0),
            Err(Error::Address)
        );
    }
    assert_eq!(
        c.accept(arp_reply(), B, A, IA, 1000),
        Err(Error::Unsolicited)
    );
    c.resolve(IB, 1000).unwrap();
    c.accept(arp_reply(), B, A, IA, 1000).unwrap();
    let mut poison = arp_reply();
    poison.sender_mac = [2, 1, 2, 3, 4, 5];
    assert_eq!(
        c.accept(poison, poison.sender_mac, A, IA, 1000),
        Err(Error::Unsolicited)
    );
    assert_eq!(c.resolve(IB, 1000), Ok(Resolution::Ready(B)));
    for n in 3..10 {
        c.resolve([192, 0, 2, n], 1000).unwrap();
    }
    assert_eq!(c.resolve([192, 0, 2, 10], 1000), Err(Error::Capacity));
    // Reachable expiry allows reclaim of its slot; incomplete slots stay bounded.
    assert_eq!(c.resolve([192, 0, 2, 10], 31000), Ok(Resolution::Request));
}
#[test]
fn logical_peer_rate_limit_address_and_no_reply_amplification() {
    let mut b = Peer::new(B, IB).unwrap();
    let request = Ethernet::encode(B, A, 0x0800, &ip_packet()).unwrap();
    for _ in 0..8 {
        assert!(b.receive(&request, 0).unwrap().is_some());
    }
    assert_eq!(b.receive(&request, 999), Err(Error::RateLimit));
    let reply = b.receive(&request, 1000).unwrap().unwrap();
    let mut a = Peer::new(A, IA).unwrap();
    assert_eq!(a.receive(&reply, 1000), Ok(None));
    let mut wrong = request.clone();
    wrong[..6].copy_from_slice(&BROADCAST);
    assert_eq!(b.receive(&wrong, 1000), Err(Error::Address));
    assert_eq!(b.receive(&request, 999), Err(Error::Header));
}
#[test]
fn malformed_corpus_never_panics_or_allocates_unbounded_output() {
    let mut peer = Peer::new(B, IB).unwrap();
    let mut seed = 0x7193u32;
    for n in 0..10_000usize {
        let size = n % 1600;
        let mut b = alloc::vec![0; size];
        for byte in &mut b {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *byte = seed as u8;
        }
        let _ = Ethernet::parse(&b);
        let _ = Arp::parse(&b);
        let _ = Ipv4::parse(&b);
        let _ = Echo::parse(&b);
        if let Ok(Some(output)) = peer.receive(&b, n as u64) {
            assert!(output.len() <= 1514);
        }
    }
    // Valid outer framing, exhaustive one-byte mutations reach inner parsers.
    let original = Ethernet::encode(B, A, 0x0800, &ip_packet()).unwrap();
    for offset in 0..original.len() {
        for value in 0..=255u8 {
            let mut b = original.clone();
            b[offset] = value;
            let _ = peer.receive(&b, 10_000);
        }
    }
}

#[test]
fn arp_peer_rejects_broadcast_reply_and_mismatched_sender() {
    let mut a = Peer::new(A, IA).unwrap();
    assert!(a.echo(IB, 1, 1, &[], 0).unwrap().is_some());
    let reply = arp_reply().encode().unwrap();
    let broadcast = Ethernet::encode(BROADCAST, B, 0x0806, &reply).unwrap();
    assert_eq!(a.receive(&broadcast, 0), Err(Error::Address));
    let wrong_source = Ethernet::encode(A, A, 0x0806, &reply).unwrap();
    assert_eq!(a.receive(&wrong_source, 0), Err(Error::Address));
    assert_eq!(
        a.neighbors.state(IB, 0),
        Ok(Some(NeighborState::Incomplete))
    );
    let correct = Ethernet::encode(A, B, 0x0806, &reply).unwrap();
    assert_eq!(a.receive(&correct, 0), Ok(None));
    assert_eq!(a.neighbors.resolve(IB, 0), Ok(Resolution::Ready(B)));
}

#[test]
fn checksum_long_input_cannot_overflow() {
    // Public checksum also accepts slices larger than an Ethernet frame.
    assert_eq!(checksum(&alloc::vec![0xff; 200_000]), 0);
    assert_eq!(checksum(&alloc::vec![0xff; 200_001]), 0x00ff);
}
