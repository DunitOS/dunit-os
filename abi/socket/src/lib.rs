//! Fixed-width v1/v2 dual-stack socket and kernel-to-service bridge records.
#![no_std]

pub const VERSION: u16 = 1;
pub const VERSION_V2: u16 = 2;

pub const PROTO_UDP: u16 = 1;
pub const PROTO_TCP: u16 = 2;
pub const PROTO_DNS: u16 = 3;
pub const PROTO_ICMP: u16 = 4;
pub const PROTO_STATUS: u16 = 5;
pub const MAX_DNS_NAME: usize = 253;
pub const MAX_ICMP_PAYLOAD: usize = 256;
pub const FAMILY_NONE: u8 = 0;
pub const FAMILY_IPV4: u8 = 4;
pub const FAMILY_IPV6: u8 = 6;
pub const SCOPE_UDP: u32 = 1;
pub const SCOPE_TCP: u32 = 2;
pub const SCOPE_UDP_BIND: u32 = 4;
pub const SCOPE_TCP_LISTEN: u32 = 8;
pub const SCOPE_LOW_PORT: u32 = 16;
pub const SCOPE_STATUS: u32 = 32;
pub const SCOPE_PING: u32 = 64;

pub const OP_CONNECT: u16 = 1;
pub const OP_SEND_TO: u16 = 2;
pub const OP_RECV_FROM: u16 = 3;
pub const OP_SHUTDOWN_WRITE: u16 = 4;
pub const OP_ERROR: u16 = 5;
pub const OP_ENDPOINTS: u16 = 6;
pub const OP_BIND: u16 = 7;
pub const OP_LISTEN: u16 = 8;
pub const OP_ACCEPT: u16 = 9;

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
pub const BRIDGE_BIND: u16 = 11;
pub const BRIDGE_LISTEN: u16 = 12;
pub const BRIDGE_ACCEPT: u16 = 13;
pub const BRIDGE_LISTENER_READY: u16 = 14;

pub const OPTIONS_LEN: usize = 8;
pub const ENDPOINT_LEN: usize = 8;
pub const RECV_HEADER_LEN: usize = 16;
pub const ENDPOINTS_LEN: usize = 16;
pub const BRIDGE_HEADER_LEN: usize = 40;
pub const ENDPOINT6_LEN: usize = 24;
pub const RECV_HEADER6_LEN: usize = 32;
pub const ENDPOINTS6_LEN: usize = 48;
pub const BRIDGE_HEADER6_LEN: usize = 56;
pub const MAX_PAYLOAD: usize = 4096;
pub const MAX_FRAME: usize = BRIDGE_HEADER_LEN + MAX_PAYLOAD;
pub const MAX_FRAME_V2: usize = BRIDGE_HEADER6_LEN + MAX_PAYLOAD;
pub const DNS_RECORDS_HEADER_LEN: usize = 2;
pub const DNS_RECORD_LEN: usize = 22;
pub const MAX_DNS_RECORDS: usize = 16;

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
        Self::encode_version(VERSION)
    }

    pub const fn encode_version(version: u16) -> [u8; OPTIONS_LEN] {
        let ver = version.to_le_bytes();
        [ver[0], ver[1], 0, 0, 0, 0, 0, 0]
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if Self::decode_version(bytes)? != VERSION {
            return Err(WireError::Invalid);
        }
        Ok(Self)
    }

    pub fn decode_version(bytes: &[u8]) -> Result<u16, WireError> {
        if bytes.len() != OPTIONS_LEN || bytes[2..] != [0; OPTIONS_LEN - 2] {
            return Err(WireError::Invalid);
        }
        let version = u16::from_le_bytes(field(bytes, 0));
        if !matches!(version, VERSION | VERSION_V2) {
            return Err(WireError::Invalid);
        }
        Ok(version)
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
        self.port != 0 && self.is_unicast_ip()
    }

    pub fn is_unicast_ip(self) -> bool {
        self.ip[0] > 0 && self.ip[0] < 224 && self.ip[0] != 127
    }

    pub fn is_bind_address(self) -> bool {
        self.ip == [0; 4] || self.is_unicast_ip()
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
        validate_bridge_fields(
            self.kind,
            self.flags,
            self.protocol,
            self.token,
            self.owner,
            self.error,
        )
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

fn validate_bridge_fields(
    kind: u16,
    flags: u16,
    protocol: u16,
    token: u64,
    owner: u64,
    error: i32,
) -> Result<(), WireError> {
    let flags_valid = match kind {
        BRIDGE_COMPLETE => matches!(
            flags,
            BRIDGE_OPEN..=BRIDGE_SHUTDOWN | BRIDGE_BIND..=BRIDGE_ACCEPT
        ),
        BRIDGE_LISTENER_READY => flags <= 1,
        BRIDGE_OPEN..=BRIDGE_CREDIT | BRIDGE_BIND..=BRIDGE_ACCEPT => flags == 0,
        _ => false,
    };
    if !flags_valid
        || !matches!(
            protocol,
            PROTO_UDP | PROTO_TCP | PROTO_DNS | PROTO_ICMP | PROTO_STATUS
        )
        || token == 0
        || owner == 0
        || error > 0
    {
        return Err(WireError::Invalid);
    }
    Ok(())
}

pub fn is_unicast_ipv4(ip: [u8; 4]) -> bool {
    ip[0] > 0 && ip[0] < 224 && ip[0] != 127
}

pub fn is_unicast_ipv6(ip: [u8; 16]) -> bool {
    const LOOPBACK: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    const V4_MAPPED_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff];
    ip != [0; 16] && ip != LOOPBACK && ip[0] != 0xff && ip[..12] != V4_MAPPED_PREFIX
}

