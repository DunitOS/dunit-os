//! Bounded logical endpoint UDP. Port ownership is local to a Peer, not a public
//! socket ABI. There is no packet queue while neighbor resolution is pending.
use super::*;
use alloc::collections::VecDeque;

pub const ENDPOINTS: usize = 8;
pub const QUEUE_DEPTH: usize = 4;
pub const MAX_PAYLOAD: usize = MTU - 28;
pub const MAX_PAYLOAD_V6: usize = MTU - 48;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Address {
    pub ip: Ip,
    pub port: u16,
}
impl Address {
    pub(crate) fn validate(self) -> Result<(), Error> {
        if !valid_ip(self.ip) || self.port == 0 {
            return Err(Error::Address);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DualAddress {
    pub ip: IpAddr,
    pub port: u16,
}
impl DualAddress {
    pub fn validate(self) -> Result<(), Error> {
        if !self.ip.is_unicast() || self.port == 0 {
            return Err(Error::Address);
        }
        Ok(())
    }

    pub fn to_v4(self) -> Option<Address> {
        match self.ip {
            IpAddr::V4(ip) => Some(Address { ip, port: self.port }),
            IpAddr::V6(_) => None,
        }
    }
}
impl From<Address> for DualAddress {
    fn from(a: Address) -> Self {
        Self {
            ip: IpAddr::V4(a.ip),
            port: a.port,
        }
    }
}
#[derive(Debug)]
pub struct Udp<'a> {
    pub source: u16,
    pub destination: u16,
    pub data: &'a [u8],
}
fn udp_checksum(source: IpAddr, destination: IpAddr, bytes: &[u8]) -> u16 {
    match (source, destination) {
        (IpAddr::V4(s), IpAddr::V4(d)) => checksum_pseudo(&s, &d, 17, bytes),
        (IpAddr::V6(s), IpAddr::V6(d)) => checksum_pseudo(&s, &d, 17, bytes),
        _ => 1,
    }
}
impl<'a> Udp<'a> {
    pub fn parse(source: Ip, destination: Ip, b: &'a [u8]) -> Result<Self, Error> {
        Self::parse_ip(IpAddr::V4(source), IpAddr::V4(destination), b)
    }

    pub fn parse_ip(source: IpAddr, destination: IpAddr, b: &'a [u8]) -> Result<Self, Error> {
        Self::validate_length(b)?;
        if !source.is_unicast() {
            return Err(Error::Address);
        }
        DualAddress {
            ip: destination,
            port: word(b, 2),
        }
        .validate()?;
        Self::parse_fields_ip(source, destination, b)
    }

    /// Validate lengths and checksum without restricting IPv4 addresses.
    pub(super) fn parse_framing(source: Ip, destination: Ip, b: &'a [u8]) -> Result<Self, Error> {
        Self::validate_length(b)?;
        Self::parse_fields_ip(IpAddr::V4(source), IpAddr::V4(destination), b)
    }

    fn validate_length(b: &[u8]) -> Result<(), Error> {
        if !(8..=MAX_PAYLOAD + 8).contains(&b.len()) || word(b, 4) as usize != b.len() {
            return Err(Error::Length);
        }
        Ok(())
    }

    fn parse_fields_ip(source: IpAddr, destination: IpAddr, b: &'a [u8]) -> Result<Self, Error> {
        // Zero checksum is allowed only in IPv4 (RFC 768); forbidden in IPv6 (RFC 8200).
        let csum = word(b, 6);
        if matches!(source, IpAddr::V6(_)) && csum == 0 {
            return Err(Error::Checksum);
        }
        if csum != 0 && udp_checksum(source, destination, b) != 0 {
            return Err(Error::Checksum);
        }
        Ok(Self {
            source: word(b, 0),
            destination: word(b, 2),
            data: &b[8..],
        })
    }

    pub fn encode(source: Address, destination: Address, data: &[u8]) -> Result<Vec<u8>, Error> {
        Self::encode_ip(source.into(), destination.into(), data)
    }

    pub fn encode_ip(
        source: DualAddress,
        destination: DualAddress,
        data: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if !source.ip.is_unicast() {
            return Err(Error::Address);
        }
        destination.validate()?;
        let max = match (source.ip, destination.ip) {
            (IpAddr::V4(_), IpAddr::V4(_)) => MAX_PAYLOAD,
            (IpAddr::V6(_), IpAddr::V6(_)) => MAX_PAYLOAD_V6,
            _ => return Err(Error::Address),
        };
        if data.len() > max {
            return Err(Error::Length);
        }
        Self::encode_framing_ip(source, destination, data)
    }

    pub(super) fn encode_framing(
        source: Address,
        destination: Address,
        data: &[u8],
    ) -> Result<Vec<u8>, Error> {
        Self::encode_framing_ip(source.into(), destination.into(), data)
    }

    fn encode_framing_ip(
        source: DualAddress,
        destination: DualAddress,
        data: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if data.len() > MAX_PAYLOAD {
            return Err(Error::Length);
        }
        let mut b = alloc::vec![0; data.len() + 8];
        put(&mut b, 0, source.port);
        put(&mut b, 2, destination.port);
        put(&mut b, 4, (data.len() + 8) as u16);
        b[8..].copy_from_slice(data);
        let c = udp_checksum(source.ip, destination.ip, &b);
        put(&mut b, 6, if c == 0 { 0xffff } else { c });
        Ok(b)
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Datagram {
    pub source: Address,
    pub data: Vec<u8>,
}
struct Endpoint {
    port: u16,
    remote: Option<Address>,
    queue: VecDeque<Datagram>,
}
pub(super) struct Endpoints {
    slots: [Option<Endpoint>; ENDPOINTS],
    next: u16,
}
impl Endpoints {
    pub fn new() -> Self {
        Self {
            slots: core::array::from_fn(|_| None),
            next: 49152,
        }
    }
    fn get(&mut self, port: u16) -> Result<&mut Endpoint, Error> {
        self.slots
            .iter_mut()
            .flatten()
            .find(|s| s.port == port)
            .ok_or(Error::NotBound)
    }
    fn bind(&mut self, requested: u16) -> Result<u16, Error> {
        if requested != 0 && self.slots.iter().flatten().any(|s| s.port == requested) {
            return Err(Error::PortInUse);
        }
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let mut port = requested;
        if port == 0 {
            // Eight endpoints cannot exhaust the 16384-port ephemeral range.
            for _ in 0..=ENDPOINTS {
                port = self.next;
                self.next = if self.next == 65535 {
                    49152
                } else {
                    self.next + 1
                };
                if !self.slots.iter().flatten().any(|s| s.port == port) {
                    break;
                }
            }
        }
        self.slots[slot] = Some(Endpoint {
            port,
            remote: None,
            queue: VecDeque::new(),
        });
        Ok(port)
    }
    pub fn deliver(&mut self, ip: Ip, udp: Udp<'_>) -> Result<(), Error> {
        let endpoint = self.get(udp.destination)?;
        let source = Address {
            ip,
            port: udp.source,
        };
        if endpoint.remote.is_some_and(|r| r != source) {
            return Err(Error::Unsolicited);
        }
        if endpoint.queue.len() == QUEUE_DEPTH {
            return Err(Error::Capacity);
        }
        endpoint.queue.push_back(Datagram {
            source,
            data: udp.data.to_vec(),
        });
        Ok(())
    }
}
pub use super::Egress as Send;
#[derive(Debug, PartialEq, Eq)]
pub struct Received {
    pub source: Address,
    pub copied: usize,
    pub original_len: usize,
}
impl Peer {
    pub fn udp_bind(&mut self, port: u16) -> Result<u16, Error> {
        self.udp.bind(port)
    }
    pub fn udp_close(&mut self, port: u16) -> Result<(), Error> {
        let slot = self
            .udp
            .slots
            .iter_mut()
            .find(|s| s.as_ref().is_some_and(|s| s.port == port))
            .ok_or(Error::NotBound)?;
        *slot = None;
        Ok(())
    }
    /// Reconnecting drops queued packets from the previous remote.
    pub fn udp_connect(&mut self, port: u16, remote: Option<Address>) -> Result<(), Error> {
        if let Some(remote) = remote {
            remote.validate()?;
            self.routes.next_hop(remote.ip)?;
        }
        let endpoint = self.udp.get(port)?;
        endpoint.remote = remote;
        endpoint.queue.clear();
        Ok(())
    }
    pub fn udp_ready(&mut self, port: u16) -> Result<bool, Error> {
        Ok(!self.udp.get(port)?.queue.is_empty())
    }
    pub fn udp_recv(&mut self, port: u16) -> Result<Datagram, Error> {
        self.udp
            .get(port)?
            .queue
            .pop_front()
            .ok_or(Error::WouldBlock)
    }
    /// Datagram semantics: excess bytes are discarded, with original length
    /// reported explicitly. A zero-sized buffer still consumes one datagram.
    pub fn udp_recv_into(&mut self, port: u16, output: &mut [u8]) -> Result<Received, Error> {
        let packet = self.udp_recv(port)?;
        let copied = packet.data.len().min(output.len());
        output[..copied].copy_from_slice(&packet.data[..copied]);
        Ok(Received {
            source: packet.source,
            copied,
            original_len: packet.data.len(),
        })
    }
    pub fn udp_send(
        &mut self,
        port: u16,
        remote: Option<Address>,
        data: &[u8],
        now: u64,
    ) -> Result<Send, Error> {
        let endpoint = self.udp.get(port)?;
        let destination = remote.or(endpoint.remote).ok_or(Error::Address)?;
        if endpoint.remote.is_some_and(|r| r != destination) {
            return Err(Error::Address);
        }
        destination.validate()?;
        if data.len() > self.routes.mtu - 28 {
            return Err(Error::Length);
        }
        let payload = Udp::encode(Address { ip: self.ip, port }, destination, data)?;
        self.route_payload(destination.ip, 17, &payload, now)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct DualDatagram {
    pub source: DualAddress,
    pub data: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DualReceived {
    pub source: DualAddress,
    pub copied: usize,
    pub original_len: usize,
}

struct DualSlot {
    port: u16,
    remote: Option<DualAddress>,
    queue: VecDeque<DualDatagram>,
}

pub struct DualEndpoints {
    slots: [Option<DualSlot>; ENDPOINTS],
    next: u16,
}

impl Default for DualEndpoints {
    fn default() -> Self {
        Self::new()
    }
}

impl DualEndpoints {
    pub fn new() -> Self {
        Self {
            slots: core::array::from_fn(|_| None),
            next: 49152,
        }
    }

    fn get(&mut self, port: u16) -> Result<&mut DualSlot, Error> {
        self.slots
            .iter_mut()
            .flatten()
            .find(|s| s.port == port)
            .ok_or(Error::NotBound)
    }

    pub fn bind(&mut self, requested: u16) -> Result<u16, Error> {
        if requested != 0 && self.slots.iter().flatten().any(|s| s.port == requested) {
            return Err(Error::PortInUse);
        }
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let mut port = requested;
        if port == 0 {
            for _ in 0..=ENDPOINTS {
                port = self.next;
                self.next = if self.next == u16::MAX {
                    49152
                } else {
                    self.next + 1
                };
                if !self.slots.iter().flatten().any(|s| s.port == port) {
                    break;
                }
            }
        }
        self.slots[slot] = Some(DualSlot {
            port,
            remote: None,
            queue: VecDeque::new(),
        });
        Ok(port)
    }

    pub fn close(&mut self, port: u16) -> Result<(), Error> {
        let slot = self
            .slots
            .iter_mut()
            .find(|s| s.as_ref().is_some_and(|s| s.port == port))
            .ok_or(Error::NotBound)?;
        *slot = None;
        Ok(())
    }

    pub fn connect(&mut self, port: u16, remote: Option<DualAddress>) -> Result<(), Error> {
        if let Some(r) = remote {
            r.validate()?;
        }
        let ep = self.get(port)?;
        ep.remote = remote;
        ep.queue.clear();
        Ok(())
    }

    pub fn deliver(&mut self, ip: IpAddr, udp: Udp<'_>) -> Result<(), Error> {
        let ep = self.get(udp.destination)?;
        let source = DualAddress {
            ip,
            port: udp.source,
        };
        if ep.remote.is_some_and(|r| r != source) {
            return Err(Error::Unsolicited);
        }
        if ep.queue.len() == QUEUE_DEPTH {
            return Err(Error::Capacity);
        }
        ep.queue.push_back(DualDatagram {
            source,
            data: udp.data.to_vec(),
        });
        Ok(())
    }

    pub fn ready(&mut self, port: u16) -> Result<bool, Error> {
        Ok(!self.get(port)?.queue.is_empty())
    }

    pub fn recv_into(&mut self, port: u16, output: &mut [u8]) -> Result<DualReceived, Error> {
        let pkt = self
            .get(port)?
            .queue
            .pop_front()
            .ok_or(Error::WouldBlock)?;
        let copied = pkt.data.len().min(output.len());
        output[..copied].copy_from_slice(&pkt.data[..copied]);
        Ok(DualReceived {
            source: pkt.source,
            copied,
            original_len: pkt.data.len(),
        })
    }
}
