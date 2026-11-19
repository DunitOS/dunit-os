//! Bounded protocol engine behind the kernel-owned application descriptors.
use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use dunit::{errno, Error, Result};
use dunit_net_core::{
    tcp::{
        self,
        table::{Outbound as TcpPacket, StreamId, Table, MAX_STREAMS},
        State, StreamError,
    },
    udp::Address,
    Egress, Error as NetError, Ip, Peer,
};
use dunit_socket_abi::{
    BridgeHeader, Endpoint, Options, BRIDGE_CLOSE, BRIDGE_COMPLETE, BRIDGE_CONNECT, BRIDGE_CREDIT,
    BRIDGE_DATA, BRIDGE_EOF, BRIDGE_ERROR, BRIDGE_OPEN, BRIDGE_SEND, BRIDGE_SHUTDOWN, MAX_FRAME,
    MAX_PAYLOAD, PROTO_TCP, PROTO_UDP,
};
use netd::{checked, Device};

use crate::{interface::Interface, outbound::Outbound};

const SOCKETS: usize = 8;
const REQUEST_TIMEOUT_MS: u64 = 10_000;
const RETRY_MS: u64 = 20;

#[derive(Clone, Copy)]
enum Transport {
    None,
    Udp(u16),
    Tcp(StreamId),
}

struct Request {
    kind: u16,
    destination: Endpoint,
    data: Vec<u8>,
    offset: usize,
    deadline: u64,
}

struct Socket {
    open: BridgeHeader,
    transport: Transport,
    request: Option<Request>,
    local: Endpoint,
    credit: bool,
    eof: bool,
    failed: bool,
    rx: Option<(Endpoint, Vec<u8>)>,
}

impl Socket {
    fn new(open: BridgeHeader) -> Self {
        Self {
            open,
            transport: Transport::None,
            request: None,
            local: Endpoint::NONE,
            credit: true,
            eof: false,
            failed: false,
            rx: None,
        }
    }

    fn bound_udp(&mut self, ip: Ip, peer: &mut Peer) -> core::result::Result<u16, isize> {
        match self.transport {
            Transport::Udp(port) => Ok(port),
            Transport::None if self.open.protocol == PROTO_UDP => {
                let port = peer.udp_bind(0).map_err(net_errno)?;
                self.transport = Transport::Udp(port);
                self.local = Endpoint { ip, port };
                Ok(port)
            }
            _ => Err(errno::EINVAL),
        }
    }
}

struct Bridge {
    fd: u32,
    write_buffer: Box<[u8; MAX_FRAME]>,
}

impl Bridge {
    /// A stale response can race the application's last close.
    fn write(
        &mut self,
        socket: BridgeHeader,
        kind: u16,
        flags: u16,
        error: isize,
        endpoint: Endpoint,
        data: &[u8],
    ) -> Result<()> {
        let header = BridgeHeader {
            kind,
            flags,
            error: error as i32,
            endpoint,
            ..socket
        };
        let size = header
            .encode(data, self.write_buffer.as_mut())
            .map_err(|_| Error(errno::EIO))?;
        match checked(libdunit::write(
            self.fd as usize,
            &self.write_buffer[..size],
        )) {
            Ok(written) if written == size => Ok(()),
            Err(Error(errno::ENOENT)) => Ok(()),
            Ok(_) => Err(Error(errno::EIO)),
            Err(error) => Err(error),
        }
    }

    fn complete(
        &mut self,
        socket: BridgeHeader,
        kind: u16,
        error: isize,
        local: Endpoint,
    ) -> Result<()> {
        self.write(socket, BRIDGE_COMPLETE, kind, error, local, &[])
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        libdunit::close(self.fd as usize);
    }
}

pub struct SocketService {
    bridge: Bridge,
    read_buffer: Box<[u8; MAX_FRAME]>,
    sockets: [Option<Socket>; SOCKETS],
    revision: u64,
    ip: Option<Ip>,
    tcp: Option<Table>,
    wire: VecDeque<TcpPacket>,
    mtu: usize,
    seed: u32,
    retry_at: Option<u64>,
}

impl SocketService {
    pub fn open(device: &Device, mtu: usize, seed: u32) -> Result<Option<Self>> {
        let rights = checked(libdunit::handle_rights(device.handle()))?;
        if rights & libdunit::RIGHT_NET_SERVICE as usize == 0 {
            return Ok(None);
        }
        let fd = checked(libdunit::net_service_register(
            device.handle(),
            &Options.encode(),
        ))?;
        let fd = u32::try_from(fd).map_err(|_| Error(errno::EIO))?;
        Ok(Some(Self {
            bridge: Bridge {
                fd,
                write_buffer: Box::new([0; MAX_FRAME]),
            },
            read_buffer: Box::new([0; MAX_FRAME]),
            sockets: core::array::from_fn(|_| None),
            revision: 0,
            ip: None,
            tcp: None,
            wire: VecDeque::with_capacity(MAX_STREAMS),
            mtu,
            seed,
            retry_at: None,
        }))
    }

