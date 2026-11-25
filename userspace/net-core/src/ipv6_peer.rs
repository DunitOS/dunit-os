//! Per-interface IPv6/NDP/DAD/SLAAC/RA/RDNSS state machine.
use crate::{
    ipv6::{self, Ipv6, ETHERTYPE_IPV6, PROTO_ICMPV6, PROTO_TCP, PROTO_UDP},
    ndp::{DadProbe, DadStatus, Icmpv6, NeighborCache6, PrefixInfo, Resolution6},
    router::{InterfaceView, Route6},
    udp::Udp,
    valid_mac, Error, Ethernet, Ip6, IpAddr, Mac, MTU,
};
use alloc::vec::Vec;

const RS_MAX_ATTEMPTS: u8 = 3;
const RS_INTERVAL_MS: u64 = 1_000;
const ECHO_RATE_LIMIT: u8 = 8;

#[derive(Clone, Debug)]
struct GlobalSlot {
    addr: Ip6,
    prefix_len: u8,
    dad: DadProbe,
    preferred: bool,
}

#[derive(Clone, Copy, Debug)]
struct PrefixSlot {
    prefix: Ip6,
    prefix_len: u8,
}

#[derive(Clone, Copy, Debug)]
struct RouterSlot {
    addr: Ip6,
}

#[derive(Debug)]
pub enum Egress6 {
    Sent(Vec<u8>),
    Neighbor(Vec<u8>),
    Queued,
}

#[derive(Debug)]
pub enum RxEvent6<'a> {
    None,
    Reply(Vec<u8>),
    Flush(Vec<Vec<u8>>),
    Udp {
        source: Ip6,
        destination: Ip6,
        source_port: u16,
        destination_port: u16,
        payload: &'a [u8],
    },
    Tcp {
        source: Ip6,
        destination: Ip6,
        payload: &'a [u8],
    },
}

pub struct Ip6Peer {
    mac: Mac,
    mtu: usize,
    dad_probes: u8,
    slaac_enabled: bool,
    link_local: DadProbe,
    globals: Vec<GlobalSlot>,
    prefixes: Vec<PrefixSlot>,
    routers: Vec<RouterSlot>,
    rdnss: Vec<Ip6>,
    static_dns: Vec<Ip6>,
    neighbors: NeighborCache6,
    rs_sent: u8,
    rs_next_ms: u64,
    ra_seen: bool,
    window: u64,
    replies: u8,
}

impl Ip6Peer {
    pub fn new(mac: Mac, mtu: usize, dad_probes: u8, now: u64) -> Result<Self, Error> {
        if !valid_mac(mac) {
            return Err(Error::Address);
        }
        if !(ipv6::MIN_MTU..=MTU).contains(&mtu) {
            return Err(Error::Length);
        }
        let ll = ipv6::link_local_from_mac(mac)?;
        Ok(Self {
            mac,
            mtu,
            dad_probes,
            slaac_enabled: true,
            link_local: DadProbe::new(ll, now, dad_probes),
            globals: Vec::new(),
            prefixes: Vec::new(),
            routers: Vec::new(),
            rdnss: Vec::new(),
            static_dns: Vec::new(),
            neighbors: NeighborCache6::new(),
            rs_sent: 0,
            rs_next_ms: now,
            ra_seen: false,
            window: 0,
            replies: 0,
        })
    }

    pub fn set_slaac(&mut self, enabled: bool) {
        self.slaac_enabled = enabled;
    }

    pub fn reset(&mut self, now: u64) {
        let ll = ipv6::link_local_from_mac(self.mac).expect("valid mac");
        self.link_local = DadProbe::new(ll, now, self.dad_probes);
        self.globals.clear();
        self.prefixes.clear();
        self.routers.clear();
        self.rdnss.clear();
        self.neighbors = NeighborCache6::new();
        self.rs_sent = 0;
        self.rs_next_ms = now;
        self.ra_seen = false;
    }

