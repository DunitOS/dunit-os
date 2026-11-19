//! Bounded userspace packet codecs and a logical Ethernet peer. No device access.
#![no_std]
extern crate alloc;
use alloc::vec::Vec;

pub type Mac = [u8; 6];
pub type Ip = [u8; 4];
pub const BROADCAST: Mac = [255; 6];
pub const MTU: usize = 1500;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Length,
    EtherType,
    Address,
    Header,
    Checksum,
    Ttl,
    Protocol,
    Fragment,
    Capacity,
    Timeout,
    Unsolicited,
    RateLimit,
    NoRoute,
    WouldBlock,
    NotBound,
    PortInUse,
    Unsupported,
    NameNotFound,
}
fn word(b: &[u8], n: usize) -> u16 {
    u16::from_be_bytes([b[n], b[n + 1]])
}
fn put(b: &mut [u8], n: usize, v: u16) {
    b[n..n + 2].copy_from_slice(&v.to_be_bytes());
}
pub fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    for pair in bytes.chunks(2) {
        sum += (pair[0] as u32) << 8 | pair.get(1).copied().unwrap_or(0) as u32;
        sum = (sum & 0xffff) + (sum >> 16);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
fn valid_mac(m: Mac) -> bool {
    m != [0; 6] && m[0] & 1 == 0
}
fn valid_ip(i: Ip) -> bool {
    i[0] != 0 && i[0] < 224 && i != [255; 4]
}
#[derive(Debug)]
pub struct Ethernet<'a> {
    pub destination: Mac,
    pub source: Mac,
    pub kind: u16,
    pub payload: &'a [u8],
}
impl<'a> Ethernet<'a> {
    /// Frames exclude preamble and FCS; accept runt-free 60..1514-byte frames.
    pub fn parse(b: &'a [u8]) -> Result<Self, Error> {
        if !(60..=1514).contains(&b.len()) {
            return Err(Error::Length);
        }
        let destination = b[..6].try_into().unwrap();
        let source = b[6..12].try_into().unwrap();
        if !valid_mac(source) {
            return Err(Error::Address);
        }
        let kind = word(b, 12);
        if kind != 0x0800 && kind != 0x0806 {
            return Err(Error::EtherType);
        }
        Ok(Self {
            destination,
            source,
            kind,
            payload: &b[14..],
        })
    }
    pub fn encode(
        destination: Mac,
        source: Mac,
        kind: u16,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if payload.len() > MTU {
            return Err(Error::Length);
        }
        if !valid_mac(source) {
            return Err(Error::Address);
        }
        if kind != 0x0800 && kind != 0x0806 {
            return Err(Error::EtherType);
        }
        let mut b = alloc::vec![0; (14 + payload.len()).max(60)];
        b[..6].copy_from_slice(&destination);
        b[6..12].copy_from_slice(&source);
        put(&mut b, 12, kind);
        b[14..14 + payload.len()].copy_from_slice(payload);
        Ok(b)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arp {
    pub operation: u16,
    pub sender_mac: Mac,
    pub sender_ip: Ip,
    pub target_mac: Mac,
    pub target_ip: Ip,
}
impl Arp {
    pub fn parse(b: &[u8]) -> Result<Self, Error> {
        if b.len() < 28 {
            return Err(Error::Length);
        }
        if word(b, 0) != 1
            || word(b, 2) != 0x0800
            || b[4] != 6
            || b[5] != 4
            || ![1, 2].contains(&word(b, 6))
        {
            return Err(Error::Header);
        }
        let a = Self {
            operation: word(b, 6),
            sender_mac: b[8..14].try_into().unwrap(),
            sender_ip: b[14..18].try_into().unwrap(),
            target_mac: b[18..24].try_into().unwrap(),
            target_ip: b[24..28].try_into().unwrap(),
        };
        if !valid_mac(a.sender_mac) || !valid_ip(a.sender_ip) || !valid_ip(a.target_ip) {
            return Err(Error::Address);
        }
        Ok(a)
    }
    pub fn encode(self) -> Result<Vec<u8>, Error> {
        let mut b = alloc::vec![0; 28];
        put(&mut b, 0, 1);
        put(&mut b, 2, 0x0800);
        b[4] = 6;
        b[5] = 4;
        put(&mut b, 6, self.operation);
        b[8..14].copy_from_slice(&self.sender_mac);
        b[14..18].copy_from_slice(&self.sender_ip);
        b[18..24].copy_from_slice(&self.target_mac);
        b[24..28].copy_from_slice(&self.target_ip);
        Self::parse(&b)?;
        Ok(b)
    }
}
#[derive(Debug)]
pub struct Ipv4<'a> {
    pub source: Ip,
    pub destination: Ip,
    pub protocol: u8,
    pub payload: &'a [u8],
}
impl<'a> Ipv4<'a> {
    /// ICMP, TCP and UDP with fixed IPv4 headers and no reassembly.
    pub fn parse(b: &'a [u8]) -> Result<Self, Error> {
        let packet = Self::parse_framing(b)?;
        if ![1, 6, 17].contains(&packet.protocol) {
            return Err(Error::Protocol);
        }
        if !valid_ip(packet.source) || !valid_ip(packet.destination) {
            return Err(Error::Address);
        }
        Ok(packet)
    }
    /// Validate the envelope independently of addressing and protocol policy.
    /// DHCP admits limited broadcast and the unassigned source separately.
    fn parse_framing(b: &'a [u8]) -> Result<Self, Error> {
        if b.len() < 20 {
            return Err(Error::Length);
        }
        if b[0] != 0x45 {
            return Err(Error::Header);
        }
        let size = word(b, 2) as usize;
        if !(20..=MTU).contains(&size) || size > b.len() {
            return Err(Error::Length);
        }
        if checksum(&b[..20]) != 0 {
            return Err(Error::Checksum);
        }
        if b[8] == 0 {
            return Err(Error::Ttl);
        }
        if word(b, 6) & 0xbfff != 0 {
            return Err(Error::Fragment);
        }
        Ok(Self {
            source: b[12..16].try_into().unwrap(),
            destination: b[16..20].try_into().unwrap(),
            protocol: b[9],
            payload: &b[20..size],
        })
    }
    pub fn encode(source: Ip, destination: Ip, payload: &[u8]) -> Result<Vec<u8>, Error> {
        Self::encode_protocol(source, destination, 1, payload)
    }
    pub fn encode_protocol(
        source: Ip,
        destination: Ip,
        protocol: u8,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        let packet = Self::encode_framing(source, destination, protocol, payload)?;
        Ipv4::parse(&packet)?;
        Ok(packet)
    }
    fn encode_framing(
        source: Ip,
        destination: Ip,
        protocol: u8,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if payload.len() > MTU - 20 {
            return Err(Error::Length);
        }
        let mut b = alloc::vec![0; 20 + payload.len()];
        b[0] = 0x45;
        let len = b.len() as u16;
        put(&mut b, 2, len);
        put(&mut b, 6, 0x4000);
        b[8] = 64;
        b[9] = protocol;
        b[12..16].copy_from_slice(&source);
        b[16..20].copy_from_slice(&destination);
        let c = checksum(&b[..20]);
        put(&mut b, 10, c);
        b[20..].copy_from_slice(payload);
        Ok(b)
    }
}
#[derive(Debug)]
pub struct Echo<'a> {
    pub reply: bool,
    pub identifier: u16,
    pub sequence: u16,
    pub data: &'a [u8],
}
impl<'a> Echo<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Self, Error> {
        if b.len() < 8 || b.len() > MTU - 20 {
            return Err(Error::Length);
        }
        if ![0, 8].contains(&b[0]) || b[1] != 0 {
            return Err(Error::Protocol);
        }
        if checksum(b) != 0 {
            return Err(Error::Checksum);
        }
        Ok(Self {
            reply: b[0] == 0,
            identifier: word(b, 4),
            sequence: word(b, 6),
            data: &b[8..],
        })
    }
    pub fn encode(
        reply: bool,
        identifier: u16,
        sequence: u16,
        data: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if data.len() > MTU - 28 {
            return Err(Error::Length);
        }
        let mut b = alloc::vec![0; 8 + data.len()];
        b[0] = if reply { 0 } else { 8 };
        put(&mut b, 4, identifier);
        put(&mut b, 6, sequence);
        b[8..].copy_from_slice(data);
        let c = checksum(&b);
        put(&mut b, 2, c);
        Ok(b)
    }
}