    pub fn descriptor(&self) -> u32 {
        self.bridge.fd
    }

    pub fn deadline(&self) -> Option<u64> {
        self.retry_at
            .into_iter()
            .chain(self.tcp.as_ref().and_then(Table::deadline))
            .chain(
                self.sockets
                    .iter()
                    .flatten()
                    .filter_map(|slot| slot.request.as_ref().map(|req| req.deadline)),
            )
            .min()
    }

    pub fn obsolete(&self, interface: &Interface) -> bool {
        self.ip.is_some() && self.revision != interface.revision()
    }

    pub fn sync(&mut self, interface: &Interface) -> Result<()> {
        if interface.revision() == self.revision {
            return Ok(());
        }
        self.revision = interface.revision();
        self.ip = interface.config().map(|config| config.address);
        self.seed = self.seed.wrapping_add(0x9e3779b9);
        self.tcp = self
            .ip
            .map(|ip| Table::new(ip, self.mtu, self.seed))
            .transpose()
            .map_err(|error| Error(tcp_errno(error)))?;
        self.wire.clear();
        Ok(())
    }

    pub fn ingress(&mut self, peer: &Peer, frame: &[u8], now: u64) -> bool {
        let Ok(Some(ip)) = peer.tcp_ingress(frame) else {
            return false;
        };
        if let Some(tcp) = &mut self.tcp {
            // Malformed, unmatched and old sequence traffic is not a service
            // error. The four-tuple and TCP state are validated by the table.
            let _ = tcp.receive(ip.source, ip.destination, ip.payload, now);
        }
        true
    }

    pub fn step(&mut self, interface: &mut Interface, tx: &mut Outbound, now: u64) -> Result<()> {
        self.sync(interface)?;
        self.read_requests(interface.peer(), now)?;
        let mut peer = interface.peer();
        let mut retry = false;
        for socket in self.sockets.iter_mut().flatten() {
            if socket.failed {
                continue;
            }
            if let Some(mut request) = socket.request.take() {
                if now >= request.deadline {
                    retire(
                        socket,
                        peer.as_deref_mut(),
                        self.tcp.as_mut(),
                        &mut self.wire,
                    );
                    fail(&mut self.bridge, socket, errno::EIO)?;
                    continue;
                }
                let status = match (self.ip, &mut peer, &mut self.tcp) {
                    (Some(ip), Some(peer), Some(tcp)) => {
                        progress(socket, &mut request, ip, peer, tcp, tx, now)
                    }
                    _ => Ok(false),
                };
                match status {
                    Ok(true) => self
                        .bridge
                        .complete(socket.open, request.kind, 0, socket.local)?,
                    Ok(false) => {
                        retry |= self.ip.is_some() && socket.open.protocol == PROTO_UDP;
                        socket.request = Some(request);
                    }
                    Err(error) => {
                        self.bridge
                            .complete(socket.open, request.kind, error, Endpoint::NONE)?;
                        retire(
                            socket,
                            peer.as_deref_mut(),
                            self.tcp.as_mut(),
                            &mut self.wire,
                        );
                        socket.failed = true;
                    }
                }
            }
            receive(
                &mut self.bridge,
                socket,
                peer.as_deref_mut(),
                self.tcp.as_mut(),
            )?;
        }

        if let (Some(peer), Some(tcp)) = (peer, &mut self.tcp) {
            for _ in self.wire.len()..MAX_STREAMS {
                let Some(packet) = tcp.poll(now).map_err(|error| Error(tcp_errno(error)))? else {
                    break;
                };
                self.wire.push_back(packet);
            }
            for _ in 0..self.wire.len() {
                let packet = self.wire.pop_front().unwrap();
                let sent =
                    match peer.tcp_egress(packet.source, packet.destination, &packet.bytes, now) {
                        Ok(Egress::Sent(frame)) => tx.try_push(&frame)?,
                        Ok(Egress::Neighbor(frame)) => {
                            tx.push(&frame)?;
                            false
                        }
                        Ok(Egress::Waiting) => false,
                        // The TCP timer retries transient misses independently.
                        Err(_) => true,
                    };
                if !sent {
                    self.wire.push_back(packet);
                }
            }
            retry |= !self.wire.is_empty();
            for socket in self.sockets.iter_mut().flatten() {
                report_tcp_error(&mut self.bridge, socket, tcp)?;
            }
        }
        self.retry_at = retry.then_some(now.saturating_add(RETRY_MS));
        Ok(())
    }