    pub fn configure_static(
        &mut self,
        addr: Ip6,
        prefix_len: u8,
        gateway: Option<Ip6>,
        dns: &[Ip6],
        now: u64,
    ) -> Result<(), Error> {
        if !ipv6::valid_unicast(addr) || prefix_len > 128 {
            return Err(Error::Address);
        }
        if !self.globals.iter().any(|g| g.addr == addr) {
            self.globals.push(GlobalSlot {
                addr,
                prefix_len,
                dad: DadProbe::new(addr, now, self.dad_probes),
                preferred: self.dad_probes == 0,
            });
        }
        if !self.prefixes.iter().any(|p| p.prefix == addr && p.prefix_len == prefix_len) {
            self.prefixes.push(PrefixSlot {
                prefix: ipv6::mask_prefix(addr, prefix_len).unwrap_or(addr),
                prefix_len,
            });
        }
        if let Some(gw) = gateway {
            if !self.routers.iter().any(|r| r.addr == gw) {
                self.routers.push(RouterSlot { addr: gw });
            }
        }
        for &d in dns {
            if ipv6::valid_unicast(d) && !self.static_dns.contains(&d) {
                self.static_dns.push(d);
            }
        }
        Ok(())
    }

    pub fn link_local_ip(&self) -> Option<Ip6> {
        (self.link_local.status == DadStatus::Preferred).then_some(self.link_local.target)
    }

    #[cfg(test)]
    pub(crate) fn link_local_from_probe_for_test(&self) -> Ip6 {
        self.link_local.target
    }

    pub fn link_local_failed(&self) -> bool {
        self.link_local.status == DadStatus::Failed
    }

    pub fn preferred_globals(&self) -> Vec<(Ip6, u8)> {
        self.globals
            .iter()
            .filter(|g| g.dad.status == DadStatus::Preferred && g.preferred)
            .map(|g| (g.addr, g.prefix_len))
            .collect()
    }

    pub fn owns_preferred(&self, ip: Ip6) -> bool {
        self.link_local_ip() == Some(ip)
            || self
                .globals
                .iter()
                .any(|g| g.dad.status == DadStatus::Preferred && g.addr == ip)
    }

    fn owns_tentative_conflict(&mut self, target: Ip6) -> bool {
        let mut hit = false;
        if self.link_local.target == target && self.link_local.status == DadStatus::Tentative {
            self.link_local.conflict();
            hit = true;
        }
        for g in &mut self.globals {
            if g.addr == target && g.dad.status == DadStatus::Tentative {
                g.dad.conflict();
                hit = true;
            }
        }
        hit
    }

    pub fn fill_interface_view(&self, view: &mut InterfaceView) {
        view.ipv6_ll = self.link_local_ip();
        view.ipv6_globals = self.preferred_globals();
        view.ipv6_routes.clear();
        for p in &self.prefixes {
            view.ipv6_routes.push(Route6 {
                prefix: p.prefix,
                prefix_len: p.prefix_len,
                gateway: None,
            });
        }
        for r in &self.routers {
            view.ipv6_routes.push(Route6 {
                prefix: ipv6::UNSPECIFIED,
                prefix_len: 0,
                gateway: Some(r.addr),
            });
        }
        view.ipv6_dns.clear();
        for &d in self.rdnss.iter().chain(self.static_dns.iter()) {
            if !view.ipv6_dns.contains(&d) {
                view.ipv6_dns.push(d);
            }
        }
    }

    pub fn deadline(&self) -> Option<u64> {
        let mut min = None;
        if self.link_local.status == DadStatus::Tentative {
            min = Some(self.link_local.next_ms);
        }
        for g in &self.globals {
            if g.dad.status == DadStatus::Tentative {
                min = Some(min.map_or(g.dad.next_ms, |m: u64| m.min(g.dad.next_ms)));
            }
        }
        if self.slaac_enabled
            && !self.ra_seen
            && self.rs_sent < RS_MAX_ATTEMPTS
            && self.link_local.status == DadStatus::Preferred
        {
            min = Some(min.map_or(self.rs_next_ms, |m| m.min(self.rs_next_ms)));
        }
        min
    }

