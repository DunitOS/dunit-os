//! ICMPv6, Neighbor Discovery (RFC 4861), DAD (RFC 4862), and RDNSS (RFC 8106).
use crate::{
    checksum_pseudo,
    ipv6::{self, PROTO_ICMPV6},
    put, valid_mac, word, Error, Ip6, Mac, MTU,
};
use alloc::collections::VecDeque;
use alloc::vec::Vec;

pub const ICMP_ECHO_REQUEST: u8 = 128;
pub const ICMP_ECHO_REPLY: u8 = 129;
pub const ICMP_ROUTER_SOLICITATION: u8 = 133;
pub const ICMP_ROUTER_ADVERTISEMENT: u8 = 134;
pub const ICMP_NEIGHBOR_SOLICITATION: u8 = 135;
pub const ICMP_NEIGHBOR_ADVERTISEMENT: u8 = 136;

const OPT_SOURCE_LL: u8 = 1;
const OPT_TARGET_LL: u8 = 2;
const OPT_PREFIX_INFO: u8 = 3;
const OPT_MTU: u8 = 5;
const OPT_RDNSS: u8 = 25;

pub const NEIGHBOR_CAPACITY: usize = 16;
pub const REACHABLE_MS: u64 = 30_000;
pub const RETRANS_MS: u64 = 1_000;
pub const MAX_MULTICAST_SOLICIT: u8 = 3;
pub const PENDING_QUEUE_CAP: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefixInfo {
    pub prefix: Ip6,
    pub prefix_len: u8,
    pub on_link: bool,
    pub autonomous: bool,
    pub valid_lifetime: u32,
    pub preferred_lifetime: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RdnssInfo {
    pub lifetime: u32,
    pub servers: Vec<Ip6>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Icmpv6<'a> {
    EchoRequest {
        identifier: u16,
        sequence: u16,
        data: &'a [u8],
    },
    EchoReply {
        identifier: u16,
        sequence: u16,
        data: &'a [u8],
    },
    RouterSolicitation {
        source_ll: Option<Mac>,
    },
    RouterAdvertisement {
        cur_hop_limit: u8,
        flags: u8,
        router_lifetime: u16,
        mtu: Option<u32>,
        source_ll: Option<Mac>,
        prefixes: Vec<PrefixInfo>,
        rdnss: Vec<RdnssInfo>,
    },
    NeighborSolicitation {
        target: Ip6,
        source_ll: Option<Mac>,
    },
    NeighborAdvertisement {
        router: bool,
        solicited: bool,
        override_flag: bool,
        target: Ip6,
        target_ll: Option<Mac>,
    },
}

fn icmpv6_checksum(source: Ip6, destination: Ip6, payload: &[u8]) -> u16 {
    checksum_pseudo(&source, &destination, PROTO_ICMPV6, payload)
}

fn parse_options<F>(mut opts: &[u8], mut handler: F) -> Result<(), Error>
where
    F: FnMut(u8, &[u8]) -> Result<(), Error>,
{
    while !opts.is_empty() {
        if opts.len() < 2 {
            return Err(Error::Length);
        }
        let kind = opts[0];
        let units = usize::from(opts[1]);
        if units == 0 {
            return Err(Error::Header);
        }
        let byte_len = units * 8;
        let opt = opts.get(..byte_len).ok_or(Error::Length)?;
        handler(kind, opt)?;
        opts = &opts[byte_len..];
    }
    Ok(())
}

impl<'a> Icmpv6<'a> {
    pub fn parse(source: Ip6, destination: Ip6, hop_limit: u8, b: &'a [u8]) -> Result<Self, Error> {
        if b.len() < 8 || b.len() > MTU - ipv6::HEADER_LEN {
            return Err(Error::Length);
        }
        if icmpv6_checksum(source, destination, b) != 0 {
            return Err(Error::Checksum);
        }
        let kind = b[0];
        let code = b[1];
        if code != 0 {
            return Err(Error::Protocol);
        }
        match kind {
            ICMP_ECHO_REQUEST => Ok(Self::EchoRequest {
                identifier: word(b, 4),
                sequence: word(b, 6),
                data: &b[8..],
            }),
            ICMP_ECHO_REPLY => Ok(Self::EchoReply {
                identifier: word(b, 4),
                sequence: word(b, 6),
                data: &b[8..],
            }),
            ICMP_ROUTER_SOLICITATION => {
                if hop_limit != 255 {
                    return Err(Error::Ttl);
                }
                let mut source_ll = None;
                parse_options(&b[8..], |opt_kind, opt| {
                    if opt_kind == OPT_SOURCE_LL && opt.len() == 8 {
                        let mac: Mac = opt[2..8].try_into().unwrap();
                        if valid_mac(mac) {
                            source_ll = Some(mac);
                        }
                    }
                    Ok(())
                })?;
                Ok(Self::RouterSolicitation { source_ll })
            }
            ICMP_ROUTER_ADVERTISEMENT => {
                if hop_limit != 255 || !ipv6::is_link_local(source) || b.len() < 16 {
                    return Err(Error::Header);
                }
                let cur_hop_limit = b[4];
                let flags = b[5];
                let router_lifetime = word(b, 6);
                let mut mtu = None;
                let mut source_ll = None;
                let mut prefixes = Vec::new();
                let mut rdnss = Vec::new();
                parse_options(&b[16..], |opt_kind, opt| {
                    match opt_kind {
                        OPT_SOURCE_LL if opt.len() == 8 => {
                            let mac: Mac = opt[2..8].try_into().unwrap();
                            if valid_mac(mac) {
                                source_ll = Some(mac);
                            }
                        }
                        OPT_MTU if opt.len() == 8 => {
                            let val = u32::from_be_bytes(opt[4..8].try_into().unwrap());
                            if val >= ipv6::MIN_MTU as u32 {
                                mtu = Some(val);
                            }
                        }
                        OPT_PREFIX_INFO if opt.len() == 32 => {
                            let prefix_len = opt[2];
                            let pflags = opt[3];
                            let valid_lifetime = u32::from_be_bytes(opt[4..8].try_into().unwrap());
                            let preferred_lifetime =
                                u32::from_be_bytes(opt[8..12].try_into().unwrap());
                            let prefix: Ip6 = opt[16..32].try_into().unwrap();
                            if prefix_len <= 128 && preferred_lifetime <= valid_lifetime {
                                prefixes.push(PrefixInfo {
                                    prefix,
                                    prefix_len,
                                    on_link: pflags & 0x80 != 0,
                                    autonomous: pflags & 0x40 != 0,
                                    valid_lifetime,
                                    preferred_lifetime,
                                });
                            }
                        }
                        OPT_RDNSS if opt.len() >= 24 && (opt.len() - 8) % 16 == 0 => {
                            let lifetime = u32::from_be_bytes(opt[4..8].try_into().unwrap());
                            let mut servers = Vec::new();
                            for chunk in opt[8..].chunks_exact(16) {
                                let addr: Ip6 = chunk.try_into().unwrap();
                                if ipv6::valid_unicast(addr) {
                                    servers.push(addr);
                                }
                            }
                            if !servers.is_empty() {
                                rdnss.push(RdnssInfo { lifetime, servers });
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                })?;
                Ok(Self::RouterAdvertisement {
                    cur_hop_limit,
                    flags,
                    router_lifetime,
                    mtu,
                    source_ll,
                    prefixes,
                    rdnss,
                })
            }
            ICMP_NEIGHBOR_SOLICITATION => {
                if hop_limit != 255 || b.len() < 24 {
                    return Err(Error::Header);
                }
                let target: Ip6 = b[8..24].try_into().unwrap();
                if ipv6::is_multicast(target) {
                    return Err(Error::Address);
                }
                let mut source_ll = None;
                parse_options(&b[24..], |opt_kind, opt| {
                    if opt_kind == OPT_SOURCE_LL && opt.len() == 8 {
                        let mac: Mac = opt[2..8].try_into().unwrap();
                        if valid_mac(mac) {
                            source_ll = Some(mac);
                        }
                    }
                    Ok(())
                })?;
                if source == ipv6::UNSPECIFIED && source_ll.is_some() {
                    return Err(Error::Header);
                }
                Ok(Self::NeighborSolicitation { target, source_ll })
            }
            ICMP_NEIGHBOR_ADVERTISEMENT => {
                if hop_limit != 255 || b.len() < 24 {
                    return Err(Error::Header);
                }
                let target: Ip6 = b[8..24].try_into().unwrap();
                if ipv6::is_multicast(target) {
                    return Err(Error::Address);
                }
                let flags = b[4];
                let mut target_ll = None;
                parse_options(&b[24..], |opt_kind, opt| {
                    if opt_kind == OPT_TARGET_LL && opt.len() == 8 {
                        let mac: Mac = opt[2..8].try_into().unwrap();
                        if valid_mac(mac) {
                            target_ll = Some(mac);
                        }
                    }
                    Ok(())
                })?;
                Ok(Self::NeighborAdvertisement {
                    router: flags & 0x80 != 0,
                    solicited: flags & 0x40 != 0,
                    override_flag: flags & 0x20 != 0,
                    target,
                    target_ll,
                })
            }
            _ => Err(Error::Unsupported),
        }
    }

    pub fn encode_echo(
        source: Ip6,
        destination: Ip6,
        reply: bool,
        identifier: u16,
        sequence: u16,
        data: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if data.len() > MTU - ipv6::HEADER_LEN - 8 {
            return Err(Error::Length);
        }
        let mut b = alloc::vec![0; 8 + data.len()];
        b[0] = if reply { ICMP_ECHO_REPLY } else { ICMP_ECHO_REQUEST };
        put(&mut b, 4, identifier);
        put(&mut b, 6, sequence);
        b[8..].copy_from_slice(data);
        let csum = icmpv6_checksum(source, destination, &b);
        put(&mut b, 2, csum);
        Ok(b)
    }

    pub fn encode_rs(source: Ip6, destination: Ip6, source_ll: Option<Mac>) -> Result<Vec<u8>, Error> {
        let len = if source_ll.is_some() { 16 } else { 8 };
        let mut b = alloc::vec![0; len];
        b[0] = ICMP_ROUTER_SOLICITATION;
        if let Some(mac) = source_ll {
            if !valid_mac(mac) {
                return Err(Error::Address);
            }
            b[8] = OPT_SOURCE_LL;
            b[9] = 1;
            b[10..16].copy_from_slice(&mac);
        }
        let csum = icmpv6_checksum(source, destination, &b);
        put(&mut b, 2, csum);
        Ok(b)
    }

    pub fn encode_ns(
        source: Ip6,
        destination: Ip6,
        target: Ip6,
        source_ll: Option<Mac>,
    ) -> Result<Vec<u8>, Error> {
        if ipv6::is_multicast(target) {
            return Err(Error::Address);
        }
        let len = if source_ll.is_some() { 32 } else { 24 };
        let mut b = alloc::vec![0; len];
        b[0] = ICMP_NEIGHBOR_SOLICITATION;
        b[8..24].copy_from_slice(&target);
        if let Some(mac) = source_ll {
            if source == ipv6::UNSPECIFIED || !valid_mac(mac) {
                return Err(Error::Address);
            }
            b[24] = OPT_SOURCE_LL;
            b[25] = 1;
            b[26..32].copy_from_slice(&mac);
        }
        let csum = icmpv6_checksum(source, destination, &b);
        put(&mut b, 2, csum);
        Ok(b)
    }

    pub fn encode_na(
        source: Ip6,
        destination: Ip6,
        solicited: bool,
        override_flag: bool,
        target: Ip6,
        target_ll: Mac,
    ) -> Result<Vec<u8>, Error> {
        if ipv6::is_multicast(target) || !valid_mac(target_ll) {
            return Err(Error::Address);
        }
        let mut b = alloc::vec![0; 32];
        b[0] = ICMP_NEIGHBOR_ADVERTISEMENT;
        b[4] = (u8::from(solicited) * 0x40) | (u8::from(override_flag) * 0x20);
        b[8..24].copy_from_slice(&target);
        b[24] = OPT_TARGET_LL;
        b[25] = 1;
        b[26..32].copy_from_slice(&target_ll);
        let csum = icmpv6_checksum(source, destination, &b);
        put(&mut b, 2, csum);
        Ok(b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DadStatus {
    Tentative,
    Preferred,
    Failed,
}

#[derive(Clone, Copy, Debug)]
pub struct DadProbe {
    pub target: Ip6,
    pub probes_sent: u8,
    pub max_probes: u8,
    pub next_ms: u64,
    pub status: DadStatus,
}

impl DadProbe {
    pub fn new(target: Ip6, now: u64, max_probes: u8) -> Self {
        Self {
            target,
            probes_sent: 0,
            max_probes,
            next_ms: now,
            status: if max_probes == 0 {
                DadStatus::Preferred
            } else {
                DadStatus::Tentative
            },
        }
    }

    pub fn poll(&mut self, now: u64) -> Option<Ip6> {
        if self.status != DadStatus::Tentative || now < self.next_ms {
            return None;
        }
        if self.probes_sent >= self.max_probes {
            self.status = DadStatus::Preferred;
            return None;
        }
        self.probes_sent += 1;
        self.next_ms = now.saturating_add(RETRANS_MS);
        Some(self.target)
    }

    pub fn conflict(&mut self) {
        if self.status == DadStatus::Tentative {
            self.status = DadStatus::Failed;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution6 {
    Ready(Mac),
    Request,
    Wait,
}

#[derive(Clone, Copy)]
enum EntryState {
    Incomplete { attempts: u8, next: u64 },
    Reachable { mac: Mac, expires: u64 },
    Stale { mac: Mac },
}

#[derive(Clone, Copy)]
struct Entry6 {
    ip: Ip6,
    state: EntryState,
}

struct PendingPacket {
    next_hop: Ip6,
    ipv6_packet: Vec<u8>,
}

pub struct NeighborCache6 {
    entries: [Option<Entry6>; NEIGHBOR_CAPACITY],
    pending: VecDeque<PendingPacket>,
    last: u64,
}

impl Default for NeighborCache6 {
    fn default() -> Self {
        Self::new()
    }
}

impl NeighborCache6 {
    pub const fn new() -> Self {
        Self {
            entries: [None; NEIGHBOR_CAPACITY],
            pending: VecDeque::new(),
            last: 0,
        }
    }

    pub fn advance(&mut self, now: u64) -> Result<(), Error> {
        if now < self.last {
            return Err(Error::Header);
        }
        self.last = now;
        for entry in self.entries.iter_mut().flatten() {
            if let EntryState::Reachable { mac, expires } = entry.state {
                if now >= expires {
                    entry.state = EntryState::Stale { mac };
                }
            }
        }
        Ok(())
    }

    pub fn record_known(&mut self, ip: Ip6, mac: Mac, now: u64) -> Vec<Vec<u8>> {
        if !ipv6::valid_unicast(ip) || !valid_mac(mac) {
            return Vec::new();
        }
        let slot = self
            .entries
            .iter()
            .position(|e| e.is_some_and(|n| n.ip == ip))
            .or_else(|| self.entries.iter().position(Option::is_none))
            .or_else(|| {
                self.entries
                    .iter()
                    .position(|e| matches!(e, Some(Entry6 { state: EntryState::Stale { .. }, .. })))
            });
        if let Some(idx) = slot {
            self.entries[idx] = Some(Entry6 {
                ip,
                state: EntryState::Reachable {
                    mac,
                    expires: now.saturating_add(REACHABLE_MS),
                },
            });
        }
        let mut ready = Vec::new();
        let mut remaining = VecDeque::new();
        while let Some(pkt) = self.pending.pop_front() {
            if pkt.next_hop == ip {
                ready.push(pkt.ipv6_packet);
            } else {
                remaining.push_back(pkt);
            }
        }
        self.pending = remaining;
        ready
    }

    pub fn resolve(&mut self, ip: Ip6, now: u64) -> Result<Resolution6, Error> {
        if ipv6::is_multicast(ip) {
            return Ok(Resolution6::Ready(ipv6::multicast_mac(ip)));
        }
        if !ipv6::valid_unicast(ip) {
            return Err(Error::Address);
        }
        self.advance(now)?;
        let index = match self.entries.iter().position(|n| n.is_some_and(|n| n.ip == ip)) {
            Some(i) => i,
            None => self
                .entries
                .iter()
                .position(|n| {
                    n.is_none()
                        || matches!(
                            n,
                            Some(Entry6 {
                                state: EntryState::Stale { .. },
                                ..
                            })
                        )
                })
                .ok_or(Error::Capacity)?,
        };
        let existing = self.entries[index].filter(|n| n.ip == ip).map(|n| n.state);
        match existing {
            Some(EntryState::Reachable { mac, .. }) => Ok(Resolution6::Ready(mac)),
            Some(EntryState::Stale { mac }) => Ok(Resolution6::Ready(mac)),
            Some(EntryState::Incomplete { next, .. }) if now < next => Ok(Resolution6::Wait),
            Some(EntryState::Incomplete { attempts, .. }) if attempts >= MAX_MULTICAST_SOLICIT => {
                self.entries[index] = None;
                self.pending.retain(|p| p.next_hop != ip);
                Err(Error::Timeout)
            }
            _ => {
                let attempts = match existing {
                    Some(EntryState::Incomplete { attempts, .. }) => attempts + 1,
                    _ => 1,
                };
                self.entries[index] = Some(Entry6 {
                    ip,
                    state: EntryState::Incomplete {
                        attempts,
                        next: now.saturating_add(RETRANS_MS),
                    },
                });
                Ok(Resolution6::Request)
            }
        }
    }

    pub fn queue_packet(&mut self, next_hop: Ip6, ipv6_packet: Vec<u8>) {
        if self.pending.len() >= PENDING_QUEUE_CAP {
            self.pending.pop_front();
        }
        self.pending.push_back(PendingPacket {
            next_hop,
            ipv6_packet,
        });
    }
}

