use super::*;

const REMOTE: Endpoint = Endpoint { ip: [192, 0, 2, 1], port: 5432 };
const LOCAL: Endpoint = Endpoint { ip: [192, 0, 2, 2], port: 49152 };
const SEND: BridgeHeader = BridgeHeader {
    token: 0x0102_0304_0506_0708,
    owner: 0x1112_1314_1516_1718,
    kind: BRIDGE_SEND,
    flags: 0,
    error: 0,
    endpoint: REMOTE,
    protocol: PROTO_UDP,
    length: 0,
};

#[test]
fn options_and_endpoints_preserve_exact_native_bytes() {
    assert_eq!(Options.encode(), [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(Options::decode(&Options.encode()), Ok(Options));
    assert_eq!(Options::decode(&[1, 0, 0, 0, 0, 0, 0, 1]), Err(WireError::Invalid));
    assert_eq!(Options::decode(&[2, 0, 0, 0, 0, 0, 0, 0]), Err(WireError::Invalid));
    assert_eq!(REMOTE.encode(), [192, 0, 2, 1, 0x38, 0x15, 0, 0]);
    assert!(REMOTE.is_unicast());
    assert!(Endpoint::NONE.is_none());
    for ip in [[0, 0, 0, 0], [127, 0, 0, 1], [224, 0, 0, 1], [255, 255, 255, 255]] {
        assert!(!Endpoint { ip, port: 1 }.is_unicast());
    }
    let mut malformed = REMOTE.encode();
    malformed[7] = 1;
    assert_eq!(Endpoint::decode(&malformed), Err(WireError::Invalid));
    let endpoints = Endpoints { local: LOCAL, remote: REMOTE };
    assert_eq!(Endpoints::decode(&endpoints.encode()), Ok(endpoints));
}

#[test]
fn datagram_metadata_preserves_full_length_and_rejects_reserved_values() {
    let header = RecvHeader { endpoint: REMOTE, full_len: 1300 };
    let mut bytes = header.encode();
    assert_eq!(&bytes[..8], &REMOTE.encode());
    assert_eq!(&bytes[8..], &[0x14, 0x05, 0, 0, 0, 0, 0, 0]);
    assert_eq!(RecvHeader::decode(&bytes), Ok(header));
    bytes[15] = 1;
    assert_eq!(RecvHeader::decode(&bytes), Err(WireError::Invalid));
    let too_long = RecvHeader { full_len: MAX_PAYLOAD as u32 + 1, ..header };
    assert_eq!(RecvHeader::decode(&too_long.encode()), Err(WireError::Invalid));
}

#[test]
fn bridge_offsets_exact_slice_and_bounded_payload() {
    let mut bytes = [0xcc; MAX_FRAME + 1];
    let payload = [0x6c; MAX_PAYLOAD];
    let len = SEND.encode(&payload, &mut bytes).unwrap();
    assert_eq!(len, MAX_FRAME);
    assert_eq!(&bytes[..16], &[8, 7, 6, 5, 4, 3, 2, 1, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11]);
    assert_eq!(&bytes[16..32], &[3, 0, 0, 0, 0, 0, 0, 0, 192, 0, 2, 1, 0x38, 0x15, 1, 0]);
    assert_eq!(&bytes[32..40], &[0, 16, 0, 0, 0, 0, 0, 0]);
    assert_eq!(bytes[MAX_FRAME], 0xcc);
    let (decoded, data) = BridgeHeader::decode(&bytes[..len]).unwrap();
    assert_eq!(decoded, BridgeHeader { length: MAX_PAYLOAD as u32, ..SEND });
    assert_eq!(data, payload);
    for split in 0..len {
        assert_eq!(BridgeHeader::decode(&bytes[..split]), Err(WireError::Invalid));
    }
    assert_eq!(BridgeHeader::decode(&bytes), Err(WireError::Invalid));
}

#[test]
fn bridge_record_kind_flags_authority_and_padding_are_checked() {
    let mut bytes = [0; BRIDGE_HEADER_LEN];
    let credit = BridgeHeader { endpoint: Endpoint::NONE, kind: BRIDGE_CREDIT, ..SEND };
    assert_eq!(credit.encode(&[], &mut bytes), Ok(BRIDGE_HEADER_LEN));
    assert_eq!(BridgeHeader::decode(&bytes), Ok((credit, &[][..])));
    let done = BridgeHeader { kind: BRIDGE_COMPLETE, flags: BRIDGE_CONNECT, error: -5, ..SEND };
    assert_eq!(done.encode(&[], &mut bytes), Ok(BRIDGE_HEADER_LEN));
    assert_eq!(BridgeHeader::decode(&bytes), Ok((done, &[][..])));
    let original = bytes;
    for invalid in [
        BridgeHeader { token: 0, ..SEND },
        BridgeHeader { owner: 0, ..SEND },
        BridgeHeader { kind: 15, ..SEND },
        BridgeHeader { protocol: 0, ..SEND },
        BridgeHeader { flags: BRIDGE_SEND, ..SEND },
        BridgeHeader { flags: 0, ..done },
        BridgeHeader { error: 5, ..done },
    ] {
        assert_eq!(invalid.encode(&[], &mut bytes), Err(WireError::Invalid));
        assert_eq!(bytes, original);
    }
    assert_eq!(SEND.encode(b"a", &mut bytes), Err(WireError::BufferTooSmall));
    assert_eq!(SEND.encode(&[0; MAX_PAYLOAD + 1], &mut bytes), Err(WireError::Invalid));
    assert_eq!(bytes, original);
    let dns = BridgeHeader { protocol: PROTO_DNS, endpoint: Endpoint::NONE, ..SEND };
    let mut dns_bytes = [0; BRIDGE_HEADER_LEN + 11];
    assert_eq!(dns.encode(b"du.net.test", &mut dns_bytes), Ok(BRIDGE_HEADER_LEN + 11));
    assert_eq!(
        BridgeHeader::decode(&dns_bytes).unwrap().0,
        BridgeHeader { length: 11, ..dns }
    );
    bytes[39] = 1;
    assert_eq!(BridgeHeader::decode(&bytes), Err(WireError::Invalid));
    let bind = BridgeHeader {
        kind: BRIDGE_BIND,
        endpoint: Endpoint { ip: [0; 4], port: 8080 },
        protocol: PROTO_TCP,
        ..SEND
    };
    assert!(bind.endpoint.is_bind_address());
    assert!(!bind.endpoint.is_unicast());
    let mut hdr = [0; BRIDGE_HEADER_LEN];
    assert_eq!(bind.encode(&[], &mut hdr), Ok(BRIDGE_HEADER_LEN));
    assert_eq!(BridgeHeader::decode(&hdr), Ok((bind, &[][..])));
    let ready = BridgeHeader {
        kind: BRIDGE_LISTENER_READY,
        flags: 1,
        endpoint: Endpoint::NONE,
        protocol: PROTO_TCP,
        ..SEND
    };
    assert_eq!(ready.encode(&[], &mut hdr), Ok(BRIDGE_HEADER_LEN));
    assert_eq!(BridgeHeader::decode(&hdr), Ok((ready, &[][..])));
    let bad_ready = BridgeHeader { flags: 2, ..ready };
    assert_eq!(bad_ready.encode(&[], &mut hdr), Err(WireError::Invalid));
}

#[test]
fn v2_endpoints_bridge_and_dns_records_roundtrip() {
    let v2_opts = Options::encode_version(VERSION_V2);
    assert_eq!(Options::decode_version(&v2_opts), Ok(VERSION_V2));
    assert_eq!(Options::decode_version(&Options.encode()), Ok(VERSION));

    let ip6 = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let ep6 = Endpoint6::ipv6(ip6, 443, 0);
    assert!(ep6.is_unicast());
    assert_eq!(Endpoint6::decode(&ep6.encode()), Ok(ep6));

    let ll6 = Endpoint6::ipv6([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2], 5353, 1);
    assert!(ll6.is_unicast());
    assert_eq!(Endpoint6::decode(&ll6.encode()), Ok(ll6));

    let ep4 = Endpoint6::from_v4(REMOTE);
    assert!(ep4.is_unicast());
    assert_eq!(ep4.to_v4(), Some(REMOTE));
    assert_eq!(Endpoint6::decode(&ep4.encode()), Ok(ep4));

    assert!(!Endpoint6::ipv6([0; 16], 443, 0).is_unicast());
    assert!(Endpoint6::ipv6([0; 16], 443, 0).is_bind_address());
    assert!(!Endpoint6::ipv6([0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], 443, 0).is_unicast());

    let hdr6 = BridgeHeader6 {
        token: 7,
        owner: 11,
        kind: BRIDGE_SEND,
        flags: 0,
        error: 0,
        endpoint: ep6,
        protocol: PROTO_UDP,
        length: 0,
    };
    let mut frame = [0; MAX_FRAME_V2];
    let n = hdr6.encode(b"hello6", &mut frame).unwrap();
    assert_eq!(n, BRIDGE_HEADER6_LEN + 6);
    let (decoded, data) = BridgeHeader6::decode(&frame[..n]).unwrap();
    assert_eq!(decoded, BridgeHeader6 { length: 6, ..hdr6 });
    assert_eq!(data, b"hello6");

    let mut dns_buf = [0; 64];
    let recs = [DnsRecord::ipv6(ip6, 0), DnsRecord::ipv4([192, 0, 2, 10])];
    let dns_len = encode_dns_records(&recs, &mut dns_buf).unwrap();
    let mut out = [DnsRecord::default(); 4];
    assert_eq!(decode_dns_records(&dns_buf[..dns_len], &mut out), Ok(2));
    assert_eq!(out[..2], recs);

    let ping_hdr = BridgeHeader6 {
        protocol: PROTO_ICMP,
        endpoint: Endpoint6::ipv6(ip6, 1, 0),
        ..hdr6
    };
    assert!(ping_hdr.encode(b"ping", &mut frame).is_ok());
    let status_hdr = BridgeHeader6 {
        protocol: PROTO_STATUS,
        endpoint: Endpoint6::NONE,
        ..hdr6
    };
    assert!(status_hdr.encode(b"status", &mut frame).is_ok());
}