    pub fn poll(&mut self, now: u64) -> Result<Vec<Vec<u8>>, Error> {
        self.neighbors.advance(now)?;
        let mut out = Vec::new();
        if let Some(target) = self.link_local.poll(now) {
            out.push(self.encode_dad_ns(target)?);
        }
        for g in &mut self.globals {
            let was_tentative = g.dad.status == DadStatus::Tentative;
            if let Some(target) = g.dad.poll(now) {
                let frame = Ethernet::encode(
                    ipv6::multicast_mac(ipv6::solicited_node_multicast(target)),
                    self.mac,
                    ETHERTYPE_IPV6,
                    &Ipv6::encode(
                        ipv6::UNSPECIFIED,
                        ipv6::solicited_node_multicast(target),
                        PROTO_ICMPV6,
                        255,
                        &Icmpv6::encode_ns(
                            ipv6::UNSPECIFIED,
                            ipv6::solicited_node_multicast(target),
                            target,
                            None,
                        )?,
                    )?,
                )?;
                out.push(frame);
            } else if was_tentative && g.dad.status == DadStatus::Preferred {
                g.preferred = true;
            }
        }
        if self.slaac_enabled
            && !self.ra_seen
            && self.link_local.status == DadStatus::Preferred
            && self.rs_sent < RS_MAX_ATTEMPTS
            && now >= self.rs_next_ms
        {
            self.rs_sent += 1;
            self.rs_next_ms = now.saturating_add(RS_INTERVAL_MS);
            let rs = Icmpv6::encode_rs(
                self.link_local.target,
                ipv6::ALL_ROUTERS_MULTICAST,
                Some(self.mac),
            )?;
            let pkt = Ipv6::encode(
                self.link_local.target,
                ipv6::ALL_ROUTERS_MULTICAST,
                PROTO_ICMPV6,
                255,
                &rs,
            )?;
            out.push(Ethernet::encode(
                ipv6::multicast_mac(ipv6::ALL_ROUTERS_MULTICAST),
                self.mac,
                ETHERTYPE_IPV6,
                &pkt,
            )?);
        }
        Ok(out)
    }

    fn encode_dad_ns(&self, target: Ip6) -> Result<Vec<u8>, Error> {
        let mcast = ipv6::solicited_node_multicast(target);
        let ns = Icmpv6::encode_ns(ipv6::UNSPECIFIED, mcast, target, None)?;
        let pkt = Ipv6::encode(ipv6::UNSPECIFIED, mcast, PROTO_ICMPV6, 255, &ns)?;
        Ethernet::encode(ipv6::multicast_mac(mcast), self.mac, ETHERTYPE_IPV6, &pkt)
    }

    fn encode_ns_for(&self, src: Ip6, target: Ip6) -> Result<Vec<u8>, Error> {
        let mcast = ipv6::solicited_node_multicast(target);
        let ns = Icmpv6::encode_ns(src, mcast, target, Some(self.mac))?;
        let pkt = Ipv6::encode(src, mcast, PROTO_ICMPV6, 255, &ns)?;
        Ethernet::encode(ipv6::multicast_mac(mcast), self.mac, ETHERTYPE_IPV6, &pkt)
    }

    pub fn send_packet(
        &mut self,
        src: Ip6,
        dst: Ip6,
        next_hop: Ip6,
        proto: u8,
        payload: &[u8],
        now: u64,
    ) -> Result<Egress6, Error> {
        let pkt = Ipv6::encode(src, dst, proto, 64, payload)?;
        if pkt.len() > self.mtu {
            return Err(Error::Length);
        }
        match self.neighbors.resolve(next_hop, now)? {
            Resolution6::Ready(mac) => {
                Ethernet::encode(mac, self.mac, ETHERTYPE_IPV6, &pkt).map(Egress6::Sent)
            }
            Resolution6::Request => {
                self.neighbors.queue_packet(next_hop, pkt);
                let ns_src = if ipv6::is_link_local(next_hop) {
                    self.link_local_ip().unwrap_or(src)
                } else {
                    src
                };
                self.encode_ns_for(ns_src, next_hop).map(Egress6::Neighbor)
            }
            Resolution6::Wait => {
                self.neighbors.queue_packet(next_hop, pkt);
                Ok(Egress6::Queued)
            }
        }
    }

