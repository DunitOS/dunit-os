//! IPv6 framing and address helpers (RFC 4291, RFC 2464, RFC 8200).
use crate::{valid_mac, word, Error, Ip6, Mac, MTU};
use alloc::vec::Vec;

pub const ETHERTYPE_IPV6: u16 = 0x86dd;
pub const HEADER_LEN: usize = 40;
pub const MIN_MTU: usize = 1280;
pub const MAX_PAYLOAD: usize = MTU - HEADER_LEN;

pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;
pub const PROTO_ICMPV6: u8 = 58;

pub const UNSPECIFIED: Ip6 = [0; 16];
pub const LOOPBACK: Ip6 = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
pub const ALL_NODES_MULTICAST: Ip6 = [0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
pub const ALL_ROUTERS_MULTICAST: Ip6 = [0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];

pub fn is_multicast(ip: Ip6) -> bool {
    ip[0] == 0xff
}

pub fn is_link_local(ip: Ip6) -> bool {
    ip[0] == 0xfe && (ip[1] & 0xc0) == 0x80
}

pub fn valid_unicast(ip: Ip6) -> bool {
    const V4_MAPPED_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff];
    ip != UNSPECIFIED && ip != LOOPBACK && !is_multicast(ip) && ip[..12] != V4_MAPPED_PREFIX
}

/// RFC 2464 Ethernet multicast mapping: `33:33` + low 32 bits of IPv6 address.
pub fn multicast_mac(ip: Ip6) -> Mac {
    [0x33, 0x33, ip[12], ip[13], ip[14], ip[15]]
}

/// RFC 4291 solicited-node multicast: `ff02::1:ffXX:XXXX`.
pub fn solicited_node_multicast(target: Ip6) -> Ip6 {
    [
        0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0xff, target[13], target[14], target[15],
    ]
}

/// Modified EUI-64 interface identifier from IEEE 802 MAC.
pub fn eui64_iid(mac: Mac) -> [u8; 8] {
    [
        mac[0] ^ 0x02,
        mac[1],
        mac[2],
        0xff,
        0xfe,
        mac[3],
        mac[4],
        mac[5],
    ]
}

pub fn link_local_from_mac(mac: Mac) -> Result<Ip6, Error> {
    if !valid_mac(mac) {
        return Err(Error::Address);
    }
    let iid = eui64_iid(mac);
    let mut ip = [0; 16];
    ip[0] = 0xfe;
    ip[1] = 0x80;
    ip[8..16].copy_from_slice(&iid);
    Ok(ip)
}

pub fn slaac_from_prefix(prefix: Ip6, prefix_len: u8, mac: Mac) -> Result<Ip6, Error> {
    if prefix_len != 64 || !valid_mac(mac) || is_multicast(prefix) || is_link_local(prefix) {
        return Err(Error::Address);
    }
    let iid = eui64_iid(mac);
    let mut ip = prefix;
    ip[8..16].copy_from_slice(&iid);
    if !valid_unicast(ip) {
        return Err(Error::Address);
    }
    Ok(ip)
}

pub fn mask_prefix(ip: Ip6, prefix_len: u8) -> Option<Ip6> {
    if prefix_len > 128 {
        return None;
    }
    let mut out = [0u8; 16];
    let full = usize::from(prefix_len / 8);
    let rem = prefix_len % 8;
    out[..full].copy_from_slice(&ip[..full]);
    if rem != 0 {
        let mask = 0xffu8 << (8 - rem);
        out[full] = ip[full] & mask;
    }
    Some(out)
}

pub fn prefix_contains(prefix: Ip6, prefix_len: u8, ip: Ip6) -> bool {
    let Some(masked_prefix) = mask_prefix(prefix, prefix_len) else {
        return false;
    };
    let Some(masked_ip) = mask_prefix(ip, prefix_len) else {
        return false;
    };
    masked_prefix == masked_ip
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv6<'a> {
    pub source: Ip6,
    pub destination: Ip6,
    pub next_header: u8,
    pub hop_limit: u8,
    pub payload: &'a [u8],
}

impl<'a> Ipv6<'a> {
    /// Parse a base IPv6 header (no extension headers) for ICMPv6, TCP, or UDP.
    pub fn parse(b: &'a [u8]) -> Result<Self, Error> {
        let packet = Self::parse_framing(b)?;
        if !matches!(packet.next_header, PROTO_ICMPV6 | PROTO_TCP | PROTO_UDP) {
            return Err(Error::Protocol);
        }
        if is_multicast(packet.source) {
            return Err(Error::Address);
        }
        Ok(packet)
    }

    pub fn parse_framing(b: &'a [u8]) -> Result<Self, Error> {
        if b.len() < HEADER_LEN {
            return Err(Error::Length);
        }
        if b[0] >> 4 != 6 {
            return Err(Error::Header);
        }
        let payload_len = usize::from(word(b, 4));
        let total = HEADER_LEN + payload_len;
        if total > b.len() || total > MTU {
            return Err(Error::Length);
        }
        if b[7] == 0 {
            return Err(Error::Ttl);
        }
        Ok(Self {
            source: b[8..24].try_into().unwrap(),
            destination: b[24..40].try_into().unwrap(),
            next_header: b[6],
            hop_limit: b[7],
            payload: &b[HEADER_LEN..total],
        })
    }

    pub fn encode(
        source: Ip6,
        destination: Ip6,
        next_header: u8,
        hop_limit: u8,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if payload.len() > MAX_PAYLOAD {
            return Err(Error::Length);
        }
        if !matches!(next_header, PROTO_ICMPV6 | PROTO_TCP | PROTO_UDP)
            || is_multicast(source)
            || hop_limit == 0
        {
            return Err(Error::Address);
        }
        let mut b = alloc::vec![0; HEADER_LEN + payload.len()];
        b[0] = 0x60;
        let len = (payload.len() as u16).to_be_bytes();
        b[4..6].copy_from_slice(&len);
        b[6] = next_header;
        b[7] = hop_limit;
        b[8..24].copy_from_slice(&source);
        b[24..40].copy_from_slice(&destination);
        b[HEADER_LEN..].copy_from_slice(payload);
        Ok(b)
    }
}

