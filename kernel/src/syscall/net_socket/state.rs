//! Fixed-size, generation-bound socket proxy state. Neither packet policy nor
//! user memory access is performed while the bridge is locked.
use super::*;

const SOCKETS: usize = 8;
const SOCKETS_PER_OWNER: usize = 4;
pub(super) const MAX_UDP: usize = 1472;
pub(super) const READ: u16 = 1;
pub(super) const WRITE: u16 = 2;
pub(super) const HANGUP: u16 = 4;
pub(super) const ERROR: u16 = 8;
const NO_ENDPOINT: Endpoint = Endpoint {
    ip: [0; 4],
    port: 0,
};

pub(super) struct ReadPlan {
    bytes: [u8; MAX_FRAME],
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
            bytes: [0; MAX_FRAME],
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
    endpoint: Endpoint,
    bytes: [u8; MAX_PAYLOAD],
    len: usize,
    sent: bool,
}

impl Request {
    fn new(kind: u16, endpoint: Endpoint, payload: &[u8]) -> Self {
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
    endpoint: Endpoint,
    bytes: [u8; MAX_PAYLOAD],
    start: usize,
    len: usize,
}

struct Socket {
    token: u64,
    owner: u64,
    protocol: u16,
    open: bool,
    local: Endpoint,
    remote: Endpoint,
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
    fn new(token: u64, owner: u64, protocol: u16) -> Self {
        Self {
            token,
            owner,
            protocol,
            open: false,
            local: NO_ENDPOINT,
            remote: NO_ENDPOINT,
            error: 0,
            eof: false,
            write_closed: false,
            closing: false,
            request: Some(Request::new(BRIDGE_OPEN, NO_ENDPOINT, &[])),
            rx: None,
            rx_reserved: false,
            credit: false,
        }
    }