    fn accept_frame_destination(&self, dst_mac: Mac) -> bool {
        dst_mac == self.mac || (dst_mac[0] == 0x33 && dst_mac[1] == 0x33)
    }

    fn apply_ra(
        &mut self,
        router_src: Ip6,
        router_lifetime: u16,
        mtu: Option<u32>,
        source_ll: Option<Mac>,
        prefixes: &[PrefixInfo],
        rdnss: &[crate::ndp::RdnssInfo],
        now: u64,
    ) -> Vec<Vec<u8>> {
        self.ra_seen = true;
        if let Some(m) = mtu {
            let clamped = (m as usize).clamp(ipv6::MIN_MTU, MTU);
            self.mtu = self.mtu.min(clamped);
        }
        let mut flushed = Vec::new();
        if let Some(mac) = source_ll {
            for pkt in self.neighbors.record_known(router_src, mac, now) {
                if let Ok(frame) = Ethernet::encode(mac, self.mac, ETHERTYPE_IPV6, &pkt) {
                    flushed.push(frame);
                }
            }
        }
        if router_lifetime > 0 && !self.routers.iter().any(|r| r.addr == router_src) {
            self.routers.push(RouterSlot { addr: router_src });
        }
        for p in prefixes {
            if p.valid_lifetime == 0 {
                continue;
            }
            if p.on_link {
                if let Some(masked) = ipv6::mask_prefix(p.prefix, p.prefix_len) {
                    if !self
                        .prefixes
                        .iter()
                        .any(|s| s.prefix == masked && s.prefix_len == p.prefix_len)
                    {
                        self.prefixes.push(PrefixSlot {
                            prefix: masked,
                            prefix_len: p.prefix_len,
                        });
                    }
                }
            }
            if p.autonomous && self.slaac_enabled && p.prefix_len == 64 {
                if let Ok(addr) = ipv6::slaac_from_prefix(p.prefix, p.prefix_len, self.mac) {
                    if !self.globals.iter().any(|g| g.addr == addr) {
                        self.globals.push(GlobalSlot {
                            addr,
                            prefix_len: 64,
                            dad: DadProbe::new(addr, now, self.dad_probes),
                            preferred: self.dad_probes == 0,
                        });
                    }
                }
            }
        }
        for entry in rdnss {
            if entry.lifetime == 0 {
                continue;
            }
            for &srv in &entry.servers {
                if !self.rdnss.contains(&srv) {
                    self.rdnss.push(srv);
                }
            }
        }
        flushed
    }

