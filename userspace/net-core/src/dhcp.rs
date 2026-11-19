//! Bounded DHCPv4 client for one Ethernet interface. Callers own packet I/O and
//! decide when to publish or remove the returned interface configuration.
use super::udp::{Address, Udp};
use super::{valid_mac, Error, Ethernet, Ip, Ipv4, Mac, Routes, BROADCAST, MTU, ROUTE_CAPACITY};
use alloc::vec::Vec;

#[cfg(test)]
mod tests;
mod wire;

pub const CLIENT_PORT: u16 = 68;
pub const SERVER_PORT: u16 = 67;
pub const LIMITED_BROADCAST: Ip = [255; 4];
const UNASSIGNED: Ip = [0; 4];
const REQUEST_ATTEMPTS: u8 = 4;
const MAX_RETRY_MS: u64 = 64_000;
const DECLINE_RETRY_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Init,
    Selecting,
    Requesting,
    Bound,
    Renewing,
    Rebinding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClasslessRoute {
    pub network: Ip,
    pub mask: Ip,
    /// `None` means destinations in this prefix are directly reachable.
    pub gateway: Option<Ip>,
}

impl ClasslessRoute {
    pub(super) fn install(self, routes: &mut Routes) -> Result<(), Error> {
        routes.add_via(self.network, self.mask, self.gateway)
    }
}