    fn header(&self, kind: u16, endpoint: Endpoint) -> BridgeHeader {
        BridgeHeader {
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

    fn next_request(&self) -> Option<(BridgeHeader, &[u8])> {
        if self.closing {
            Some((self.header(BRIDGE_CLOSE, NO_ENDPOINT), &[]))
        } else if self.credit {
            Some((self.header(BRIDGE_CREDIT, NO_ENDPOINT), &[]))
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

    fn finish(&mut self, header: BridgeHeader) -> Result<(), i64> {
        let pending = self
            .request
            .as_ref()
            .filter(|value| value.sent)
            .ok_or(EINVAL)?;
        if header.flags != pending.kind {
            return Err(EINVAL);
        }
        if !header.endpoint.is_none()
            && (!header.endpoint.is_unicast()
                || (!self.local.is_none() && self.local != header.endpoint))
        {
            return Err(EINVAL);
        }
        if header.error != 0 {
            self.error = i64::from(header.error);
        } else {
            match pending.kind {
                BRIDGE_OPEN => self.open = true,
                BRIDGE_CONNECT => self.remote = pending.endpoint,
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

    pub(super) fn register(&mut self, owner: u64) -> Result<u64, i64> {
        if self.service.is_some() {
            return Err(EBUSY);
        }
        let generation = self.next_token()?;
        self.service = Some(Service {
            owner,
            generation,
            cursor: 0,
            read_reserved: false,
        });
        Ok(generation)
    }

    pub(super) fn open(&mut self, owner: u64, protocol: u16) -> Result<u64, i64> {
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
        let slot = self
            .sockets
            .iter()
            .position(Option::is_none)
            .ok_or(ENFILE)?;
        let token = self.next_token()?;
        self.sockets[slot] = Some(Socket::new(token, owner, protocol));
        Ok(token)
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
        if socket.owner != owner || socket.closing {
            return Err(EPERM);
        }
        Ok(socket)
    }

    fn client_mut(&mut self, owner: u64, token: u64) -> Result<&mut Socket, i64> {
        let index = self.socket_index(token).ok_or(EPIPE)?;
        let socket = self.sockets[index].as_mut().ok_or(EPIPE)?;
        if socket.owner != owner || socket.closing {
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
                let socket = self.sockets[index].as_mut().expect("known socket");
                if socket.closing {
                    return false;
                }
                socket.closing = true;
                socket.request = None;
                socket.rx = None;
                socket.credit = false;
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
        let service = self.service(owner, generation)?;
        if service.read_reserved {
            return Err(EAGAIN);
        }
        let start = service.cursor;
        for offset in 0..SOCKETS {
            let index = (start + offset) % SOCKETS;
            let Some(socket) = self.sockets[index].as_ref() else {
                continue;
            };
            let Some((header, payload)) = socket.next_request() else {
                continue;
            };
            if BRIDGE_HEADER_LEN + payload.len() > capacity {
                return Err(EMSGSIZE);
            }
            let mut plan = ReadPlan::new(Source::Service {
                generation,
                token: socket.token,
                kind: header.kind,
            });
            plan.len = header
                .encode(payload, &mut plan.bytes)
                .map_err(wire_error)?;
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
        if from && socket.protocol != PROTO_UDP {
            return Err(EOPNOTSUPP);
        }
        if from && capacity < RECV_HEADER_LEN {
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
        let prefix = if from { RECV_HEADER_LEN } else { 0 };
        let count = (rx.len - rx.start).min(capacity - prefix);
        let consumed = if socket.protocol == PROTO_UDP {
            rx.len
        } else {
            count
        };
        let mut plan = ReadPlan::new(Source::Socket { token, consumed });
        if from {
            plan.bytes[..prefix].copy_from_slice(
                &RecvHeader {
                    endpoint: rx.endpoint,
                    full_len: rx.len as u32,
                }
                .encode(),
            );
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

    pub(super) fn service_write(
        &mut self,
        owner: u64,
        generation: u64,
        header: BridgeHeader,
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
                socket.finish(header)
            }
            BRIDGE_DATA if header.flags == 0 && header.error == 0 => {
                if !socket.open || socket.eof || socket.error != 0 {
                    return Err(EPIPE);
                }
                let valid = if socket.protocol == PROTO_UDP {
                    header.endpoint.is_unicast() && data.len() <= MAX_UDP
                } else {
                    header.endpoint.is_none() && !socket.remote.is_none() && !data.is_empty()
                };
                if !valid {
                    return Err(EINVAL);
                }
                if socket.rx.is_some() || socket.credit {
                    return Err(EAGAIN);
                }
                let mut bytes = [0; MAX_PAYLOAD];
                bytes[..data.len()].copy_from_slice(data);
                socket.rx = Some(Receive {
                    endpoint: header.endpoint,
                    bytes,
                    start: 0,
                    len: data.len(),
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

    pub(super) fn protocol(&self, owner: u64, token: u64) -> Result<u16, i64> {
        Ok(self.client(owner, token)?.protocol)
    }

    pub(super) fn send(
        &mut self,
        owner: u64,
        token: u64,
        endpoint: Endpoint,
        payload: &[u8],
    ) -> Result<(), i64> {
        let socket = self.client_mut(owner, token)?;
        socket.check_request()?;
        let max = if socket.protocol == PROTO_UDP {
            MAX_UDP
        } else {
            MAX_PAYLOAD
        };
        if payload.len() > max {
            return Err(EMSGSIZE);
        }
        if socket.write_closed {
            return Err(EPIPE);
        }
        if endpoint.is_none() && socket.remote.is_none() {
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

    pub(super) fn connect(
        &mut self,
        owner: u64,
        token: u64,
        endpoint: Endpoint,
    ) -> Result<bool, i64> {
        let socket = self.client_mut(owner, token)?;
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
        if socket.protocol != PROTO_TCP {
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
        socket.request = Some(Request::new(BRIDGE_SHUTDOWN, NO_ENDPOINT, &[]));
        Ok(true)
    }

    pub(super) fn error(&self, owner: u64, token: u64) -> Result<i32, i64> {
        Ok(self.client(owner, token)?.error as i32)
    }

    pub(super) fn endpoints(&self, owner: u64, token: u64) -> Result<Endpoints, i64> {
        let socket = self.client(owner, token)?;
        Ok(Endpoints {
            local: socket.local,
            remote: socket.remote,
        })
    }
}

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
        assert_eq!(bridge.open(APP, PROTO_TCP), Err(EPIPE));
        let generation = bridge.register(SERVICE).unwrap();
        assert_eq!(bridge.register(OTHER), Err(EBUSY));
        let socket = bridge.open(APP, PROTO_TCP).unwrap();
        for _ in 1..SOCKETS_PER_OWNER {
            bridge.open(APP, PROTO_UDP).unwrap();
        }
        assert_eq!(bridge.open(APP, PROTO_TCP), Err(ENFILE));
        assert!(bridge.open(OTHER, PROTO_TCP).is_ok());
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
        let fresh_socket = bridge.open(APP, PROTO_TCP).unwrap();
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
        let token = bridge.open(APP, PROTO_TCP).unwrap();
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
        let token = bridge.open(APP, PROTO_UDP).unwrap();
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
}