    pub fn receive<'a>(&mut self, bytes: &'a [u8], now: u64) -> Result<RxEvent6<'a>, Error> {
        self.neighbors.advance(now)?;
        let frame = Ethernet::parse(bytes)?;
        if frame.kind != ETHERTYPE_IPV6 || !self.accept_frame_destination(frame.destination) {
            return Err(Error::EtherType);
        }
        let pkt = Ipv6::parse(frame.payload)?;
        if pkt.payload.len() + ipv6::HEADER_LEN > self.mtu {
            return Err(Error::Length);
        }
        match pkt.next_header {
            PROTO_ICMPV6 => {
                let msg = Icmpv6::parse(pkt.source, pkt.destination, pkt.hop_limit, pkt.payload)?;
                match msg {
                    Icmpv6::RouterAdvertisement {
                        router_lifetime,
                        mtu,
                        source_ll,
                        prefixes,
                        rdnss,
                        ..
                    } => {
                        let flushed = self.apply_ra(
                            pkt.source,
                            router_lifetime,
                            mtu,
                            source_ll,
                            &prefixes,
                            &rdnss,
                            now,
                        );
                        if flushed.is_empty() {
                            Ok(RxEvent6::None)
                        } else {
                            Ok(RxEvent6::Flush(flushed))
                        }
                    }
                    Icmpv6::NeighborSolicitation { target, source_ll } => {
                        if let Some(mac) = source_ll {
                            if ipv6::valid_unicast(pkt.source) {
                                let _ = self.neighbors.record_known(pkt.source, mac, now);
                            }
                        }
                        if pkt.source == ipv6::UNSPECIFIED && self.owns_tentative_conflict(target) {
                            return Ok(RxEvent6::None);
                        }
                        if !self.owns_preferred(target) {
                            return Ok(RxEvent6::None);
                        }
                        let (na_dst, dst_mac, solicited) = if pkt.source == ipv6::UNSPECIFIED {
                            (
                                ipv6::ALL_NODES_MULTICAST,
                                ipv6::multicast_mac(ipv6::ALL_NODES_MULTICAST),
                                false,
                            )
                        } else {
                            (pkt.source, frame.source, true)
                        };
                        let na = Icmpv6::encode_na(
                            target, na_dst, solicited, true, target, self.mac,
                        )?;
                        let reply = Ipv6::encode(target, na_dst, PROTO_ICMPV6, 255, &na)?;
                        Ok(RxEvent6::Reply(Ethernet::encode(
                            dst_mac,
                            self.mac,
                            ETHERTYPE_IPV6,
                            &reply,
                        )?))
                    }
                    Icmpv6::NeighborAdvertisement {
                        target, target_ll, ..
                    } => {
                        if self.owns_tentative_conflict(target) {
                            return Ok(RxEvent6::None);
                        }
                        let mac = target_ll.unwrap_or(frame.source);
                        let pkts = self.neighbors.record_known(target, mac, now);
                        if pkts.is_empty() {
                            Ok(RxEvent6::None)
                        } else {
                            let mut frames = Vec::with_capacity(pkts.len());
                            for p in pkts {
                                frames.push(Ethernet::encode(mac, self.mac, ETHERTYPE_IPV6, &p)?);
                            }
                            Ok(RxEvent6::Flush(frames))
                        }
                    }
                    Icmpv6::EchoRequest {
                        identifier,
                        sequence,
                        data,
                    } => {
                        if !self.owns_preferred(pkt.destination) {
                            return Ok(RxEvent6::None);
                        }
                        if now.saturating_sub(self.window) >= 1000 {
                            self.window = now;
                            self.replies = 0;
                        }
                        if self.replies >= ECHO_RATE_LIMIT {
                            return Err(Error::RateLimit);
                        }
                        self.replies += 1;
                        let echo = Icmpv6::encode_echo(
                            pkt.destination,
                            pkt.source,
                            true,
                            identifier,
                            sequence,
                            data,
                        )?;
                        let reply = Ipv6::encode(
                            pkt.destination,
                            pkt.source,
                            PROTO_ICMPV6,
                            64,
                            &echo,
                        )?;
                        Ok(RxEvent6::Reply(Ethernet::encode(
                            frame.source,
                            self.mac,
                            ETHERTYPE_IPV6,
                            &reply,
                        )?))
                    }
                    Icmpv6::EchoReply { .. } | Icmpv6::RouterSolicitation { .. } => Ok(RxEvent6::None),
                }
            }
            PROTO_UDP => {
                if !self.owns_preferred(pkt.destination) {
                    return Err(Error::Address);
                }
                let datagram =
                    Udp::parse_ip(IpAddr::V6(pkt.source), IpAddr::V6(pkt.destination), pkt.payload)?;
                Ok(RxEvent6::Udp {
                    source: pkt.source,
                    destination: pkt.destination,
                    source_port: datagram.source,
                    destination_port: datagram.destination,
                    payload: datagram.data,
                })
            }
            PROTO_TCP => {
                if !self.owns_preferred(pkt.destination) {
                    return Err(Error::Address);
                }
                Ok(RxEvent6::Tcp {
                    source: pkt.source,
                    destination: pkt.destination,
                    payload: pkt.payload,
                })
            }
            _ => Err(Error::Protocol),
        }
    }
}