pub const ARP_CAPACITY: usize = 8;
pub const ARP_REACHABLE_MS: u64 = 30_000;
pub const ARP_RETRY_MS: u64 = 1_000;
pub const ARP_ATTEMPTS: u8 = 3;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Ready(Mac),
    Request,
    Wait,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeighborState {
    Incomplete,
    Reachable,
    Stale,
}
#[derive(Clone, Copy)]
enum State {
    Incomplete { attempts: u8, next: u64 },
    Reachable { mac: Mac, expires: u64 },
    Stale,
}
#[derive(Clone, Copy)]
struct Neighbor {
    ip: Ip,
    state: State,
}
/// A solicited-reply-only cache. Caller supplies monotonic milliseconds.
/// It queues no packets; retries occur only when the caller polls `resolve`.
pub struct ArpCache {
    entries: [Option<Neighbor>; ARP_CAPACITY],
    last: u64,
}
impl Default for ArpCache {
    fn default() -> Self {
        Self::new()
    }
}
impl ArpCache {
    pub const fn new() -> Self {
        Self {
            entries: [None; ARP_CAPACITY],
            last: 0,
        }
    }
    fn advance(&mut self, now: u64) -> Result<(), Error> {
        if now < self.last {
            return Err(Error::Header);
        }
        self.last = now;
        for entry in self.entries.iter_mut().flatten() {
            if let State::Reachable { expires, .. } = entry.state {
                if now >= expires {
                    entry.state = State::Stale;
                }
            }
        }
        Ok(())
    }
    pub fn state(&mut self, ip: Ip, now: u64) -> Result<Option<NeighborState>, Error> {
        self.advance(now)?;
        Ok(self
            .entries
            .iter()
            .flatten()
            .find(|n| n.ip == ip)
            .map(|n| match n.state {
                State::Incomplete { .. } => NeighborState::Incomplete,
                State::Reachable { .. } => NeighborState::Reachable,
                State::Stale => NeighborState::Stale,
            }))
    }
    pub fn resolve(&mut self, ip: Ip, now: u64) -> Result<Resolution, Error> {
        if !valid_ip(ip) {
            return Err(Error::Address);
        }
        self.advance(now)?;
        let index = self
            .entries
            .iter()
            .position(|n| n.map(|n| n.ip == ip).unwrap_or(false));
        let index = match index {
            Some(i) => i,
            None => self
                .entries
                .iter()
                .position(|n| {
                    n.is_none()
                        || matches!(
                            n,
                            Some(Neighbor {
                                state: State::Stale,
                                ..
                            })
                        )
                })
                .ok_or(Error::Capacity)?,
        };
        let state = self.entries[index]
            .filter(|n| n.ip == ip)
            .map(|n| n.state)
            .unwrap_or(State::Stale);
        match state {
            State::Reachable { mac, .. } => Ok(Resolution::Ready(mac)),
            State::Incomplete { next, .. } if now < next => Ok(Resolution::Wait),
            State::Incomplete { attempts, .. } if attempts >= ARP_ATTEMPTS => {
                self.entries[index] = None;
                Err(Error::Timeout)
            }
            _ => {
                let attempts = if let State::Incomplete { attempts, .. } = state {
                    attempts + 1
                } else {
                    1
                };
                self.entries[index] = Some(Neighbor {
                    ip,
                    state: State::Incomplete {
                        attempts,
                        next: now.saturating_add(ARP_RETRY_MS),
                    },
                });
                Ok(Resolution::Request)
            }
        }
    }
    pub fn accept(
        &mut self,
        arp: Arp,
        ethernet_source: Mac,
        local_mac: Mac,
        local_ip: Ip,
        now: u64,
    ) -> Result<(), Error> {
        self.advance(now)?;
        if arp.operation != 2
            || arp.sender_mac != ethernet_source
            || !valid_mac(arp.sender_mac)
            || arp.target_mac != local_mac
            || arp.target_ip != local_ip
        {
            return Err(Error::Address);
        }
        let entry = self
            .entries
            .iter_mut()
            .flatten()
            .find(|n| n.ip == arp.sender_ip)
            .ok_or(Error::Unsolicited)?;
        match entry.state {
            State::Incomplete { next, .. } if now < next => {
                entry.state = State::Reachable {
                    mac: arp.sender_mac,
                    expires: now.saturating_add(ARP_REACHABLE_MS),
                };
                Ok(())
            }
            _ => Err(Error::Unsolicited),
        }
    }
}

