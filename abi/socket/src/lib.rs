//! Fixed-width v1 IPv4 socket and kernel-to-service bridge records.
#![no_std]

pub const VERSION: u16 = 1;

pub const PROTO_UDP: u16 = 1;
pub const PROTO_TCP: u16 = 2;
pub const SCOPE_UDP: u32 = 1;
pub const SCOPE_TCP: u32 = 2;

pub const OP_CONNECT: u16 = 1;
pub const OP_SEND_TO: u16 = 2;
pub const OP_RECV_FROM: u16 = 3;
pub const OP_SHUTDOWN_WRITE: u16 = 4;
pub const OP_ERROR: u16 = 5;
pub const OP_ENDPOINTS: u16 = 6;

pub const BRIDGE_OPEN: u16 = 1;
pub const BRIDGE_CONNECT: u16 = 2;
pub const BRIDGE_SEND: u16 = 3;
pub const BRIDGE_SHUTDOWN: u16 = 4;
pub const BRIDGE_CLOSE: u16 = 5;
pub const BRIDGE_COMPLETE: u16 = 6;
pub const BRIDGE_DATA: u16 = 7;
pub const BRIDGE_EOF: u16 = 8;
pub const BRIDGE_ERROR: u16 = 9;
pub const BRIDGE_CREDIT: u16 = 10;

pub const OPTIONS_LEN: usize = 8;
pub const ENDPOINT_LEN: usize = 8;
pub const RECV_HEADER_LEN: usize = 16;
pub const ENDPOINTS_LEN: usize = 16;
pub const BRIDGE_HEADER_LEN: usize = 40;
pub const MAX_PAYLOAD: usize = 4096;
pub const MAX_FRAME: usize = BRIDGE_HEADER_LEN + MAX_PAYLOAD;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    Invalid,
    BufferTooSmall,
}

fn field<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
    bytes[offset..offset + N].try_into().unwrap()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Options;

