//! Deterministic malformed-corpus and mutation fuzzing across net-core parsers.
use alloc::{vec, vec::Vec};

use crate::{
    dhcp::Client as DhcpClient,
    dns::{self, dual::DualResolver, Kind, Question},
    ipv6::Ipv6,
    ipv6_peer::Ip6Peer,
    ndp::Icmpv6,
    tcp::{Endpoint, Segment},
    udp::{Address, Udp},
    Arp, Echo, Ethernet, IpAddr, Ipv4, Peer,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push((self.next() & 0xFF) as u8);
        }
        out
    }
}

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut v = seed.to_vec();
    let op = rng.next() % 5;
    match op {
        0 if !v.is_empty() => {
            let idx = (rng.next() as usize) % v.len();
            v[idx] ^= (rng.next() as u8).max(1);
        }
        1 if v.len() > 2 => {
            let cut = 1 + ((rng.next() as usize) % (v.len() - 1));
            v.truncate(cut);
        }
        2 => {
            let extra_len = 1 + ((rng.next() as usize) % 64);
            let extra = rng.bytes(extra_len);
            v.extend_from_slice(&extra);
        }
        3 if v.len() >= 2 => {
            let i = (rng.next() as usize) % v.len();
            v[i] = if rng.next() % 2 == 0 { 0x00 } else { 0xFF };
        }
        _ => {
            let n = (rng.next() as usize) % 300;
            v = rng.bytes(n);
        }
    }
    v
}

#[test]
fn fuzz_all_wire_and_control_parsers_never_panic_or_loop() {
    let mut rng = Rng(0xD00D_F00D_2026_1126);
    let mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    let peer_mac = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0x01];
    let src4 = [192, 0, 2, 1];
    let dst4 = [192, 0, 2, 12];
    let src6 = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
    let dst6 = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2];

    let mut peer4 = Peer::new(mac, dst4, [255, 255, 255, 0], 1500).unwrap();
    let mut peer6 = Ip6Peer::new(mac, 1500, 1, 100).unwrap();
    let mut dhcp = DhcpClient::new(mac, 7).unwrap();
    let mut dual_dns = DualResolver::new();
    let _ = dual_dns.start(b"fuzz.du.net.test", &[IpAddr::V4(src4)], true, true, 100);

    let valid_udp4 = Udp::encode(
        Address { ip: dst4, port: 53 },
        Address { ip: src4, port: 12345 },
        b"hello",
    )
    .unwrap();
    let valid_ip4 = Ipv4::encode_protocol(src4, dst4, 17, &valid_udp4).unwrap();
    let valid_eth4 = Ethernet::encode(mac, peer_mac, 0x0800, &valid_ip4).unwrap();
    let valid_arp = Ethernet::encode(
        [0xff; 6],
        peer_mac,
        0x0806,
        &Arp {
            operation: 1,
            sender_mac: peer_mac,
            sender_ip: src4,
            target_mac: [0; 6],
            target_ip: dst4,
        }
        .encode()
        .unwrap(),
    )
    .unwrap();
    let valid_icmp6 = Icmpv6::encode_ns(src6, dst6, dst6, Some(peer_mac)).unwrap();
    let valid_ip6 = Ipv6::encode(src6, dst6, 58, 255, &valid_icmp6).unwrap();
    let valid_eth6 = Ethernet::encode(mac, peer_mac, 0x86dd, &valid_ip6).unwrap();
    let valid_dns_q = Question::new(1, b"du.net.test", Kind::A).unwrap();

    let dns_loop = vec![
        0x00, 0x01, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
        0xC0, 0x0C, 0x00, 0x01, 0x00, 0x01,
        0xC0, 0x0C, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x10, 0x00, 0x04, 1, 2, 3, 4,
    ];
    let _ = dns::parse_response(&dns_loop, &valid_dns_q);

    let seeds = [
        valid_eth4.as_slice(),
        valid_arp.as_slice(),
        valid_eth6.as_slice(),
        valid_ip4.as_slice(),
        valid_ip6.as_slice(),
        valid_icmp6.as_slice(),
        valid_udp4.as_slice(),
        dns_loop.as_slice(),
    ];

    for iter in 0..4000usize {
        let base = seeds[iter % seeds.len()];
        let sample = if iter < 128 {
            rng.bytes(iter)
        } else {
            mutate(base, &mut rng)
        };

        let _ = Ethernet::parse(&sample);
        let _ = Arp::parse(&sample);
        let _ = Ipv4::parse(&sample);
        let _ = Echo::parse(&sample);
        let _ = Ipv6::parse(&sample);
        let _ = Icmpv6::parse(src6, dst6, (iter % 256) as u8, &sample);
        let _ = Udp::parse(src4, dst4, &sample);
        let _ = Udp::parse_ip(IpAddr::V4(src4), IpAddr::V4(dst4), &sample);
        let _ = Udp::parse_ip(IpAddr::V6(src6), IpAddr::V6(dst6), &sample);
        let _ = Segment::parse(
            Endpoint { ip: src4, port: 443 },
            Endpoint { ip: dst4, port: 50000 },
            &sample,
        );
        let _ = dns::parse_response(&sample, &valid_dns_q);
        let _ = dual_dns.receive_udp(IpAddr::V4(src4), &sample, 200 + iter as u64);
        let _ = dual_dns.receive_tcp(IpAddr::V4(src4), &sample, Kind::A, 200 + iter as u64);
        let _ = dhcp.receive(&sample, 200 + iter as u64);
        let _ = peer4.receive(&sample, 200 + iter as u64);
        let _ = peer6.receive(&sample, 200 + iter as u64);
    }
}
