//! TCPv4 segment framing and checksum, independent of connection state.
use crate::{put, valid_ip, word, Error, Ip, MTU};
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
        if !(HEADER..=HEADER + MAX_DATA).contains(&bytes.len()) {
            return Err(Error::Length);
        }
        source.validate()?;
        destination.validate()?;
        if Self::ports(bytes)? != (source.port, destination.port) {
            return Err(Error::Address);
        }
        let length = usize::from(bytes[12] >> 4) * 4;
        if !(HEADER..=bytes.len()).contains(&length) || bytes[12] & 15 != 0 {
            return Err(Error::Header);
        }
        if segment_checksum(source.ip, destination.ip, bytes) != 0 {
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
        self.validate()?;
        let length = HEADER + if self.mss.is_some() { 4 } else { 0 };
        if self.data.len() > MTU - 20 - length {
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
        let checksum = segment_checksum(source.ip, destination.ip, &bytes);
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

fn segment_checksum(source: Ip, destination: Ip, bytes: &[u8]) -> u16 {
    let mut sum = 6 + bytes.len() as u32;
    for addresses in [source, destination] {
        sum += u32::from(word(&addresses, 0)) + u32::from(word(&addresses, 2));
    }
    for pair in bytes.chunks(2) {
        sum += (u32::from(pair[0]) << 8) | u32::from(pair.get(1).copied().unwrap_or(0));
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