    fn read_requests(&mut self, mut peer: Option<&mut Peer>, now: u64) -> Result<()> {
        // Keep socket requests from starving link and lease timers.
        for _ in 0..(SOCKETS * 2) {
            let size = match checked(libdunit::read(
                self.bridge.fd as usize,
                self.read_buffer.as_mut(),
            )) {
                Ok(size) if size != 0 && size <= MAX_FRAME => size,
                Err(Error(errno::EAGAIN)) => break,
                Ok(_) => return Err(Error(errno::EIO)),
                Err(error) => return Err(error),
            };
            let (header, payload) =
                BridgeHeader::decode(&self.read_buffer[..size]).map_err(|_| Error(errno::EIO))?;
            let index = self.sockets.iter().position(|slot| {
                slot.as_ref().is_some_and(|socket| {
                    socket.open.token == header.token
                        && socket.open.owner == header.owner
                        && socket.open.protocol == header.protocol
                })
            });
            match (header.kind, index) {
                (BRIDGE_OPEN, None) => {
                    let Some(slot) = self.sockets.iter_mut().find(|slot| slot.is_none()) else {
                        self.bridge.complete(
                            header,
                            BRIDGE_OPEN,
                            errno::ENOBUFS,
                            Endpoint::NONE,
                        )?;
                        continue;
                    };
                    *slot = Some(Socket::new(header));
                    self.bridge
                        .complete(header, BRIDGE_OPEN, 0, Endpoint::NONE)?;
                }
                (BRIDGE_CLOSE, Some(index)) => {
                    let mut socket = self.sockets[index].take().unwrap();
                    retire(
                        &mut socket,
                        peer.as_deref_mut(),
                        self.tcp.as_mut(),
                        &mut self.wire,
                    );
                }
                (BRIDGE_CREDIT, Some(index)) => self.sockets[index].as_mut().unwrap().credit = true,
                (BRIDGE_CLOSE | BRIDGE_CREDIT, None) => {}
                (BRIDGE_CONNECT | BRIDGE_SEND | BRIDGE_SHUTDOWN, Some(index)) => {
                    let socket = self.sockets[index].as_mut().unwrap();
                    if socket.failed {
                        self.bridge
                            .complete(header, header.kind, errno::EPIPE, Endpoint::NONE)?;
                    } else if socket.request.is_some() {
                        return Err(Error(errno::EIO));
                    } else {
                        socket.request = Some(Request {
                            kind: header.kind,
                            destination: header.endpoint,
                            data: payload.to_vec(),
                            offset: 0,
                            deadline: now.saturating_add(REQUEST_TIMEOUT_MS),
                        });
                    }
                }
                _ => return Err(Error(errno::EIO)),
            }
        }
        Ok(())
    }
}

fn progress(
    socket: &mut Socket,
    request: &mut Request,
    ip: Ip,
    peer: &mut Peer,
    tcp: &mut Table,
    tx: &mut Outbound,
    now: u64,
) -> core::result::Result<bool, isize> {
    let remote = request.destination;
    match (socket.open.protocol, request.kind) {
        (PROTO_UDP, BRIDGE_CONNECT) => {
            let port = socket.bound_udp(ip, peer)?;
            peer.udp_connect(
                port,
                Some(Address {
                    ip: remote.ip,
                    port: remote.port,
                }),
            )
            .map_err(net_errno)?;
            Ok(true)
        }
        (PROTO_UDP, BRIDGE_SEND) => {
            let port = socket.bound_udp(ip, peer)?;
            let address = (!remote.is_none()).then_some(Address {
                ip: remote.ip,
                port: remote.port,
            });
            match peer
                .udp_send(port, address, &request.data, now)
                .map_err(net_errno)?
            {
                Egress::Sent(frame) => tx.try_push(&frame).map_err(|error| error.0),
                Egress::Neighbor(frame) => {
                    tx.push(&frame).map_err(|error| error.0)?;
                    Ok(false)
                }
                Egress::Waiting => Ok(false),
            }
        }
        (PROTO_TCP, kind) => {
            let id = match socket.transport {
                Transport::Tcp(id) => id,
                Transport::None if kind == BRIDGE_CONNECT => {
                    let id = tcp
                        .connect(
                            tcp::Endpoint {
                                ip: remote.ip,
                                port: remote.port,
                            },
                            now,
                        )
                        .map_err(tcp_errno)?;
                    let (local, _) = tcp.endpoints(id).map_err(tcp_errno)?;
                    socket.local = Endpoint {
                        ip: local.ip,
                        port: local.port,
                    };
                    socket.transport = Transport::Tcp(id);
                    id
                }
                _ => return Err(errno::EINVAL),
            };
            match kind {
                BRIDGE_CONNECT => match tcp.state(id).map_err(tcp_errno)? {
                    State::Established | State::CloseWait => Ok(true),
                    State::Closed => Err(tcp
                        .read(id, &mut [0; 1])
                        .err()
                        .map_or(errno::EPIPE, tcp_errno)),
                    _ => Ok(false),
                },
                BRIDGE_SEND => match tcp.send(id, &request.data[request.offset..]) {
                    Ok(count) => {
                        request.offset += count;
                        Ok(request.offset == request.data.len())
                    }
                    Err(StreamError::WouldBlock) => Ok(false),
                    Err(error) => Err(tcp_errno(error)),
                },
                BRIDGE_SHUTDOWN => tcp.shutdown_write(id).map(|()| true).map_err(tcp_errno),
                _ => Err(errno::EOPNOTSUPP),
            }
        }
        _ => Err(errno::EOPNOTSUPP),
    }
}

