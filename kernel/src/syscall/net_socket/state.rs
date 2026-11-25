//! Fixed-size, generation-bound socket proxy state. Neither packet policy nor
//! user memory access is performed while the bridge is locked.
use super::*;

const SOCKETS: usize = 16;
const SOCKETS_PER_OWNER: usize = 8;
pub(super) const MAX_UDP: usize = 1472;
pub(super) const READ: u16 = 1;
pub(super) const WRITE: u16 = 2;
pub(super) const HANGUP: u16 = 4;
pub(super) const ERROR: u16 = 8;

pub(super) struct ReadPlan {
    bytes: [u8; MAX_FRAME_V2],
    len: usize,
    source: Source,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Socket {
        token: u64,
        consumed: usize,
    },
    Service {
        generation: u64,
        token: u64,
        kind: u16,
    },
}

impl ReadPlan {
    fn new(source: Source) -> Self {
        Self {
            bytes: [0; MAX_FRAME_V2],
            len: 0,
            source,
        }
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

struct Request {
    kind: u16,
    endpoint: Endpoint6,
    bytes: [u8; MAX_PAYLOAD],
    len: usize,
    sent: bool,
}

impl Request {
    fn new(kind: u16, endpoint: Endpoint6, payload: &[u8]) -> Self {
        let mut bytes = [0; MAX_PAYLOAD];
        bytes[..payload.len()].copy_from_slice(payload);
        Self {
            kind,
            endpoint,
            bytes,
            len: payload.len(),
            sent: false,
        }
    }
}

struct Receive {
    endpoint: Endpoint6,
    bytes: [u8; MAX_PAYLOAD],
    start: usize,
    len: usize,
}

pub(super) enum AcceptStep6 {
    Ready { child_token: u64, remote: Endpoint6 },
    Pending,
    Empty,
}

struct Socket {
    token: u64,
    owner: u64,
    protocol: u16,
    scope: u32,
    version: u16,
    open: bool,
    bound: bool,
    bind_target: Option<Endpoint6>,
    listening: bool,
    listen_reported: bool,
    accept_ready: bool,
    pending_accept: Option<u64>,
    accepted: Option<(u64, Endpoint6)>,
    accepted_pending: bool,
    local: Endpoint6,
    remote: Endpoint6,
    error: i64,
    eof: bool,
    write_closed: bool,
    closing: bool,
    request: Option<Request>,
    rx: Option<Receive>,
    rx_reserved: bool,
    credit: bool,
}

impl Socket {
    fn new(token: u64, owner: u64, protocol: u16, scope: u32, version: u16) -> Self {
        Self {
            token,
            owner,
            protocol,
            scope,
            version,
            open: false,
            bound: false,
            bind_target: None,
            listening: false,
            listen_reported: false,
            accept_ready: false,
            pending_accept: None,
            accepted: None,
            accepted_pending: false,
            local: Endpoint6::NONE,
            remote: Endpoint6::NONE,
            error: 0,
            eof: false,
            write_closed: false,
            closing: false,
            request: Some(Request::new(BRIDGE_OPEN, Endpoint6::NONE, &[])),
            rx: None,
            rx_reserved: false,
            credit: false,
        }
    }

    fn new_accepted(token: u64, owner: u64, scope: u32, version: u16) -> Self {
        Self {
            request: None,
            accepted_pending: true,
            ..Self::new(token, owner, PROTO_TCP, scope, version)
        }
    }

    fn header(&self, kind: u16, endpoint: Endpoint6) -> BridgeHeader6 {
        BridgeHeader6 {
            token: self.token,
            owner: self.owner,
            kind,
            flags: 0,
            error: 0,
            endpoint,
            protocol: self.protocol,
            length: 0,
        }
    }

    fn next_request(&self) -> Option<(BridgeHeader6, &[u8])> {
        if self.closing {
            Some((self.header(BRIDGE_CLOSE, Endpoint6::NONE), &[]))
        } else if self.credit {
            Some((self.header(BRIDGE_CREDIT, Endpoint6::NONE), &[]))
        } else if let Some(req) = self.request.as_ref().filter(|request| !request.sent) {
            Some((self.header(req.kind, req.endpoint), &req.bytes[..req.len]))
        } else {
            None
        }
    }

    fn client_ready(&self) -> u16 {
        if self.error != 0 {
            return ERROR | HANGUP | u16::from(!self.rx_reserved && self.rx.is_some()) * READ;
        }
        if self.listening {
            let read = self.accept_ready || self.accepted.is_some();
            let write = !self.listen_reported;
            return u16::from(read) * READ | u16::from(write) * WRITE;
        }
        let read = !self.rx_reserved && (self.rx.is_some() || self.eof);
        let write = self.open && self.request.is_none() && !self.write_closed;
        u16::from(read) * READ
            | u16::from(write) * WRITE
            | u16::from(self.eof && self.write_closed) * HANGUP
    }

    fn check_request(&self) -> Result<(), i64> {
        if self.error != 0 {
            Err(self.error)
        } else if !self.open || self.request.is_some() {
            Err(EAGAIN)
        } else {
            Ok(())
        }
    }

    fn finish(&mut self, header: BridgeHeader6) -> Result<(), i64> {
        let pending = self
            .request
            .as_ref()
            .filter(|value| value.sent)
            .ok_or(EINVAL)?;
        if header.flags != pending.kind {
            return Err(EINVAL);
        }
        let valid_local = header.endpoint.is_unicast()
            || (self.version == VERSION_V2
                && pending.kind == BRIDGE_BIND
                && header.endpoint.is_bind_address()
                && header.endpoint.port != 0);
        if !header.endpoint.is_none()
            && (!valid_local || (!self.local.is_none() && self.local != header.endpoint))
        {
            return Err(EINVAL);
        }
        if pending.kind == BRIDGE_BIND && header.error == 0 && !valid_local {
            return Err(EINVAL);
        }
        if header.error != 0 {
            self.error = i64::from(header.error);
        } else {
            match pending.kind {
                BRIDGE_OPEN => self.open = true,
                BRIDGE_CONNECT => self.remote = pending.endpoint,
                BRIDGE_BIND => self.bound = true,
                BRIDGE_LISTEN => self.listening = true,
                BRIDGE_SEND | BRIDGE_SHUTDOWN => {}
                _ => return Err(EINVAL),
            }
            if !header.endpoint.is_none() {
                self.local = header.endpoint;
            }
        }
        self.request = None;
        Ok(())
    }
}

struct Service {
    owner: u64,
    generation: u64,
    version: u16,
    cursor: usize,
    read_reserved: bool,
}

pub(super) struct Bridge {
    service: Option<Service>,
    token: u64,
    sockets: [Option<Socket>; SOCKETS],
}

impl Bridge {
    pub(super) const fn new() -> Self {
        Self {
            service: None,
            token: 0,
            sockets: [const { None }; SOCKETS],
        }
    }

    fn next_token(&mut self) -> Result<u64, i64> {
        self.token = self.token.checked_add(1).ok_or(ENFILE)?;
        Ok(self.token)
    }

    pub(super) fn register_version(&mut self, owner: u64, version: u16) -> Result<u64, i64> {
        if self.service.is_some() {
            return Err(EBUSY);
        }
        let generation = self.next_token()?;
        self.service = Some(Service {
            owner,
            generation,
            version,
            cursor: 0,
            read_reserved: false,
        });
        Ok(generation)
    }

    fn reserve_slot(&self, owner: u64) -> Result<usize, i64> {
        if self.service.is_none() {
            return Err(EPIPE);
        }
        if self
            .sockets
            .iter()
            .flatten()
            .filter(|socket| socket.owner == owner)
            .count()
            >= SOCKETS_PER_OWNER
        {
            return Err(ENFILE);
        }
        self.sockets.iter().position(Option::is_none).ok_or(ENFILE)
    }

    pub(super) fn open_version(
        &mut self,
        owner: u64,
        protocol: u16,
        scope: u32,
        version: u16,
    ) -> Result<u64, i64> {
        let slot = self.reserve_slot(owner)?;
        let token = self.next_token()?;
        self.sockets[slot] = Some(Socket::new(token, owner, protocol, scope, version));
        Ok(token)
    }

    pub(super) fn service_version(&self, owner: u64, token: u64) -> Result<u16, i64> {
        Ok(self.service(owner, token)?.version)
    }

    pub(super) fn socket_version(&self, owner: u64, token: u64) -> Result<u16, i64> {
        Ok(self.client(owner, token)?.version)
    }

    fn check_endpoint_for_service(&self, endpoint: Endpoint6) -> Result<(), i64> {
        let service = self.service.as_ref().ok_or(EPIPE)?;
        if service.version == VERSION && endpoint.family == FAMILY_IPV6 {
            return Err(EOPNOTSUPP);
        }
        Ok(())
    }

    fn socket_index(&self, token: u64) -> Option<usize> {
        self.sockets
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|s| s.token == token))
    }