pub const ROUTE_CAPACITY: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Route {
    network: u32,
    mask: u32,
    gateway: Option<Ip>,
}
impl Route {
    fn new(network: Ip, mask: Ip, gateway: Option<Ip>) -> Result<Self, Error> {
        let network = u32::from_be_bytes(network);
        let mask = u32::from_be_bytes(mask);
        // Every set bit must belong to the leading contiguous run.
        if mask.leading_ones() != mask.count_ones() || network & !mask != 0 {
            return Err(Error::Header);
        }
        Ok(Self {
            network,
            mask,
            gateway,
        })
    }
    fn contains(self, ip: Ip) -> bool {
        u32::from_be_bytes(ip) & self.mask == self.network
    }
    fn host(self, ip: Ip) -> bool {
        self.contains(ip)
            && (self.mask.count_ones() >= 31
                || (u32::from_be_bytes(ip) != self.network
                    && u32::from_be_bytes(ip) != (self.network | !self.mask)))
    }
}

/// One interface owns its immutable connected route and seven removable routes.
/// Gateways must be unicast hosts reachable on that connected interface.
pub struct Routes {
    local: Ip,
    connected: Route,
    entries: [Option<Route>; ROUTE_CAPACITY - 1],
    mtu: usize,
}
impl Routes {
    pub fn new(local: Ip, netmask: Ip, mtu: usize) -> Result<Self, Error> {
        if !valid_ip(local) {
            return Err(Error::Address);
        }
        if !(68..=MTU).contains(&mtu) {
            return Err(Error::Length);
        }
        let network = (u32::from_be_bytes(local) & u32::from_be_bytes(netmask)).to_be_bytes();
        let connected = Route::new(network, netmask, None)?;
        if !connected.host(local) {
            return Err(Error::Address);
        }
        Ok(Self {
            local,
            connected,
            entries: [None; ROUTE_CAPACITY - 1],
            mtu,
        })
    }
    /// Duplicate prefixes are rejected; remove before replacing. Validation is
    /// atomic, and the connected route cannot be replaced or removed.
    pub fn add(&mut self, network: Ip, mask: Ip, gateway: Ip) -> Result<(), Error> {
        let route = Route::new(network, mask, Some(gateway))?;
        if !valid_ip(gateway) || gateway == self.local || !self.connected.host(gateway) {
            return Err(Error::Address);
        }
        if (route.network == self.connected.network && route.mask == self.connected.mask)
            || self
                .entries
                .iter()
                .flatten()
                .any(|r| r.network == route.network && r.mask == route.mask)
        {
            return Err(Error::Header);
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|r| r.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(route);
        Ok(())
    }
    pub fn remove(&mut self, network: Ip, mask: Ip) -> Result<(), Error> {
        let route = Route::new(network, mask, None)?;
        let slot = self
            .entries
            .iter_mut()
            .find(|r| {
                r.map(|r| r.network == route.network && r.mask == route.mask)
                    .unwrap_or(false)
            })
            .ok_or(Error::NoRoute)?;
        *slot = None;
        Ok(())
    }
    pub fn next_hop(&self, destination: Ip) -> Result<Ip, Error> {
        if !valid_ip(destination)
            || destination == self.local
            || (self.connected.contains(destination) && !self.connected.host(destination))
        {
            return Err(Error::Address);
        }
        let route = core::iter::once(&self.connected)
            .chain(self.entries.iter().flatten())
            .filter(|r| r.contains(destination))
            .max_by_key(|r| r.mask.count_ones())
            .ok_or(Error::NoRoute)?;
        Ok(route.gateway.unwrap_or(destination))
    }
}

/// Outcome of routing one transport payload through the Ethernet interface.
#[derive(Debug, PartialEq, Eq)]
pub enum Egress {
    Sent(Vec<u8>),
    Neighbor(Vec<u8>),
    Waiting,
}

/// A logical peer: owned address, bounded ARP cache and eight echo replies per
/// one-second window. Routes select outbound next hops; this endpoint does not
/// forward packets and has no NIC or service authority.
pub struct Peer {
    mac: Mac,
    ip: Ip,
    routes: Routes,
    pub neighbors: ArpCache,
    window: u64,
    replies: u8,
    udp: udp::Endpoints,
}
impl Peer {
    pub fn new(mac: Mac, ip: Ip, netmask: Ip, mtu: usize) -> Result<Self, Error> {
        if !valid_mac(mac) || !valid_ip(ip) {
            return Err(Error::Address);
        }
        Ok(Self {
            mac,
            ip,
            routes: Routes::new(ip, netmask, mtu)?,
            neighbors: ArpCache::new(),
            window: 0,
            replies: 0,
            udp: udp::Endpoints::new(),
        })
    }
    pub fn add_route(&mut self, network: Ip, mask: Ip, gateway: Ip) -> Result<(), Error> {
        self.routes.add(network, mask, gateway)
    }
    pub fn remove_route(&mut self, network: Ip, mask: Ip) -> Result<(), Error> {
        self.routes.remove(network, mask)
    }
    /// Returns an ARP request, an echo request, or None while ARP is pending.
    pub fn echo(
        &mut self,
        destination: Ip,
        identifier: u16,
        sequence: u16,
        data: &[u8],
        now: u64,
    ) -> Result<Option<Vec<u8>>, Error> {
        // Validate size before mutating neighbor state.
        if data.len() > self.routes.mtu - 28 {
            return Err(Error::Length);
        }
        let next_hop = self.routes.next_hop(destination)?;
        match self.neighbors.resolve(next_hop, now)? {
            Resolution::Wait => Ok(None),
            Resolution::Request => Ok(Some(self.arp_request(next_hop)?)),
            Resolution::Ready(mac) => Ok(Some(Ethernet::encode(
                mac,
                self.mac,
                0x0800,
                &Ipv4::encode(
                    self.ip,
                    destination,
                    &Echo::encode(false, identifier, sequence, data)?,
                )?,
            )?)),
        }
    }
    fn route_payload(
        &mut self,
        destination: Ip,
        protocol: u8,
        payload: &[u8],
        now: u64,
    ) -> Result<Egress, Error> {
        if payload.len() > self.routes.mtu - 20 {
            return Err(Error::Length);
        }
        let next = self.routes.next_hop(destination)?;
        let packet = Ipv4::encode_protocol(self.ip, destination, protocol, payload)?;
        match self.neighbors.resolve(next, now)? {
            Resolution::Wait => Ok(Egress::Waiting),
            Resolution::Request => Ok(Egress::Neighbor(self.arp_request(next)?)),
            Resolution::Ready(mac) => {
                Ethernet::encode(mac, self.mac, 0x0800, &packet).map(Egress::Sent)
            }
        }
    }