fn report_tcp_error(bridge: &mut Bridge, socket: &mut Socket, tcp: &mut Table) -> Result<()> {
    if let Transport::Tcp(id) = socket.transport {
        if !socket.failed && tcp.readiness(id).is_ok_and(|ready| ready.error) {
            let error = tcp
                .read(id, &mut [0; 1])
                .err()
                .map_or(errno::EPIPE, tcp_errno);
            fail(bridge, socket, error)?;
        }
    }
    Ok(())
}

fn receive(
    bridge: &mut Bridge,
    socket: &mut Socket,
    peer: Option<&mut Peer>,
    tcp: Option<&mut Table>,
) -> Result<()> {
    if socket.failed {
        return Ok(());
    }
    if socket.rx.is_none() && socket.credit && !socket.eof {
        match (socket.transport, peer, tcp) {
            (Transport::Udp(port), Some(peer), _) => {
                // Source port zero is permitted on the wire but does not name
                // a service endpoint. Drop it before forming a bridge record.
                while let Ok(packet) = peer.udp_recv(port) {
                    let source = Endpoint {
                        ip: packet.source.ip,
                        port: packet.source.port,
                    };
                    if source.is_unicast() {
                        socket.rx = Some((source, packet.data));
                        break;
                    }
                }
            }
            (Transport::Tcp(id), _, Some(tcp)) => {
                let ready = tcp.readiness(id).map_err(|error| Error(tcp_errno(error)))?;
                if ready.readable || ready.error {
                    let mut data = alloc::vec![0; MAX_PAYLOAD];
                    match tcp.read(id, &mut data) {
                        Ok(0) => {
                            bridge.write(socket.open, BRIDGE_EOF, 0, 0, Endpoint::NONE, &[])?;
                            socket.eof = true;
                        }
                        Ok(count) => {
                            data.truncate(count);
                            socket.rx = Some((Endpoint::NONE, data));
                        }
                        Err(StreamError::WouldBlock) => {}
                        Err(error) => fail(bridge, socket, tcp_errno(error))?,
                    }
                }
            }
            _ => {}
        }
    }
    if let Some((endpoint, data)) = socket.rx.as_ref() {
        match bridge.write(socket.open, BRIDGE_DATA, 0, 0, *endpoint, data) {
            Ok(()) => {
                socket.rx = None;
                socket.credit = false;
            }
            Err(Error(errno::EAGAIN)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn retire(
    socket: &mut Socket,
    peer: Option<&mut Peer>,
    tcp: Option<&mut Table>,
    wire: &mut VecDeque<TcpPacket>,
) {
    match (socket.transport, peer, tcp) {
        (Transport::Udp(port), Some(peer), _) => {
            let _ = peer.udp_close(port);
        }
        (Transport::Tcp(id), _, Some(tcp)) => {
            if let Ok((source, destination)) = tcp.endpoints(id) {
                wire.retain(|packet| (packet.source, packet.destination) != (source, destination));
            }
            let _ = tcp.close(id);
        }
        _ => {}
    }
    socket.transport = Transport::None;
}

fn fail(bridge: &mut Bridge, socket: &mut Socket, error: isize) -> Result<()> {
    bridge.write(socket.open, BRIDGE_ERROR, 0, error, Endpoint::NONE, &[])?;
    socket.failed = true;
    socket.request = None;
    socket.rx = None;
    Ok(())
}

fn net_errno(error: NetError) -> isize {
    match error {
        NetError::Length => errno::EMSGSIZE,
        NetError::Capacity => errno::ENOBUFS,
        NetError::WouldBlock => errno::EAGAIN,
        NetError::Unsupported | NetError::Protocol => errno::EOPNOTSUPP,
        NetError::NoRoute | NetError::Timeout => errno::EIO,
        _ => errno::EINVAL,
    }
}

fn tcp_errno(error: StreamError) -> isize {
    match error {
        StreamError::Packet(error) => net_errno(error),
        StreamError::Closed | StreamError::Reset => errno::EPIPE,
        StreamError::WouldBlock => errno::EAGAIN,
        StreamError::Timeout | StreamError::Clock => errno::EIO,
    }
}