    fn client(&self, owner: u64, token: u64) -> Result<&Socket, i64> {
        let socket = self.sockets[self.socket_index(token).ok_or(EPIPE)?]
            .as_ref()
            .ok_or(EPIPE)?;
        if socket.owner != owner || socket.closing || socket.accepted_pending {
            return Err(EPERM);
        }
        Ok(socket)
    }

    fn client_mut(&mut self, owner: u64, token: u64) -> Result<&mut Socket, i64> {
        let index = self.socket_index(token).ok_or(EPIPE)?;
        let socket = self.sockets[index].as_mut().ok_or(EPIPE)?;
        if socket.owner != owner || socket.closing || socket.accepted_pending {
            return Err(EPERM);
        }
        Ok(socket)
    }

    fn service(&self, owner: u64, token: u64) -> Result<&Service, i64> {
        let service = self
            .service
            .as_ref()
            .filter(|s| s.generation == token)
            .ok_or(EPIPE)?;
        if service.owner != owner {
            return Err(EPERM);
        }
        Ok(service)
    }

    pub(super) fn ready(&self, owner: u64, target: FdTarget) -> u16 {
        match target {
            FdTarget::NetSocket { token } => self.client(owner, token).map(Socket::client_ready),
            FdTarget::NetService { token } => self.service(owner, token).map(|service| {
                let readable = !service.read_reserved
                    && self
                        .sockets
                        .iter()
                        .flatten()
                        .any(|socket| socket.next_request().is_some());
                WRITE | u16::from(readable) * READ
            }),
            _ => Err(EBADF),
        }
        .unwrap_or(ERROR | HANGUP)
    }

