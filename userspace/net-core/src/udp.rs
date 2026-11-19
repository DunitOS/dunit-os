//! Bounded logical endpoint UDP. Port ownership is local to a Peer, not a public
//! socket ABI. There is no packet queue while neighbor resolution is pending.
use super::*;
use alloc::collections::VecDeque;

pub const ENDPOINTS: usize = 8;
pub const QUEUE_DEPTH: usize = 4;
pub const MAX_PAYLOAD: usize = MTU - 28;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Address {
    pub ip: Ip,
    pub port: u16,
}
impl Address {
    fn validate(self) -> Result<(), Error> {
        if !valid_ip(self.ip) || self.port == 0 {
            return Err(Error::Address);
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct Udp<'a> {
    pub source: u16,
    pub destination: u16,
    pub data: &'a [u8],
}
fn udp_checksum(source: Ip, destination: Ip, bytes: &[u8]) -> u16 {
    // Bounded by MTU; the temporary pseudoheader never contains Ethernet padding.
    let mut pseudo = Vec::with_capacity(12 + bytes.len());
    pseudo.extend_from_slice(&source);
    pseudo.extend_from_slice(&destination);
    pseudo.extend_from_slice(&[0, 17]);
    pseudo.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(bytes);
    checksum(&pseudo)
}
impl<'a> Udp<'a> {
    pub fn parse(source: Ip, destination: Ip, b: &'a [u8]) -> Result<Self, Error> {
        Self::validate_length(b)?;
        // RFC 768 allows an omitted source port (zero); destination zero is
        // reserved. Bound local endpoints always transmit nonzero source ports.
        if !valid_ip(source) {
            return Err(Error::Address);
        }
        Address {
            ip: destination,
            port: word(b, 2),
        }
        .validate()?;
        Self::parse_fields(source, destination, b)
    }
    /// Validate lengths and checksum without restricting IPv4 addresses.
    pub(super) fn parse_framing(source: Ip, destination: Ip, b: &'a [u8]) -> Result<Self, Error> {
        Self::validate_length(b)?;
        Self::parse_fields(source, destination, b)
    }

    fn validate_length(b: &[u8]) -> Result<(), Error> {
        if !(8..=MAX_PAYLOAD + 8).contains(&b.len()) || word(b, 4) as usize != b.len() {
            return Err(Error::Length);
        }
        Ok(())
    }

    fn parse_fields(source: Ip, destination: Ip, b: &'a [u8]) -> Result<Self, Error> {
        // A zero checksum explicitly means absent for IPv4 UDP (RFC 768).
        if word(b, 6) != 0 && udp_checksum(source, destination, b) != 0 {
            return Err(Error::Checksum);
        }
        Ok(Self {
            source: word(b, 0),
            destination: word(b, 2),
            data: &b[8..],
        })
    }
    pub fn encode(source: Address, destination: Address, data: &[u8]) -> Result<Vec<u8>, Error> {
        if !valid_ip(source.ip) {
            return Err(Error::Address);
        }
        destination.validate()?;
        Self::encode_framing(source, destination, data)
    }
    pub(super) fn encode_framing(source: Address, destination: Address, data: &[u8]) -> Result<Vec<u8>, Error> {
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
#[derive(Debug, PartialEq, Eq)]
pub enum Send {
    /// Caller must exchange this ARP frame and retry the datagram.
    Neighbor(Vec<u8>),
    Waiting,
    Sent(Vec<u8>),
}
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
        let next = self.routes.next_hop(destination.ip)?;
        let payload = Udp::encode(Address { ip: self.ip, port }, destination, data)?;
        match self.neighbors.resolve(next, now)? {
            Resolution::Wait => Ok(Send::Waiting),
            Resolution::Request => Ok(Send::Neighbor(self.arp_request(next)?)),
            Resolution::Ready(mac) => Ok(Send::Sent(Ethernet::encode(
                mac,
                self.mac,
                0x0800,
                &Ipv4::encode_protocol(self.ip, destination.ip, 17, &payload)?,
            )?)),
        }
    }
}
