//! TCPv4/TCPv6 segment framing and checksum, independent of connection state.
use crate::{checksum_pseudo, put, valid_ip, word, Error, Ip, IpAddr, MTU};
use alloc::vec::Vec;

pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;
pub const ACK: u8 = 0x10;
const URG: u8 = 0x20;

pub const HEADER: usize = 20;
pub const MAX_DATA: usize = MTU - 20 - HEADER;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub ip: Ip,
    pub port: u16,
}

impl Endpoint {
    pub(super) fn validate(self) -> Result<(), Error> {
        if self.port == 0 || !valid_ip(self.ip) {
            return Err(Error::Address);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DualEndpoint {
    pub ip: IpAddr,
    pub port: u16,
}

impl DualEndpoint {
    pub fn validate(self) -> Result<(), Error> {
        if self.port == 0 || !self.ip.is_unicast() {
            return Err(Error::Address);
        }
        Ok(())
    }

    pub fn to_v4(self) -> Option<Endpoint> {
        match self.ip {
            IpAddr::V4(ip) => Some(Endpoint { ip, port: self.port }),
            IpAddr::V6(_) => None,
        }
    }
}

impl From<Endpoint> for DualEndpoint {
    fn from(ep: Endpoint) -> Self {
        Self {
            ip: IpAddr::V4(ep.ip),
            port: ep.port,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment<'a> {
    pub sequence: u32,
    pub acknowledgement: u32,
    pub flags: u8,
    pub window: u16,
    pub mss: Option<u16>,
    pub data: &'a [u8],
}

impl<'a> Segment<'a> {
    pub fn ports(bytes: &[u8]) -> Result<(u16, u16), Error> {
        if bytes.len() < HEADER {
            return Err(Error::Length);
        }
        Ok((word(bytes, 0), word(bytes, 2)))
    }

    pub fn parse(source: Endpoint, destination: Endpoint, bytes: &'a [u8]) -> Result<Self, Error> {
        source.validate()?;
        destination.validate()?;
        Self::parse_ip(source.into(), destination.into(), bytes)
    }

    pub fn parse_ip(
        source: DualEndpoint,
        destination: DualEndpoint,
        bytes: &'a [u8],
    ) -> Result<Self, Error> {
        source.validate()?;
        destination.validate()?;
        let ip_header = match (source.ip, destination.ip) {
            (IpAddr::V4(_), IpAddr::V4(_)) => 20,
            (IpAddr::V6(_), IpAddr::V6(_)) => 40,
            _ => return Err(Error::Address),
        };
        if !(HEADER..=MTU - ip_header).contains(&bytes.len()) {
            return Err(Error::Length);
        }
        if Self::ports(bytes)? != (source.port, destination.port) {
            return Err(Error::Address);
        }
        let length = usize::from(bytes[12] >> 4) * 4;
        if !(HEADER..=bytes.len()).contains(&length) || bytes[12] & 15 != 0 {
            return Err(Error::Header);
        }
        if segment_checksum_ip(source.ip, destination.ip, bytes) != 0 {
            return Err(Error::Checksum);
        }
        if word(bytes, 18) != 0 {
            return Err(Error::Header);
        }
        let segment = Self {
            sequence: u32::from_be_bytes(bytes[4..8].try_into().unwrap()),
            acknowledgement: u32::from_be_bytes(bytes[8..12].try_into().unwrap()),
            flags: bytes[13],
            window: word(bytes, 14),
            mss: parse_options(&bytes[HEADER..length])?,
            data: &bytes[length..],
        };
        segment.validate()?;
        Ok(segment)
    }

    pub fn encode(self, source: Endpoint, destination: Endpoint) -> Result<Vec<u8>, Error> {
        source.validate()?;
        destination.validate()?;
        self.encode_ip(source.into(), destination.into())
    }

    pub fn encode_ip(
        self,
        source: DualEndpoint,
        destination: DualEndpoint,
    ) -> Result<Vec<u8>, Error> {
        source.validate()?;
        destination.validate()?;
        let ip_header = match (source.ip, destination.ip) {
            (IpAddr::V4(_), IpAddr::V4(_)) => 20,
            (IpAddr::V6(_), IpAddr::V6(_)) => 40,
            _ => return Err(Error::Address),
        };
        self.validate()?;
        let length = HEADER + if self.mss.is_some() { 4 } else { 0 };
        if self.data.len() > MTU - ip_header - length {
            return Err(Error::Length);
        }
        let mut bytes = alloc::vec![0; length + self.data.len()];
        put(&mut bytes, 0, source.port);
        put(&mut bytes, 2, destination.port);
        bytes[4..8].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[8..12].copy_from_slice(&self.acknowledgement.to_be_bytes());
        bytes[12] = (length / 4) as u8 * 16;
        bytes[13] = self.flags;
        put(&mut bytes, 14, self.window);
        if let Some(size) = self.mss {
            bytes[20..22].copy_from_slice(&[2, 4]);
            put(&mut bytes, 22, size);
        }
        bytes[length..].copy_from_slice(self.data);
        let checksum = segment_checksum_ip(source.ip, destination.ip, &bytes);
        put(&mut bytes, 16, checksum);
        Ok(bytes)
    }

    pub(super) const fn has(self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    fn validate(self) -> Result<(), Error> {
        if self.has(URG)
            || (self.has(SYN) && (self.has(FIN | RST) || !self.data.is_empty()))
            || (self.has(RST) && (self.has(FIN) || !self.data.is_empty()))
            || self.mss.is_some_and(|size| !self.has(SYN) || size == 0)
        {
            return Err(Error::Header);
        }
        Ok(())
    }
}

fn parse_options(mut bytes: &[u8]) -> Result<Option<u16>, Error> {
    let mut mss = None;
    while let Some(&kind) = bytes.first() {
        match kind {
            0 => {
                if bytes[1..].iter().any(|&byte| byte != 0) {
                    return Err(Error::Header);
                }
                break;
            }
            1 => bytes = &bytes[1..],
            _ => {
                let length = usize::from(*bytes.get(1).ok_or(Error::Header)?);
                if length < 2 || length > bytes.len() {
                    return Err(Error::Header);
                }
                if kind == 2 {
                    if length != 4 || mss.is_some() {
                        return Err(Error::Header);
                    }
                    mss = Some(word(bytes, 2));
                }
                bytes = &bytes[length..];
            }
        }
    }
    Ok(mss)
}

fn segment_checksum_ip(source: IpAddr, destination: IpAddr, bytes: &[u8]) -> u16 {
    match (source, destination) {
        (IpAddr::V4(s), IpAddr::V4(d)) => checksum_pseudo(&s, &d, 6, bytes),
        (IpAddr::V6(s), IpAddr::V6(d)) => checksum_pseudo(&s, &d, 6, bytes),
        _ => 1,
    }
}