    pub(super) fn close(&mut self, target: FdTarget) -> bool {
        match target {
            FdTarget::NetService { token }
                if self.service.as_ref().is_some_and(|s| s.generation == token) =>
            {
                self.service = None;
                self.sockets.fill_with(|| None);
                true
            }
            FdTarget::NetSocket { token } => {
                let Some(index) = self.socket_index(token) else {
                    return false;
                };
                let (pending, unsent, accepted) = {
                    let socket = self.sockets[index].as_mut().expect("known socket");
                    if socket.closing {
                        return false;
                    }
                    let unsent = socket
                        .request
                        .as_ref()
                        .is_some_and(|req| req.kind == BRIDGE_ACCEPT && !req.sent);
                    socket.closing = true;
                    socket.request = None;
                    socket.rx = None;
                    socket.credit = false;
                    socket.accept_ready = false;
                    (socket.pending_accept.take(), unsent, socket.accepted.take())
                };
                if let Some(child_token) = pending {
                    if let Some(child_idx) = self.socket_index(child_token) {
                        if unsent {
                            self.sockets[child_idx] = None;
                        } else if let Some(child) = self.sockets[child_idx].as_mut() {
                            child.accepted_pending = false;
                            child.closing = true;
                            child.rx = None;
                            child.credit = false;
                        }
                    }
                }
                if let Some((child_token, _)) = accepted {
                    if let Some(child) = self
                        .socket_index(child_token)
                        .and_then(|idx| self.sockets[idx].as_mut())
                    {
                        child.accepted_pending = false;
                        child.closing = true;
                        child.rx = None;
                        child.credit = false;
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub(super) fn reserve_service(
        &mut self,
        owner: u64,
        generation: u64,
        capacity: usize,
    ) -> Result<ReadPlan, i64> {
        let service_version = self.service(owner, generation)?.version;
        if self.service(owner, generation)?.read_reserved {
            return Err(EAGAIN);
        }
        let hdr_len = if service_version == VERSION_V2 {
            BRIDGE_HEADER6_LEN
        } else {
            BRIDGE_HEADER_LEN
        };
        let start = self.service(owner, generation)?.cursor;
        for offset in 0..SOCKETS {
            let index = (start + offset) % SOCKETS;
            let Some(socket) = self.sockets[index].as_ref() else {
                continue;
            };
            let Some((header, payload)) = socket.next_request() else {
                continue;
            };
            if hdr_len + payload.len() > capacity {
                return Err(EMSGSIZE);
            }
            let mut plan = ReadPlan::new(Source::Service {
                generation,
                token: socket.token,
                kind: header.kind,
            });
            plan.len = if service_version == VERSION_V2 {
                header.encode(payload, &mut plan.bytes).map_err(wire_error)?
            } else {
                header
                    .to_v1()
                    .and_then(|v1| v1.encode(payload, &mut plan.bytes))
                    .map_err(wire_error)?
            };
            self.service
                .as_mut()
                .expect("validated service")
                .read_reserved = true;
            return Ok(plan);
        }
        Err(EAGAIN)
    }

    pub(super) fn reserve_client(
        &mut self,
        owner: u64,
        token: u64,
        capacity: usize,
        from: bool,
    ) -> Result<Option<ReadPlan>, i64> {
        let socket = self.client_mut(owner, token)?;
        if socket.listening {
            return Err(EOPNOTSUPP);
        }
        if from && socket.protocol != PROTO_UDP {
            return Err(EOPNOTSUPP);
        }
        let v2 = socket.version == VERSION_V2;
        let prefix = if from {
            if v2 {
                RECV_HEADER6_LEN
            } else {
                RECV_HEADER_LEN
            }
        } else {
            0
        };
        if from && capacity < prefix {
            return Err(EMSGSIZE);
        }
        if socket.rx_reserved {
            return Err(EAGAIN);
        }
        let Some(rx) = socket.rx.as_ref() else {
            return if socket.error != 0 {
                Err(socket.error)
            } else if socket.eof {
                Ok(None)
            } else {
                Err(EAGAIN)
            };
        };
        let count = (rx.len - rx.start).min(capacity - prefix);
        let consumed = if socket.protocol == PROTO_UDP {
            rx.len
        } else {
            count
        };
        let mut plan = ReadPlan::new(Source::Socket { token, consumed });
        if from {
            if v2 {
                plan.bytes[..prefix].copy_from_slice(
                    &RecvHeader6 {
                        endpoint: rx.endpoint,
                        full_len: rx.len as u32,
                    }
                    .encode(),
                );
            } else {
                let endpoint = rx.endpoint.to_v4().ok_or(EINVAL)?;
                plan.bytes[..prefix].copy_from_slice(
                    &RecvHeader {
                        endpoint,
                        full_len: rx.len as u32,
                    }
                    .encode(),
                );
            }
        }
        plan.bytes[prefix..prefix + count].copy_from_slice(&rx.bytes[rx.start..rx.start + count]);
        plan.len = prefix + count;
        socket.rx_reserved = true;
        Ok(Some(plan))
    }

    pub(super) fn finish_read(&mut self, plan: &ReadPlan, consume: bool) -> Result<(), i64> {
        match plan.source {
            Source::Socket { token, consumed } => {
                let index = self.socket_index(token).ok_or(EPIPE)?;
                let socket = self.sockets[index]
                    .as_mut()
                    .filter(|s| !s.closing)
                    .ok_or(EPIPE)?;
                socket.rx_reserved = false;
                if consume {
                    let rx = socket.rx.as_mut().ok_or(EPIPE)?;
                    rx.start += consumed;
                    if rx.start == rx.len {
                        socket.rx = None;
                        socket.credit = true;
                    }
                }
            }
            Source::Service {
                generation,
                token,
                kind,
            } => {
                let service = self
                    .service
                    .as_mut()
                    .filter(|s| s.generation == generation)
                    .ok_or(EPIPE)?;
                service.read_reserved = false;
                if !consume {
                    return Ok(());
                }
                let index = self.socket_index(token).ok_or(EPIPE)?;
                let socket = self.sockets[index].as_mut().expect("known socket");
                match kind {
                    BRIDGE_CLOSE => self.sockets[index] = None,
                    BRIDGE_CREDIT => socket.credit = false,
                    _ => {
                        // Final alias close may have cancelled the request
                        // while the copyout was pinned. Delivery is followed
                        // by CLOSE before the id can be reused.
                        if let Some(request) = &mut socket.request {
                            request.sent = true;
                        }
                    }
                }
                self.service.as_mut().expect("validated service").cursor = (index + 1) % SOCKETS;
            }
        }
        Ok(())
    }

    pub(super) fn service_write6(
        &mut self,
        owner: u64,
        generation: u64,
        header: BridgeHeader6,
        data: &[u8],
    ) -> Result<(), i64> {
        self.service(owner, generation)?;
        let index = self.socket_index(header.token).ok_or(ENOENT)?;
        let socket = self.sockets[index]
            .as_mut()
            .filter(|s| !s.closing)
            .ok_or(ENOENT)?;
        if socket.owner != header.owner || socket.protocol != header.protocol {
            return Err(EPERM);
        }
        match header.kind {
            BRIDGE_COMPLETE if data.is_empty() && header.error <= 0 && header.error >= -4095 => {
                if header.error != 0 && !header.endpoint.is_none() {
                    return Err(EINVAL);
                }
                if header.flags == BRIDGE_ACCEPT {
                    return self.complete_accept(index, header);
                }
                socket.finish(header)
            }
            BRIDGE_LISTENER_READY
                if socket.protocol == PROTO_TCP
                    && socket.listening
                    && socket.error == 0
                    && header.flags <= 1
                    && header.error == 0
                    && header.endpoint.is_none()
                    && data.is_empty() =>
            {
                socket.accept_ready = header.flags == 1;
                Ok(())
            }
            BRIDGE_DATA if header.flags == 0 && header.error == 0 => {
                if !socket.open || socket.eof || socket.error != 0 {
                    return Err(EPIPE);
                }
                let mut dns_recs = [DnsRecord::default(); MAX_DNS_RECORDS];
                let dns_count = if socket.protocol == PROTO_DNS && data.len() != 4 {
                    Some(decode_dns_records(data, &mut dns_recs).map_err(wire_error)?)
                } else {
                    None
                };
                let valid = match socket.protocol {
                    PROTO_UDP => header.endpoint.is_unicast() && data.len() <= MAX_UDP,
                    PROTO_DNS => header.endpoint.is_none() && (data.len() == 4 || dns_count.is_some()),
                    _ => {
                        header.endpoint.is_none() && !socket.remote.is_none() && !data.is_empty()
                    }
                };
                if !valid {
                    return Err(EINVAL);
                }
                if socket.rx.is_some() || socket.credit {
                    return Err(EAGAIN);
                }
                let mut bytes = [0; MAX_PAYLOAD];
                let len = if socket.protocol == PROTO_DNS {
                    if socket.version == VERSION {
                        if let Some(count) = dns_count {
                            let Some(v4) = dns_recs[..count]
                                .iter()
                                .find(|r| r.family == FAMILY_IPV4)
                            else {
                                socket.error = ENOENT;
                                return Ok(());
                            };
                            bytes[..4].copy_from_slice(&v4.addr[..4]);
                            4
                        } else {
                            bytes[..4].copy_from_slice(data);
                            4
                        }
                    } else if dns_count.is_some() {
                        bytes[..data.len()].copy_from_slice(data);
                        data.len()
                    } else {
                        let rec = DnsRecord::ipv4(data.try_into().map_err(|_| EINVAL)?);
                        encode_dns_records(&[rec], &mut bytes).map_err(wire_error)?
                    }
                } else {
                    bytes[..data.len()].copy_from_slice(data);
                    data.len()
                };
                socket.rx = Some(Receive {
                    endpoint: header.endpoint,
                    bytes,
                    start: 0,
                    len,
                });
                Ok(())
            }
            BRIDGE_EOF
                if socket.protocol == PROTO_TCP
                    && header.flags == 0
                    && header.error == 0
                    && header.endpoint.is_none()
                    && data.is_empty() =>
            {
                if socket.remote.is_none() || socket.error != 0 {
                    return Err(EINVAL);
                }
                socket.eof = true;
                Ok(())
            }
            BRIDGE_ERROR
                if header.flags == 0
                    && header.error < 0
                    && header.error >= -4095
                    && header.endpoint.is_none()
                    && data.is_empty() =>
            {
                socket.error = i64::from(header.error);
                socket.request = None;
                Ok(())
            }
            _ => Err(EINVAL),
        }
    }

    fn complete_accept(&mut self, index: usize, header: BridgeHeader6) -> Result<(), i64> {
        let (child_token, local) = {
            let socket = self.sockets[index].as_mut().expect("known socket");
            let valid = socket
                .request
                .as_ref()
                .is_some_and(|req| req.kind == BRIDGE_ACCEPT && req.sent);
            if !valid {
                return Err(EINVAL);
            }
            let Some(child_token) = socket.pending_accept else {
                return Err(EINVAL);
            };
            if header.error == 0 && !header.endpoint.is_unicast() {
                return Err(EINVAL);
            }
            socket.pending_accept = None;
            socket.request = None;
            socket.accept_ready = false;
            if header.error != 0 && i64::from(header.error) != EAGAIN {
                socket.error = i64::from(header.error);
            }
            (child_token, socket.local)
        };
        if header.error != 0 {
            if let Some(child_idx) = self.socket_index(child_token) {
                if self.sockets[child_idx].as_ref().is_some_and(|c| !c.closing) {
                    self.sockets[child_idx] = None;
                }
            }
            return Ok(());
        }
        let child_alive = self
            .socket_index(child_token)
            .and_then(|idx| self.sockets[idx].as_mut())
            .filter(|child| !child.closing);
        if let Some(child) = child_alive {
            child.open = true;
            child.local = local;
            child.remote = header.endpoint;
            if let Some(listener) = self.sockets[index].as_mut() {
                listener.accepted = Some((child_token, header.endpoint));
            }
        }
        Ok(())
    }

    pub(super) fn protocol(&self, owner: u64, token: u64) -> Result<u16, i64> {
        Ok(self.client(owner, token)?.protocol)
    }

    pub(super) fn send6(
        &mut self,
        owner: u64,
        token: u64,
        endpoint: Endpoint6,
        payload: &[u8],
    ) -> Result<(), i64> {
        self.check_endpoint_for_service(endpoint)?;
        let socket = self.client_mut(owner, token)?;
        if socket.listening {
            return Err(EOPNOTSUPP);
        }
        socket.check_request()?;
        let max = match socket.protocol {
            PROTO_UDP => MAX_UDP,
            PROTO_DNS => MAX_DNS_NAME,
            _ => MAX_PAYLOAD,
        };
        if payload.len() > max {
            return Err(EMSGSIZE);
        }
        if socket.write_closed {
            return Err(EPIPE);
        }
        if socket.protocol == PROTO_DNS {
            if !endpoint.is_none() {
                return Err(EOPNOTSUPP);
            }
            if payload.is_empty() {
                return Err(EINVAL);
            }
        } else if endpoint.is_none() && socket.remote.is_none() {
            return Err(EINVAL);
        }
        if !endpoint.is_none() {
            if socket.protocol != PROTO_UDP {
                return Err(EOPNOTSUPP);
            }
            if !socket.remote.is_none() && socket.remote != endpoint {
                return Err(EINVAL);
            }
        }
        socket.request = Some(Request::new(BRIDGE_SEND, endpoint, payload));
        Ok(())
    }

    pub(super) fn connect6(
        &mut self,
        owner: u64,
        token: u64,
        endpoint: Endpoint6,
    ) -> Result<bool, i64> {
        self.check_endpoint_for_service(endpoint)?;
        let socket = self.client_mut(owner, token)?;
        if socket.protocol == PROTO_DNS {
            return Err(EOPNOTSUPP);
        }
        let needed = if socket.protocol == PROTO_TCP {
            SCOPE_TCP
        } else {
            SCOPE_UDP
        };
        if socket.scope & needed == 0 {
            return Err(EPERM);
        }
        if socket.listening || (socket.protocol == PROTO_TCP && socket.bound) {
            return Err(EOPNOTSUPP);
        }
        if socket.error != 0 {
            return Err(socket.error);
        }
        if socket.remote == endpoint {
            return Ok(true);
        }
        if !socket.remote.is_none() {
            return Err(EOPNOTSUPP);
        }
        socket.check_request()?;
        socket.request = Some(Request::new(BRIDGE_CONNECT, endpoint, &[]));
        Ok(false)
    }

    pub(super) fn shutdown(&mut self, owner: u64, token: u64) -> Result<bool, i64> {
        let socket = self.client_mut(owner, token)?;
        if socket.protocol != PROTO_TCP || socket.listening {
            return Err(EOPNOTSUPP);
        }
        if socket.error != 0 {
            return Err(socket.error);
        }
        if socket.write_closed {
            return Ok(false);
        }
        socket.check_request()?;
        if socket.remote.is_none() {
            return Err(EINVAL);
        }
        socket.write_closed = true;
        socket.request = Some(Request::new(BRIDGE_SHUTDOWN, Endpoint6::NONE, &[]));
        Ok(true)
    }

    pub(super) fn bind6(
        &mut self,
        owner: u64,
        token: u64,
        endpoint: Endpoint6,
    ) -> Result<bool, i64> {
        self.check_endpoint_for_service(endpoint)?;
        let socket = self.client_mut(owner, token)?;
        let needed = match socket.protocol {
            PROTO_UDP => SCOPE_UDP_BIND,
            PROTO_TCP => SCOPE_TCP_LISTEN,
            _ => return Err(EOPNOTSUPP),
        };
        if socket.scope & needed == 0 {
            return Err(EPERM);
        }
        if (1..1024).contains(&endpoint.port) && socket.scope & SCOPE_LOW_PORT == 0 {
            return Err(EACCES);
        }
        if socket.error != 0 {
            return Err(socket.error);
        }
        if socket.bound && socket.bind_target == Some(endpoint) {
            return Ok(true);
        }
        if socket.bound || socket.listening || !socket.remote.is_none() {
            return Err(EINVAL);
        }
        socket.check_request()?;
        socket.bind_target = Some(endpoint);
        socket.request = Some(Request::new(BRIDGE_BIND, endpoint, &[]));
        Ok(false)
    }

    pub(super) fn listen(&mut self, owner: u64, token: u64, backlog: u32) -> Result<bool, i64> {
        let socket = self.client_mut(owner, token)?;
        if socket.protocol != PROTO_TCP {
            return Err(EOPNOTSUPP);
        }
        if socket.scope & SCOPE_TCP_LISTEN == 0 {
            return Err(EPERM);
        }
        if socket.error != 0 {
            return Err(socket.error);
        }
        if socket.listening {
            socket.listen_reported = true;
            return Ok(true);
        }
        if !socket.bound || !socket.remote.is_none() {
            return Err(EINVAL);
        }
        socket.check_request()?;
        let clamped = backlog.clamp(1, 16);
        socket.request = Some(Request::new(
            BRIDGE_LISTEN,
            Endpoint6::NONE,
            &clamped.to_le_bytes(),
        ));
        Ok(false)
    }

    pub(super) fn accept_begin6(&mut self, owner: u64, token: u64) -> Result<AcceptStep6, i64> {
        let index = self.socket_index(token).ok_or(EPIPE)?;
        let (scope, version) = {
            let socket = self.sockets[index].as_mut().ok_or(EPIPE)?;
            if socket.owner != owner || socket.closing || socket.accepted_pending {
                return Err(EPERM);
            }
            if socket.protocol != PROTO_TCP {
                return Err(EOPNOTSUPP);
            }
            if socket.scope & SCOPE_TCP_LISTEN == 0 {
                return Err(EPERM);
            }
            if !socket.listening {
                return Err(EINVAL);
            }
            if socket.error != 0 {
                return Err(socket.error);
            }
            if let Some((child_token, remote)) = socket.accepted.take() {
                if let Some(child_idx) = self.socket_index(child_token) {
                    if let Some(child) = self.sockets[child_idx].as_mut() {
                        child.accepted_pending = false;
                    }
                }
                return Ok(AcceptStep6::Ready {
                    child_token,
                    remote,
                });
            }
            if socket
                .request
                .as_ref()
                .is_some_and(|req| req.kind == BRIDGE_ACCEPT)
            {
                return Ok(AcceptStep6::Pending);
            }
            socket.check_request()?;
            if !socket.accept_ready {
                return Ok(AcceptStep6::Empty);
            }
            (socket.scope, socket.version)
        };
        let child_slot = self.reserve_slot(owner)?;
        let child_token = self.next_token()?;
        self.sockets[child_slot] = Some(Socket::new_accepted(child_token, owner, scope, version));
        let socket = self.sockets[index].as_mut().unwrap();
        socket.pending_accept = Some(child_token);
        socket.request = Some(Request::new(
            BRIDGE_ACCEPT,
            Endpoint6::NONE,
            &child_token.to_le_bytes(),
        ));
        Ok(AcceptStep6::Pending)
    }

    pub(super) fn error(&self, owner: u64, token: u64) -> Result<i32, i64> {
        Ok(self.client(owner, token)?.error as i32)
    }

    pub(super) fn endpoints6(&self, owner: u64, token: u64) -> Result<Endpoints6, i64> {
        let socket = self.client(owner, token)?;
        Ok(Endpoints6 {
            local: socket.local,
            remote: socket.remote,
        })
    }
}

#[cfg(test)]
impl Bridge {
    fn register(&mut self, owner: u64) -> Result<u64, i64> {
        self.register_version(owner, VERSION)
    }

    fn open(&mut self, owner: u64, protocol: u16, scope: u32) -> Result<u64, i64> {
        self.open_version(owner, protocol, scope, VERSION)
    }

    fn service_write(
        &mut self,
        owner: u64,
        generation: u64,
        header: BridgeHeader,
        data: &[u8],
    ) -> Result<(), i64> {
        self.service_write6(owner, generation, BridgeHeader6::from_v1(header), data)
    }

    fn send(&mut self, owner: u64, token: u64, endpoint: Endpoint, payload: &[u8]) -> Result<(), i64> {
        self.send6(owner, token, Endpoint6::from_v4(endpoint), payload)
    }

    fn connect(&mut self, owner: u64, token: u64, endpoint: Endpoint) -> Result<bool, i64> {
        self.connect6(owner, token, Endpoint6::from_v4(endpoint))
    }

    fn bind(&mut self, owner: u64, token: u64, endpoint: Endpoint) -> Result<bool, i64> {
        self.bind6(owner, token, Endpoint6::bind_from_v4(endpoint))
    }

    fn accept_begin(&mut self, owner: u64, token: u64) -> Result<AcceptStep, i64> {
        match self.accept_begin6(owner, token)? {
            AcceptStep6::Ready {
                child_token,
                remote,
            } => Ok(AcceptStep::Ready {
                child_token,
                remote: remote.to_v4().ok_or(EINVAL)?,
            }),
            AcceptStep6::Pending => Ok(AcceptStep::Pending),
            AcceptStep6::Empty => Ok(AcceptStep::Empty),
        }
    }

    fn endpoints(&self, owner: u64, token: u64) -> Result<Endpoints, i64> {
        let eps = self.endpoints6(owner, token)?;
        Ok(Endpoints {
            local: eps.local.to_v4().ok_or(EINVAL)?,
            remote: eps.remote.to_v4().ok_or(EINVAL)?,
        })
    }
}

#[cfg(test)]
enum AcceptStep {
    Ready { child_token: u64, remote: Endpoint },
    Pending,
    Empty,
}

#[cfg(test)]
const NO_ENDPOINT: Endpoint = Endpoint::NONE;

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE: u64 = 10;
    const APP: u64 = 11;
    const OTHER: u64 = 12;
    const LOCAL: Endpoint = Endpoint {
        ip: [192, 0, 2, 15],
        port: 49152,
    };
    const REMOTE: Endpoint = Endpoint {
        ip: [192, 0, 2, 2],
        port: 8080,
    };

    fn request(bridge: &mut Bridge, generation: u64) -> (BridgeHeader, Vec<u8>) {
        let plan = bridge
            .reserve_service(SERVICE, generation, MAX_FRAME)
            .unwrap();
        let (header, data) = BridgeHeader::decode(plan.bytes()).unwrap();
        let payload = data.to_vec();
        bridge.finish_read(&plan, true).unwrap();
        (header, payload)
    }

    fn response(mut request: BridgeHeader, kind: u16, endpoint: Endpoint) -> BridgeHeader {
        request.flags = if kind == BRIDGE_COMPLETE {
            request.kind
        } else {
            0
        };
        request.kind = kind;
        request.endpoint = endpoint;
        request
    }

    #[test]
    fn checks_generation_owner_and_per_process_limit() {
        let mut bridge = Bridge::new();
        assert_eq!(bridge.open(APP, PROTO_TCP, SCOPE_TCP), Err(EPIPE));
        let generation = bridge.register(SERVICE).unwrap();
        assert_eq!(bridge.register(OTHER), Err(EBUSY));
        let socket = bridge.open(APP, PROTO_TCP, SCOPE_TCP).unwrap();
        for _ in 1..SOCKETS_PER_OWNER {
            bridge.open(APP, PROTO_UDP, SCOPE_UDP).unwrap();
        }
        assert_eq!(bridge.open(APP, PROTO_TCP, SCOPE_TCP), Err(ENFILE));
        assert!(bridge.open(OTHER, PROTO_TCP, SCOPE_TCP).is_ok());
        assert_eq!(bridge.protocol(OTHER, socket), Err(EPERM));
        assert!(matches!(
            bridge.reserve_service(OTHER, generation, MAX_FRAME),
            Err(EPERM)
        ));
        let (open, _) = request(&mut bridge, generation);
        let mut completion = response(open, BRIDGE_COMPLETE, NO_ENDPOINT);
        completion.owner = OTHER;
        assert_eq!(
            bridge.service_write(SERVICE, generation, completion, &[]),
            Err(EPERM)
        );
        assert!(bridge.close(FdTarget::NetService { token: generation }));
        assert_eq!(bridge.protocol(APP, socket), Err(EPIPE));
        let fresh = bridge.register(SERVICE).unwrap();
        let fresh_socket = bridge.open(APP, PROTO_TCP, SCOPE_TCP).unwrap();
        assert_ne!(fresh_socket, socket);
        assert_eq!(
            bridge.service_write(SERVICE, generation, completion, &[]),
            Err(EPIPE)
        );
        assert_eq!(
            bridge.service_write(SERVICE, fresh, completion, &[]),
            Err(ENOENT)
        );
    }

    #[test]
    fn retain_faulted_records_and_reject_conflicting_completion_before_mutating() {
        let mut bridge = Bridge::new();
        let generation = bridge.register(SERVICE).unwrap();
        let token = bridge.open(APP, PROTO_TCP, SCOPE_TCP).unwrap();
        let target = FdTarget::NetSocket { token };
        assert_eq!(bridge.ready(APP, target), 0);
        assert!(matches!(
            bridge.reserve_service(SERVICE, generation, 5),
            Err(EMSGSIZE)
        ));
        let plan = bridge
            .reserve_service(SERVICE, generation, MAX_FRAME)
            .unwrap();
        assert!(matches!(
            bridge.reserve_service(SERVICE, generation, MAX_FRAME),
            Err(EAGAIN)
        ));
        let (open, _) = BridgeHeader::decode(plan.bytes()).unwrap();
        assert_eq!(open.kind, BRIDGE_OPEN);
        bridge.finish_read(&plan, false).unwrap();
        assert_eq!(request(&mut bridge, generation).0, open);
        bridge
            .service_write(
                SERVICE,
                generation,
                response(open, BRIDGE_COMPLETE, LOCAL),
                &[],
            )
            .unwrap();
        assert_eq!(bridge.ready(APP, target), WRITE);
        assert_eq!(bridge.error(APP, token), Ok(0));
        assert_eq!(bridge.connect(APP, token, REMOTE), Ok(false));
        assert_eq!(bridge.connect(APP, token, REMOTE), Err(EAGAIN));
        let (connect, _) = request(&mut bridge, generation);
        assert_eq!(connect.kind, BRIDGE_CONNECT);
        let error = bridge.service_write(
            SERVICE,
            generation,
            response(connect, BRIDGE_COMPLETE, REMOTE),
            &[],
        );
        assert_eq!(error, Err(EINVAL));
        assert_eq!(bridge.endpoints(APP, token).unwrap().remote, NO_ENDPOINT);
        assert_eq!(bridge.ready(APP, target), 0);
        bridge
            .service_write(
                SERVICE,
                generation,
                response(connect, BRIDGE_COMPLETE, LOCAL),
                &[],
            )
            .unwrap();
        assert_eq!(bridge.connect(APP, token, REMOTE), Ok(true));

        let data = response(connect, BRIDGE_DATA, NO_ENDPOINT);
        bridge
            .service_write(SERVICE, generation, data, b"abcdef")
            .unwrap();
        let first = bridge
            .reserve_client(APP, token, 2, false)
            .unwrap()
            .unwrap();
        assert_eq!(first.bytes(), b"ab");
        bridge.finish_read(&first, false).unwrap();
        let retry = bridge
            .reserve_client(APP, token, 2, false)
            .unwrap()
            .unwrap();
        assert_eq!(retry.bytes(), b"ab");
        bridge.finish_read(&retry, true).unwrap();
        assert_eq!(
            bridge.service_write(SERVICE, generation, data, b"overflow"),
            Err(EAGAIN)
        );
        bridge
            .service_write(
                SERVICE,
                generation,
                response(connect, BRIDGE_EOF, NO_ENDPOINT),
                &[],
            )
            .unwrap();
        let tail = bridge
            .reserve_client(APP, token, 10, false)
            .unwrap()
            .unwrap();
        assert_eq!(tail.bytes(), b"cdef");
        bridge.finish_read(&tail, true).unwrap();
        assert!(bridge
            .reserve_client(APP, token, 10, false)
            .unwrap()
            .is_none());
        assert_eq!(request(&mut bridge, generation).0.kind, BRIDGE_CREDIT);
        assert_eq!(bridge.ready(APP, target), READ | WRITE);
        assert_eq!(bridge.shutdown(APP, token), Ok(true));
        assert_eq!(bridge.ready(APP, target), READ | HANGUP);
        assert_eq!(request(&mut bridge, generation).0.kind, BRIDGE_SHUTDOWN);
    }

    #[test]
    fn udp_truncation_preserves_source_and_close_retires_slot() {
        let mut bridge = Bridge::new();
        let generation = bridge.register(SERVICE).unwrap();
        let token = bridge.open(APP, PROTO_UDP, SCOPE_UDP).unwrap();
        let (open, _) = request(&mut bridge, generation);
        bridge
            .service_write(
                SERVICE,
                generation,
                response(open, BRIDGE_COMPLETE, NO_ENDPOINT),
                &[],
            )
            .unwrap();
        bridge.send(APP, token, REMOTE, b"out").unwrap();
        assert_eq!(bridge.send(APP, token, REMOTE, b"x"), Err(EAGAIN));
        let (send, payload) = request(&mut bridge, generation);
        assert_eq!(
            (send.kind, send.endpoint, payload.as_slice()),
            (BRIDGE_SEND, REMOTE, b"out".as_slice())
        );
        bridge
            .service_write(
                SERVICE,
                generation,
                response(send, BRIDGE_COMPLETE, LOCAL),
                &[],
            )
            .unwrap();
        bridge
            .service_write(
                SERVICE,
                generation,
                response(send, BRIDGE_DATA, REMOTE),
                b"abcdef",
            )
            .unwrap();
        let short = bridge
            .reserve_client(APP, token, RECV_HEADER_LEN + 2, true)
            .unwrap()
            .unwrap();
        let bytes = short.bytes();
        let address = RecvHeader::decode(&bytes[..RECV_HEADER_LEN]).unwrap();
        assert_eq!(
            (
                address.endpoint,
                address.full_len,
                &bytes[RECV_HEADER_LEN..]
            ),
            (REMOTE, 6, b"ab".as_slice())
        );
        bridge.finish_read(&short, true).unwrap();
        assert!(matches!(
            bridge.reserve_client(APP, token, 128, true),
            Err(EAGAIN)
        ));
        assert_eq!(request(&mut bridge, generation).0.kind, BRIDGE_CREDIT);
        assert!(bridge.close(FdTarget::NetSocket { token }));
        assert_eq!(request(&mut bridge, generation).0.kind, BRIDGE_CLOSE);
        assert_eq!(bridge.protocol(APP, token), Err(EPIPE));
    }

    #[test]
    fn dns_sockets_enforce_bounds_and_reject_stream_ops() {
        let mut bridge = Bridge::new();
        let generation = bridge.register(SERVICE).unwrap();
        let token = bridge.open(APP, PROTO_DNS, SCOPE_UDP).unwrap();
        let (open, _) = request(&mut bridge, generation);
        assert_eq!(open.protocol, PROTO_DNS);
        bridge
            .service_write(
                SERVICE,
                generation,
                response(open, BRIDGE_COMPLETE, NO_ENDPOINT),
                &[],
            )
            .unwrap();
        assert_eq!(bridge.connect(APP, token, REMOTE), Err(EOPNOTSUPP));
        assert_eq!(bridge.shutdown(APP, token), Err(EOPNOTSUPP));
        assert_eq!(bridge.send(APP, token, REMOTE, b"a"), Err(EOPNOTSUPP));
        assert_eq!(bridge.send(APP, token, NO_ENDPOINT, b""), Err(EINVAL));
        assert_eq!(
            bridge.send(APP, token, NO_ENDPOINT, &[b'a'; MAX_DNS_NAME + 1]),
            Err(EMSGSIZE)
        );
        bridge.send(APP, token, NO_ENDPOINT, b"du.net.test").unwrap();
        let (query, name) = request(&mut bridge, generation);
        assert_eq!(name.as_slice(), b"du.net.test");
        let mut bad_reply = response(query, BRIDGE_DATA, NO_ENDPOINT);
        bad_reply.protocol = PROTO_DNS;
        assert_eq!(
            bridge.service_write(SERVICE, generation, bad_reply, &[1, 2, 3]),
            Err(EINVAL)
        );
        bridge
            .service_write(SERVICE, generation, bad_reply, &[192, 0, 2, 10])
            .unwrap();
        bridge
            .service_write(
                SERVICE,
                generation,
                response(query, BRIDGE_COMPLETE, NO_ENDPOINT),
                &[],
            )
            .unwrap();
        let plan = bridge.reserve_client(APP, token, 16, false).unwrap().unwrap();
        assert_eq!(plan.bytes(), &[192, 0, 2, 10]);
        bridge.finish_read(&plan, true).unwrap();
    }

    #[test]
    fn listener_authority_and_accept_lifecycle() {
        let mut bridge = Bridge::new();
        let generation = bridge.register(SERVICE).unwrap();
        let outbound = bridge.open(APP, PROTO_TCP, SCOPE_TCP).unwrap();
        let (open_out, _) = request(&mut bridge, generation);
        bridge
            .service_write(
                SERVICE,
                generation,
                response(open_out, BRIDGE_COMPLETE, NO_ENDPOINT),
                &[],
            )
            .unwrap();
        let any_8080 = Endpoint {
            ip: [0; 4],
            port: 8080,
        };
        assert_eq!(bridge.bind(APP, outbound, any_8080), Err(EPERM));

        let listener = bridge.open(APP, PROTO_TCP, SCOPE_TCP_LISTEN).unwrap();
        let target = FdTarget::NetSocket { token: listener };
        let (open_listen, _) = request(&mut bridge, generation);
        bridge
            .service_write(
                SERVICE,
                generation,
                response(open_listen, BRIDGE_COMPLETE, NO_ENDPOINT),
                &[],
            )
            .unwrap();
        assert_eq!(bridge.connect(APP, listener, REMOTE), Err(EPERM));
        assert_eq!(
            bridge.bind(
                APP,
                listener,
                Endpoint {
                    ip: [0; 4],
                    port: 80
                }
            ),
            Err(EACCES)
        );
        assert_eq!(bridge.bind(APP, listener, any_8080), Ok(false));
        let (bind_req, _) = request(&mut bridge, generation);
        assert_eq!((bind_req.kind, bind_req.endpoint), (BRIDGE_BIND, any_8080));
        let bound_local = Endpoint {
            ip: LOCAL.ip,
            port: 8080,
        };
        bridge
            .service_write(
                SERVICE,
                generation,
                response(bind_req, BRIDGE_COMPLETE, bound_local),
                &[],
            )
            .unwrap();
        assert_eq!(bridge.listen(APP, listener, 2), Ok(false));
        let (listen_req, backlog) = request(&mut bridge, generation);
        assert_eq!(
            (listen_req.kind, backlog.as_slice()),
            (BRIDGE_LISTEN, 2u32.to_le_bytes().as_slice())
        );
        bridge
            .service_write(
                SERVICE,
                generation,
                response(listen_req, BRIDGE_COMPLETE, NO_ENDPOINT),
                &[],
            )
            .unwrap();
        assert_eq!(bridge.ready(APP, target), WRITE);
        assert_eq!(bridge.listen(APP, listener, 2), Ok(true));
        assert_eq!(bridge.ready(APP, target), 0);
        assert!(matches!(
            bridge.accept_begin(APP, listener),
            Ok(AcceptStep::Empty)
        ));

        let ready = BridgeHeader {
            flags: 1,
            ..response(listen_req, BRIDGE_LISTENER_READY, NO_ENDPOINT)
        };
        bridge.service_write(SERVICE, generation, ready, &[]).unwrap();
        assert_eq!(bridge.ready(APP, target), READ);
        assert!(matches!(
            bridge.accept_begin(APP, listener),
            Ok(AcceptStep::Pending)
        ));
        let (acc_req, acc_payload) = request(&mut bridge, generation);
        assert_eq!(acc_req.kind, BRIDGE_ACCEPT);
        let child_token = u64::from_le_bytes(acc_payload.try_into().unwrap());
        bridge
            .service_write(
                SERVICE,
                generation,
                response(acc_req, BRIDGE_COMPLETE, REMOTE),
                &[],
            )
            .unwrap();
        assert_eq!(bridge.ready(APP, target), READ);
        assert!(matches!(
            bridge.accept_begin(APP, listener),
            Ok(AcceptStep::Ready {
                child_token: got,
                remote
            }) if got == child_token && remote == REMOTE
        ));
        assert_eq!(
            bridge.endpoints(APP, child_token).unwrap(),
            Endpoints {
                local: bound_local,
                remote: REMOTE
            }
        );
    }

    #[test]
    fn v2_ipv6_and_dns_interoperate_with_v1_and_v2_clients() {
        let mut bridge = Bridge::new();
        let generation = bridge.register_version(SERVICE, VERSION_V2).unwrap();
        assert_eq!(bridge.service_version(SERVICE, generation), Ok(VERSION_V2));
        let tcp6 = bridge.open_version(APP, PROTO_TCP, SCOPE_TCP, VERSION_V2).unwrap();
        assert_eq!(bridge.socket_version(APP, tcp6), Ok(VERSION_V2));
        let plan = bridge
            .reserve_service(SERVICE, generation, MAX_FRAME_V2)
            .unwrap();
        let (open_req, _) = BridgeHeader6::decode(plan.bytes()).unwrap();
        bridge.finish_read(&plan, true).unwrap();
        bridge
            .service_write6(
                SERVICE,
                generation,
                BridgeHeader6 {
                    kind: BRIDGE_COMPLETE,
                    flags: BRIDGE_OPEN,
                    endpoint: Endpoint6::NONE,
                    ..open_req
                },
                &[],
            )
            .unwrap();
        let local6 = Endpoint6::ipv6(
            [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10],
            50000,
            0,
        );
        let remote6 = Endpoint6::ipv6(
            [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x20],
            443,
            0,
        );
        assert_eq!(bridge.connect6(APP, tcp6, remote6), Ok(false));
        let plan = bridge
            .reserve_service(SERVICE, generation, MAX_FRAME_V2)
            .unwrap();
        let (conn_req, _) = BridgeHeader6::decode(plan.bytes()).unwrap();
        assert_eq!(conn_req.endpoint, remote6);
        bridge.finish_read(&plan, true).unwrap();
        bridge
            .service_write6(
                SERVICE,
                generation,
                BridgeHeader6 {
                    kind: BRIDGE_COMPLETE,
                    flags: BRIDGE_CONNECT,
                    endpoint: local6,
                    ..conn_req
                },
                &[],
            )
            .unwrap();
        assert_eq!(
            bridge.endpoints6(APP, tcp6).unwrap(),
            Endpoints6 {
                local: local6,
                remote: remote6
            }
        );

        let dns_v1 = bridge.open_version(APP, PROTO_DNS, SCOPE_UDP, VERSION).unwrap();
        let plan = bridge
            .reserve_service(SERVICE, generation, MAX_FRAME_V2)
            .unwrap();
        let (dns_open, _) = BridgeHeader6::decode(plan.bytes()).unwrap();
        bridge.finish_read(&plan, true).unwrap();
        bridge
            .service_write6(
                SERVICE,
                generation,
                BridgeHeader6 {
                    kind: BRIDGE_COMPLETE,
                    flags: BRIDGE_OPEN,
                    endpoint: Endpoint6::NONE,
                    ..dns_open
                },
                &[],
            )
            .unwrap();
        bridge.send6(APP, dns_v1, Endpoint6::NONE, b"du.test").unwrap();
        let plan = bridge
            .reserve_service(SERVICE, generation, MAX_FRAME_V2)
            .unwrap();
        let (dns_req, q) = BridgeHeader6::decode(plan.bytes()).unwrap();
        assert_eq!(q, b"du.test");
        bridge.finish_read(&plan, true).unwrap();
        let mut dns_payload = [0; 64];
        let dns_len = encode_dns_records(
            &[
                DnsRecord::ipv6(remote6.addr, 0),
                DnsRecord::ipv4([192, 0, 2, 44]),
            ],
            &mut dns_payload,
        )
        .unwrap();
        bridge
            .service_write6(
                SERVICE,
                generation,
                BridgeHeader6 {
                    kind: BRIDGE_DATA,
                    flags: 0,
                    endpoint: Endpoint6::NONE,
                    ..dns_req
                },
                &dns_payload[..dns_len],
            )
            .unwrap();
        let read_plan = bridge.reserve_client(APP, dns_v1, 64, false).unwrap().unwrap();
        assert_eq!(read_plan.bytes(), &[192, 0, 2, 44]);
        bridge.finish_read(&read_plan, true).unwrap();
    }
}