    fn arp_request(&self, target_ip: Ip) -> Result<Vec<u8>, Error> {
        Ethernet::encode(
            BROADCAST,
            self.mac,
            0x0806,
            &Arp {
                operation: 1,
                sender_mac: self.mac,
                sender_ip: self.ip,
                target_mac: [0; 6],
                target_ip,
            }
            .encode()?,
        )
    }
    /// An unresolved reverse gateway produces ARP instead of an echo reply;
    /// there is no queue, so the sender must retry its original echo request.
    pub fn receive(&mut self, bytes: &[u8], now: u64) -> Result<Option<Vec<u8>>, Error> {
        self.neighbors.advance(now)?;
        let frame = Ethernet::parse(bytes)?;
        if frame.destination != self.mac
            && !(frame.kind == 0x0806 && frame.destination == BROADCAST)
        {
            return Err(Error::Address);
        }
        if frame.kind == 0x0806 {
            let arp = Arp::parse(frame.payload)?;
            if arp.sender_mac != frame.source || arp.target_ip != self.ip {
                return Err(Error::Address);
            }
            if arp.operation == 2 {
                if frame.destination != self.mac {
                    return Err(Error::Address);
                }
                self.neighbors
                    .accept(arp, frame.source, self.mac, self.ip, now)?;
                return Ok(None);
            }
            return Ok(Some(Ethernet::encode(
                frame.source,
                self.mac,
                0x0806,
                &Arp {
                    operation: 2,
                    sender_mac: self.mac,
                    sender_ip: self.ip,
                    target_mac: arp.sender_mac,
                    target_ip: arp.sender_ip,
                }
                .encode()?,
            )?));
        }
        let ip = Ipv4::parse(frame.payload)?;
        if ip.destination != self.ip {
            return Err(Error::Address);
        }
        if ip.payload.len() + 20 > self.routes.mtu {
            return Err(Error::Length);
        }
        if ip.protocol == 17 {
            let datagram = udp::Udp::parse(ip.source, ip.destination, ip.payload)?;
            self.udp.deliver(ip.source, datagram)?;
            return Ok(None);
        }
        if ip.protocol != 1 {
            return Err(Error::Protocol);
        }
        let echo = Echo::parse(ip.payload)?;
        if echo.reply {
            return Ok(None);
        }
        if now.saturating_sub(self.window) >= 1000 {
            self.window = now;
            self.replies = 0;
        }
        if self.replies >= 8 {
            return Err(Error::RateLimit);
        }
        let next_hop = self.routes.next_hop(ip.source)?;
        let reply_mac = if next_hop == ip.source {
            frame.source
        } else {
            match self.neighbors.resolve(next_hop, now)? {
                Resolution::Ready(mac) => mac,
                Resolution::Wait => return Ok(None),
                Resolution::Request => return Ok(Some(self.arp_request(next_hop)?)),
            }
        };
        self.replies += 1;
        Ok(Some(Ethernet::encode(
            reply_mac,
            self.mac,
            0x0800,
            &Ipv4::encode(
                self.ip,
                ip.source,
                &Echo::encode(true, echo.identifier, echo.sequence, echo.data)?,
            )?,
        )?))
    }
}

pub mod dhcp;
pub mod diagnostic;
pub mod dns;
#[cfg(test)]
mod tests;
pub mod transport_diagnostic;
#[cfg(test)]
mod transport_tests;
pub mod tcp;
pub mod udp;