impl Options {
    pub fn encode(self) -> [u8; OPTIONS_LEN] {
        let mut bytes = [0; OPTIONS_LEN];
        bytes[..2].copy_from_slice(&VERSION.to_le_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != OPTIONS_LEN
            || u16::from_le_bytes(field(bytes, 0)) != VERSION
            || bytes[2..] != [0; OPTIONS_LEN - 2]
        {
            return Err(WireError::Invalid);
        }
        Ok(Self)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Endpoint {
    pub ip: [u8; 4],
    pub port: u16,
}

impl Endpoint {
    pub const NONE: Self = Self { ip: [0; 4], port: 0 };

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    /// An IPv4 unicast endpoint with a nonzero port.
    pub fn is_unicast(self) -> bool {
        self.port != 0 && self.ip[0] > 0 && self.ip[0] < 224 && self.ip[0] != 127
    }

    pub fn encode(self) -> [u8; ENDPOINT_LEN] {
        let mut bytes = [0; ENDPOINT_LEN];
        bytes[..4].copy_from_slice(&self.ip);
        bytes[4..6].copy_from_slice(&self.port.to_le_bytes());
        bytes
    }

    /// Check the representation. The operation decides whether an unspecified
    /// endpoint is accepted.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != ENDPOINT_LEN || bytes[6..] != [0; 2] {
            return Err(WireError::Invalid);
        }
        Ok(Self {
            ip: field(bytes, 0),
            port: u16::from_le_bytes(field(bytes, 4)),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecvHeader {
    pub endpoint: Endpoint,
    /// Datagram bytes before truncation to the caller's destination capacity.
    pub full_len: u32,
}

impl RecvHeader {
    pub fn encode(self) -> [u8; RECV_HEADER_LEN] {
        let mut bytes = [0; RECV_HEADER_LEN];
        bytes[..ENDPOINT_LEN].copy_from_slice(&self.endpoint.encode());
        bytes[8..12].copy_from_slice(&self.full_len.to_le_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != RECV_HEADER_LEN || bytes[12..] != [0; 4] {
            return Err(WireError::Invalid);
        }
        let full_len = u32::from_le_bytes(field(bytes, 8));
        if full_len as usize > MAX_PAYLOAD {
            return Err(WireError::Invalid);
        }
        Ok(Self {
            endpoint: Endpoint::decode(&bytes[..ENDPOINT_LEN])?,
            full_len,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Endpoints {
    pub local: Endpoint,
    pub remote: Endpoint,
}

impl Endpoints {
    pub fn encode(self) -> [u8; ENDPOINTS_LEN] {
        let mut bytes = [0; ENDPOINTS_LEN];
        bytes[..ENDPOINT_LEN].copy_from_slice(&self.local.encode());
        bytes[ENDPOINT_LEN..].copy_from_slice(&self.remote.encode());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != ENDPOINTS_LEN {
            return Err(WireError::Invalid);
        }
        Ok(Self {
            local: Endpoint::decode(&bytes[..ENDPOINT_LEN])?,
            remote: Endpoint::decode(&bytes[ENDPOINT_LEN..])?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BridgeHeader {
    pub token: u64,
    pub owner: u64,
    pub kind: u16,
    /// The completed request kind for COMPLETE. Zero for other records.
    pub flags: u16,
    pub error: i32,
    pub endpoint: Endpoint,
    pub protocol: u16,
    /// The decoded payload length. Encoding derives it from the supplied slice.
    pub length: u32,
}

impl BridgeHeader {
    fn validate(self) -> Result<(), WireError> {
        let flags_valid = match self.kind {
            BRIDGE_COMPLETE => matches!(self.flags, BRIDGE_OPEN..=BRIDGE_SHUTDOWN),
            BRIDGE_OPEN..=BRIDGE_CREDIT => self.flags == 0,
            _ => false,
        };
        if !flags_valid
            || !matches!(self.protocol, PROTO_UDP | PROTO_TCP)
            || self.token == 0
            || self.owner == 0
            || self.error > 0
        {
            return Err(WireError::Invalid);
        }
        Ok(())
    }

    /// Decode one exact record. Direction, state and operation-specific
    /// payload rules belong to the authenticated bridge owner.
    pub fn decode(bytes: &[u8]) -> Result<(Self, &[u8]), WireError> {
        let header = bytes.get(..BRIDGE_HEADER_LEN).ok_or(WireError::Invalid)?;
        let payload = &bytes[BRIDGE_HEADER_LEN..];
        let result = Self {
            token: u64::from_le_bytes(field(header, 0)),
            owner: u64::from_le_bytes(field(header, 8)),
            kind: u16::from_le_bytes(field(header, 16)),
            flags: u16::from_le_bytes(field(header, 18)),
            error: i32::from_le_bytes(field(header, 20)),
            endpoint: Endpoint {
                ip: field(header, 24),
                port: u16::from_le_bytes(field(header, 28)),
            },
            protocol: u16::from_le_bytes(field(header, 30)),
            length: u32::from_le_bytes(field(header, 32)),
        };
        if payload.len() > MAX_PAYLOAD
            || payload.len() != result.length as usize
            || header[36..40] != [0; 4]
        {
            return Err(WireError::Invalid);
        }
        result.validate()?;
        Ok((result, payload))
    }

    /// Encode one exact record into caller-owned storage. Errors leave output
    /// untouched.
    pub fn encode(self, payload: &[u8], out: &mut [u8]) -> Result<usize, WireError> {
        self.validate()?;
        if payload.len() > MAX_PAYLOAD {
            return Err(WireError::Invalid);
        }
        let len = BRIDGE_HEADER_LEN + payload.len();
        let bytes = out.get_mut(..len).ok_or(WireError::BufferTooSmall)?;
        bytes[..8].copy_from_slice(&self.token.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.owner.to_le_bytes());
        bytes[16..18].copy_from_slice(&self.kind.to_le_bytes());
        bytes[18..20].copy_from_slice(&self.flags.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.error.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.endpoint.ip);
        bytes[28..30].copy_from_slice(&self.endpoint.port.to_le_bytes());
        bytes[30..32].copy_from_slice(&self.protocol.to_le_bytes());
        bytes[32..36].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[36..40].fill(0);
        bytes[BRIDGE_HEADER_LEN..].copy_from_slice(payload);
        Ok(len)
    }
}

#[cfg(test)]
mod tests;