/// The directly connected prefix occupies the remaining route-table slot.
pub type ClasslessRoutes = [Option<ClasslessRoute>; ROUTE_CAPACITY - 1];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    pub address: Ip,
    pub mask: Ip,
    pub router: Option<Ip>,
    /// An advertised list replaces the Router option, even without a default route.
    pub routes: Option<ClasslessRoutes>,
    pub dns: [Option<Ip>; 2],
    pub server: Ip,
    pub renew_at: u64,
    pub rebind_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Transmission {
    pub source: Ip,
    pub destination: Ip,
    pub payload: Vec<u8>,
}
impl Transmission {
    /// Encode the exceptional DHCP limited-broadcast envelope without
    /// relaxing the normal IPv4 or UDP addressing rules.
    pub fn broadcast_frame(&self, mac: Mac) -> Result<Vec<u8>, Error> {
        if self.destination != LIMITED_BROADCAST
            || !(self.source == UNASSIGNED || usable_ip(self.source))
        {
            return Err(Error::Address);
        }
        frame(
            mac,
            BROADCAST,
            Address {
                ip: self.source,
                port: CLIENT_PORT,
            },
            Address {
                ip: self.destination,
                port: SERVER_PORT,
            },
            &self.payload,
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Idle,
    Send(Transmission),
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Ignored,
    Offered,
    Bound(Lease),
    Lost,
}

pub struct Client {
    mac: Mac,
    random: u32,
    state: State,
    xid: u32,
    started: u64,
    next_send: u64,
    attempts: u8,
    clock: u64,
    offer: Option<Offer>,
    lease: Option<Lease>,
    lease_xid: u32,
    decline_until: u64,
}

#[derive(Clone, Copy)]
struct Offer {
    address: Ip,
    server: Ip,
}

const fn usable_ip(ip: Ip) -> bool {
    // DHCP may configure only an IPv4 unicast host, excluding local loopback.
    ip[0] > 0 && ip[0] != 127 && ip[0] < 224
}

fn frame(
    from: Mac,
    to: Mac,
    source: Address,
    destination: Address,
    payload: &[u8],
) -> Result<Vec<u8>, Error> {
    let datagram = Udp::encode_framing(source, destination, payload)?;
    let ip = Ipv4::encode_framing(source.ip, destination.ip, 17, &datagram)?;
    Ethernet::encode(to, from, 0x0800, &ip)
}

/// Classify the DHCP UDP port pair after parsing the Ethernet and IPv4
/// envelopes. The caller must pass candidates to the DHCP client for validation.
pub fn is_candidate(frame: &[u8]) -> bool {
    ethernet_ip(frame).is_ok_and(|(_, ip)| downstream(&ip))
}

fn downstream(ip: &Ipv4<'_>) -> bool {
    ip.protocol == 17 && ip.payload.get(..4) == Some(&[0, SERVER_PORT as u8, 0, CLIENT_PORT as u8])
}

fn ethernet_ip(frame: &[u8]) -> Result<(Ethernet<'_>, Ipv4<'_>), Error> {
    let ethernet = Ethernet::parse(frame)?;
    if ethernet.kind != 0x0800 {
        return Err(Error::Protocol);
    }
    let ip = Ipv4::parse_framing(ethernet.payload)?;
    Ok((ethernet, ip))
}

impl Client {
    /// The seed diversifies transaction IDs and retransmission times. Supply
    /// fresh seed material per interface/startup.
    pub fn new(mac: Mac, seed: u32) -> Result<Self, Error> {
        if !valid_mac(mac) {
            return Err(Error::Address);
        }
        Ok(Self {
            mac,
            random: seed.max(1),
            state: State::Init,
            xid: 0,
            started: 0,
            next_send: 0,
            attempts: 0,
            clock: 0,
            offer: None,
            lease: None,
            lease_xid: 0,
            decline_until: 0,
        })
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn lease(&self) -> Option<Lease> {
        self.lease
    }

    /// Earliest time the event loop should poll the client again.
    pub fn deadline(&self) -> u64 {
        match (self.state, self.lease) {
            (State::Bound, Some(lease)) => lease.renew_at,
            (State::Renewing, Some(lease)) => self.next_send.min(lease.rebind_at),
            (State::Rebinding, Some(lease)) => self.next_send.min(lease.expires_at),
            _ => self.next_send,
        }
    }

    /// Restart negotiation after link loss. Caller must withdraw its active
    /// configuration and any connected state along with the lease.
    pub fn reset(&mut self, now_ms: u64) {
        self.state = State::Init;
        self.offer = None;
        self.lease = None;
        self.lease_xid = 0;
        self.attempts = 0;
        self.clock = self.clock.max(now_ms);
        self.next_send = self.clock.max(self.decline_until);
    }

    /// Withdraw a validated, live lease after IPv4 conflict detection.
    /// The caller keeps the address inactive and sends the returned frame.
    /// RFC 2131 section 3.1 recommends waiting at least ten seconds before
    /// restarting, even if a link transition resets negotiation meanwhile.
    pub fn decline(&mut self, now_ms: u64) -> Result<Transmission, Error> {
        let lease = self
            .lease
            .filter(|lease| now_ms < lease.expires_at)
            .ok_or(Error::NotBound)?;
        let deadline = now_ms.checked_add(DECLINE_RETRY_MS).ok_or(Error::Header)?;
        self.advance(now_ms)?;

        let payload = wire::request(
            wire::Kind::Decline,
            self.lease_xid,
            self.mac,
            0,
            UNASSIGNED,
            LIMITED_BROADCAST,
            Some(Offer {
                address: lease.address,
                server: lease.server,
            }),
        );
        self.decline_until = deadline;
        self.reset(now_ms);
        Ok(Transmission {
            source: UNASSIGNED,
            destination: LIMITED_BROADCAST,
            payload,
        })
    }

    fn advance(&mut self, now_ms: u64) -> Result<(), Error> {
        if now_ms < self.clock {
            return Err(Error::Header);
        }
        self.clock = now_ms;
        Ok(())
    }

    fn sample(&mut self) -> u32 {
        let mut value = self.random;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.random = value;
        value
    }

    fn start(&mut self, phase: State, now: u64) {
        self.xid = self.sample();
        self.state = phase;
        self.started = now;
        self.attempts = 0;
        self.next_send = now;
    }

    pub fn poll(&mut self, now_ms: u64) -> Result<Action, Error> {
        self.advance(now_ms)?;
        if let Some(lease) = self.lease {
            if now_ms >= lease.expires_at {
                self.reset(now_ms);
                return Ok(Action::Expired);
            }
            if self.state == State::Bound && now_ms >= lease.renew_at {
                self.start(
                    if now_ms >= lease.rebind_at {
                        State::Rebinding
                    } else {
                        State::Renewing
                    },
                    now_ms,
                );
            } else if self.state == State::Renewing && now_ms >= lease.rebind_at {
                self.state = State::Rebinding;
                self.attempts = 0;
                self.next_send = now_ms;
            }
        }
        if now_ms < self.deadline() || self.state == State::Bound {
            return Ok(Action::Idle);
        }
        if self.state == State::Init
            || (matches!(self.state, State::Selecting | State::Requesting)
                && self.attempts >= REQUEST_ATTEMPTS)
        {
            self.offer = None;
            self.start(State::Selecting, now_ms);
        }

        let secs = (now_ms.saturating_sub(self.started) / 1000).min(u16::MAX as u64) as u16;
        let (kind, source, destination, chosen) = match self.state {
            State::Selecting => (wire::Kind::Discover, UNASSIGNED, LIMITED_BROADCAST, None),
            State::Requesting => (
                wire::Kind::Request,
                UNASSIGNED,
                LIMITED_BROADCAST,
                self.offer,
            ),
            State::Renewing | State::Rebinding => {
                let lease = self.lease.ok_or(Error::Header)?;
                let destination = if self.state == State::Renewing {
                    lease.server
                } else {
                    LIMITED_BROADCAST
                };
                (wire::Kind::Request, lease.address, destination, None)
            }
            State::Init | State::Bound => return Ok(Action::Idle),
        };
        let payload = wire::request(kind, self.xid, self.mac, secs, source, destination, chosen);

        let delay = if let Some(lease) = self.lease {
            let limit = if self.state == State::Renewing {
                lease.rebind_at
            } else {
                lease.expires_at
            };
            ((limit - now_ms) / 2).clamp(1000, MAX_RETRY_MS)
        } else {
            let base = 3500u64 << self.attempts.min(4);
            (base + (u64::from(self.sample()) & 1023)).min(MAX_RETRY_MS)
        };
        self.attempts = self.attempts.saturating_add(1);
        self.next_send = now_ms.saturating_add(delay);
        Ok(Action::Send(Transmission {
            source,
            destination,
            payload,
        }))
    }

    pub fn receive(&mut self, frame: &[u8], now_ms: u64) -> Result<Event, Error> {
        self.advance(now_ms)?;
        if self.lease.is_some_and(|lease| now_ms >= lease.expires_at) {
            self.reset(now_ms);
            return Ok(Event::Lost);
        }
        if matches!(self.state, State::Init | State::Bound) {
            return Ok(Event::Ignored);
        }
        let Ok((ethernet, ip)) = ethernet_ip(frame) else {
            return Ok(Event::Ignored);
        };
        if !downstream(&ip) {
            return Ok(Event::Ignored);
        }
        if !(ethernet.destination == self.mac || ethernet.destination == BROADCAST)
            || !usable_ip(ip.source)
        {
            return Err(Error::Unsolicited);
        }
        let udp = Udp::parse_framing(ip.source, ip.destination, ip.payload)?;
        let reply = wire::reply(udp.data)?;
        if reply.xid != self.xid || reply.mac != self.mac {
            return Err(Error::Unsolicited);
        }
        // The relay may preserve the server's source IP (RFC 1542) or use
        // giaddr. Option 54 still names the server for selection and renewal.
        if reply.server != ip.source && reply.relay != Some(ip.source) {
            return Err(Error::Unsolicited);
        }
        let current = self.lease.map_or(UNASSIGNED, |lease| lease.address);
        if (reply.client != UNASSIGNED && reply.client != current)
            || (ip.destination != LIMITED_BROADCAST
                && ip.destination != reply.address
                && (current == UNASSIGNED || ip.destination != current))
        {
            return Err(Error::Unsolicited);
        }
        match (self.state, reply.kind) {
            (State::Selecting, wire::Kind::Offer) => {
                // Validate the candidate before selecting, without reserving
                // routes or publishing an address.
                reply.lease(now_ms, None)?;
                self.offer = Some(Offer {
                    address: reply.address,
                    server: reply.server,
                });
                self.state = State::Requesting;
                self.attempts = 0;
                self.next_send = now_ms;
                Ok(Event::Offered)
            }
            (
                State::Requesting | State::Renewing | State::Rebinding,
                wire::Kind::Ack | wire::Kind::Nak,
            ) => {
                if self.attempts == 0 {
                    return Err(Error::Unsolicited);
                }
                let expected = match self.state {
                    State::Requesting => self.offer.map(|offer| offer.server),
                    State::Renewing => self.lease.map(|lease| lease.server),
                    _ => None,
                };
                if expected.is_some_and(|server| server != reply.server) {
                    return Err(Error::Unsolicited);
                }
                if reply.kind == wire::Kind::Nak {
                    if reply.address != UNASSIGNED {
                        return Err(Error::Header);
                    }
                    // Debounce repeated NAKs while allowing a fresh transaction
                    // after the first second.
                    self.reset(now_ms);
                    self.next_send = now_ms.saturating_add(1000);
                    return Ok(Event::Lost);
                }
                let expected_address = self.offer.map_or(current, |offer| offer.address);
                if reply.address != expected_address {
                    return Err(Error::Unsolicited);
                }
                let lease = reply.lease(now_ms, self.lease)?;
                self.lease = Some(lease);
                self.lease_xid = self.xid;
                self.offer = None;
                self.state = State::Bound;
                Ok(Event::Bound(lease))
            }
            _ => Ok(Event::Ignored),
        }
    }
}

fn validate_config(
    address: Ip,
    mask: Ip,
    router: Option<Ip>,
    classless: Option<&ClasslessRoutes>,
) -> Result<(), Error> {
    if !usable_ip(address) || mask == UNASSIGNED {
        return Err(Error::Address);
    }
    let mut routes = Routes::new(address, mask, MTU)?;
    if let Some(classless) = classless {
        for route in classless.iter().flatten() {
            route.install(&mut routes)?;
        }
    } else if let Some(router) = router {
        routes.add(UNASSIGNED, UNASSIGNED, router)?;
    }
    Ok(())
}