pub fn is_link_local_ipv6(ip: [u8; 16]) -> bool {
    ip[0] == 0xfe && (ip[1] & 0xc0) == 0x80
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Endpoint6 {
    pub family: u8,
    pub port: u16,
    pub scope_id: u32,
    pub addr: [u8; 16],
}

impl Endpoint6 {
    pub const NONE: Self = Self {
        family: FAMILY_NONE,
        port: 0,
        scope_id: 0,
        addr: [0; 16],
    };

    pub fn ipv4(ip: [u8; 4], port: u16) -> Self {
        let mut addr = [0; 16];
        addr[..4].copy_from_slice(&ip);
        Self {
            family: FAMILY_IPV4,
            port,
            scope_id: 0,
            addr,
        }
    }

    pub fn ipv6(addr: [u8; 16], port: u16, scope_id: u32) -> Self {
        Self {
            family: FAMILY_IPV6,
            port,
            scope_id,
            addr,
        }
    }

    pub fn from_v4(ep: Endpoint) -> Self {
        if ep.is_none() {
            Self::NONE
        } else {
            Self::ipv4(ep.ip, ep.port)
        }
    }

    pub fn bind_from_v4(ep: Endpoint) -> Self {
        Self::ipv4(ep.ip, ep.port)
    }

    pub fn to_v4(self) -> Option<Endpoint> {
        match self.family {
            FAMILY_NONE => Some(Endpoint::NONE),
            FAMILY_IPV4 => Some(Endpoint {
                ip: field(&self.addr, 0),
                port: self.port,
            }),
            _ => None,
        }
    }

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    pub fn is_unicast_ip(self) -> bool {
        match self.family {
            FAMILY_IPV4 => self.scope_id == 0 && self.addr[4..] == [0; 12] && is_unicast_ipv4(field(&self.addr, 0)),
            FAMILY_IPV6 => is_unicast_ipv6(self.addr),
            _ => false,
        }
    }

    pub fn is_unicast(self) -> bool {
        self.port != 0 && self.is_unicast_ip()
    }

    pub fn is_bind_address(self) -> bool {
        match self.family {
            FAMILY_IPV4 => self.scope_id == 0 && self.addr[4..] == [0; 12] && (self.addr == [0; 16] || self.is_unicast_ip()),
            FAMILY_IPV6 => self.addr == [0; 16] || self.is_unicast_ip(),
            _ => false,
        }
    }

    pub fn is_valid_representation(self) -> bool {
        match self.family {
            FAMILY_NONE => self.port == 0 && self.scope_id == 0 && self.addr == [0; 16],
            FAMILY_IPV4 => self.scope_id == 0 && self.addr[4..] == [0; 12],
            FAMILY_IPV6 => true,
            _ => false,
        }
    }

    pub fn encode(self) -> [u8; ENDPOINT6_LEN] {
        let mut bytes = [0; ENDPOINT6_LEN];
        bytes[0] = self.family;
        bytes[1] = 0;
        bytes[2..4].copy_from_slice(&self.port.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.scope_id.to_le_bytes());
        bytes[8..24].copy_from_slice(&self.addr);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != ENDPOINT6_LEN || bytes[1] != 0 {
            return Err(WireError::Invalid);
        }
        let endpoint = Self {
            family: bytes[0],
            port: u16::from_le_bytes(field(bytes, 2)),
            scope_id: u32::from_le_bytes(field(bytes, 4)),
            addr: field(bytes, 8),
        };
        if !endpoint.is_valid_representation() {
            return Err(WireError::Invalid);
        }
        Ok(endpoint)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecvHeader6 {
    pub endpoint: Endpoint6,
    pub full_len: u32,
}

impl RecvHeader6 {
    pub fn encode(self) -> [u8; RECV_HEADER6_LEN] {
        let mut bytes = [0; RECV_HEADER6_LEN];
        bytes[..ENDPOINT6_LEN].copy_from_slice(&self.endpoint.encode());
        bytes[24..28].copy_from_slice(&self.full_len.to_le_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != RECV_HEADER6_LEN || bytes[28..] != [0; 4] {
            return Err(WireError::Invalid);
        }
        let full_len = u32::from_le_bytes(field(bytes, 24));
        if full_len as usize > MAX_PAYLOAD {
            return Err(WireError::Invalid);
        }
        Ok(Self {
            endpoint: Endpoint6::decode(&bytes[..ENDPOINT6_LEN])?,
            full_len,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Endpoints6 {
    pub local: Endpoint6,
    pub remote: Endpoint6,
}

impl Endpoints6 {
    pub fn encode(self) -> [u8; ENDPOINTS6_LEN] {
        let mut bytes = [0; ENDPOINTS6_LEN];
        bytes[..ENDPOINT6_LEN].copy_from_slice(&self.local.encode());
        bytes[ENDPOINT6_LEN..].copy_from_slice(&self.remote.encode());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != ENDPOINTS6_LEN {
            return Err(WireError::Invalid);
        }
        Ok(Self {
            local: Endpoint6::decode(&bytes[..ENDPOINT6_LEN])?,
            remote: Endpoint6::decode(&bytes[ENDPOINT6_LEN..])?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BridgeHeader6 {
    pub token: u64,
    pub owner: u64,
    pub kind: u16,
    pub flags: u16,
    pub error: i32,
    pub endpoint: Endpoint6,
    pub protocol: u16,
    pub length: u32,
}

impl BridgeHeader6 {
    pub fn from_v1(h: BridgeHeader) -> Self {
        Self {
            token: h.token,
            owner: h.owner,
            kind: h.kind,
            flags: h.flags,
            error: h.error,
            endpoint: if h.kind == BRIDGE_BIND {
                Endpoint6::bind_from_v4(h.endpoint)
            } else {
                Endpoint6::from_v4(h.endpoint)
            },
            protocol: h.protocol,
            length: h.length,
        }
    }

    pub fn to_v1(self) -> Result<BridgeHeader, WireError> {
        let endpoint = self.endpoint.to_v4().ok_or(WireError::Invalid)?;
        Ok(BridgeHeader {
            token: self.token,
            owner: self.owner,
            kind: self.kind,
            flags: self.flags,
            error: self.error,
            endpoint,
            protocol: self.protocol,
            length: self.length,
        })
    }

    fn validate(self) -> Result<(), WireError> {
        if !self.endpoint.is_valid_representation() {
            return Err(WireError::Invalid);
        }
        validate_bridge_fields(
            self.kind,
            self.flags,
            self.protocol,
            self.token,
            self.owner,
            self.error,
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<(Self, &[u8]), WireError> {
        let header = bytes.get(..BRIDGE_HEADER6_LEN).ok_or(WireError::Invalid)?;
        let payload = &bytes[BRIDGE_HEADER6_LEN..];
        let result = Self {
            token: u64::from_le_bytes(field(header, 0)),
            owner: u64::from_le_bytes(field(header, 8)),
            kind: u16::from_le_bytes(field(header, 16)),
            flags: u16::from_le_bytes(field(header, 18)),
            error: i32::from_le_bytes(field(header, 20)),
            endpoint: Endpoint6::decode(&header[24..48])?,
            protocol: u16::from_le_bytes(field(header, 48)),
            length: u32::from_le_bytes(field(header, 52)),
        };
        if payload.len() > MAX_PAYLOAD
            || payload.len() != result.length as usize
            || header[50..52] != [0; 2]
        {
            return Err(WireError::Invalid);
        }
        result.validate()?;
        Ok((result, payload))
    }

    pub fn encode(self, payload: &[u8], out: &mut [u8]) -> Result<usize, WireError> {
        self.validate()?;
        if payload.len() > MAX_PAYLOAD {
            return Err(WireError::Invalid);
        }
        let len = BRIDGE_HEADER6_LEN + payload.len();
        let bytes = out.get_mut(..len).ok_or(WireError::BufferTooSmall)?;
        bytes[..8].copy_from_slice(&self.token.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.owner.to_le_bytes());
        bytes[16..18].copy_from_slice(&self.kind.to_le_bytes());
        bytes[18..20].copy_from_slice(&self.flags.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.error.to_le_bytes());
        bytes[24..48].copy_from_slice(&self.endpoint.encode());
        bytes[48..50].copy_from_slice(&self.protocol.to_le_bytes());
        bytes[50..52].fill(0);
        bytes[52..56].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[BRIDGE_HEADER6_LEN..].copy_from_slice(payload);
        Ok(len)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DnsRecord {
    pub family: u8,
    pub scope_id: u32,
    pub addr: [u8; 16],
}

impl DnsRecord {
    pub fn ipv4(ip: [u8; 4]) -> Self {
        let mut addr = [0; 16];
        addr[..4].copy_from_slice(&ip);
        Self {
            family: FAMILY_IPV4,
            scope_id: 0,
            addr,
        }
    }

    pub fn ipv6(addr: [u8; 16], scope_id: u32) -> Self {
        Self {
            family: FAMILY_IPV6,
            scope_id,
            addr,
        }
    }

    pub fn is_valid(self) -> bool {
        match self.family {
            FAMILY_IPV4 => {
                self.scope_id == 0
                    && self.addr[4..] == [0; 12]
                    && is_unicast_ipv4(field(&self.addr, 0))
            }
            FAMILY_IPV6 => is_unicast_ipv6(self.addr),
            _ => false,
        }
    }
}

pub fn encode_dns_records(records: &[DnsRecord], out: &mut [u8]) -> Result<usize, WireError> {
    if records.is_empty() || records.len() > MAX_DNS_RECORDS {
        return Err(WireError::Invalid);
    }
    let total = DNS_RECORDS_HEADER_LEN + records.len() * DNS_RECORD_LEN;
    let buf = out.get_mut(..total).ok_or(WireError::BufferTooSmall)?;
    buf[0] = records.len() as u8;
    buf[1] = 0;
    for (index, rec) in records.iter().enumerate() {
        if !rec.is_valid() {
            return Err(WireError::Invalid);
        }
        let off = DNS_RECORDS_HEADER_LEN + index * DNS_RECORD_LEN;
        buf[off] = rec.family;
        buf[off + 1] = 0;
        buf[off + 2..off + 6].copy_from_slice(&rec.scope_id.to_le_bytes());
        buf[off + 6..off + 22].copy_from_slice(&rec.addr);
    }
    Ok(total)
}

pub fn decode_dns_records(bytes: &[u8], out: &mut [DnsRecord]) -> Result<usize, WireError> {
    if bytes.len() < DNS_RECORDS_HEADER_LEN || bytes[1] != 0 {
        return Err(WireError::Invalid);
    }
    let count = usize::from(bytes[0]);
    if count == 0
        || count > MAX_DNS_RECORDS
        || bytes.len() != DNS_RECORDS_HEADER_LEN + count * DNS_RECORD_LEN
    {
        return Err(WireError::Invalid);
    }
    if out.len() < count {
        return Err(WireError::BufferTooSmall);
    }
    for (index, slot) in out[..count].iter_mut().enumerate() {
        let off = DNS_RECORDS_HEADER_LEN + index * DNS_RECORD_LEN;
        if bytes[off + 1] != 0 {
            return Err(WireError::Invalid);
        }
        let rec = DnsRecord {
            family: bytes[off],
            scope_id: u32::from_le_bytes(field(bytes, off + 2)),
            addr: field(bytes, off + 6),
        };
        if !rec.is_valid() {
            return Err(WireError::Invalid);
        }
        *slot = rec;
    }
    Ok(count)
}

#[cfg(test)]
mod tests;
