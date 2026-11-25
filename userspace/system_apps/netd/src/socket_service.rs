//! Multi-NIC dual-stack (IPv4 + IPv6) protocol engine behind the kernel socket bridge.
use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use dunit::{errno, Error, Result};
use dunit_net_core::{
    ipv6::{self, PROTO_TCP, PROTO_UDP as V6_PROTO_UDP},
    ipv6_peer::Egress6,
    router::{MultiRouter, RouteChoice},
    tcp::{
        table::{DualOutbound, ListenerId, StreamId, Table, MAX_PENDING, MAX_STREAMS},
        DualEndpoint, State, StreamError,
    },
    udp::{DualAddress, DualEndpoints, Udp},
    Egress, Error as NetError, IpAddr,
};
use dunit_socket_abi::{
    BridgeHeader6, Endpoint6, Options, BRIDGE_ACCEPT, BRIDGE_BIND, BRIDGE_CLOSE, BRIDGE_COMPLETE,
    BRIDGE_CONNECT, BRIDGE_CREDIT, BRIDGE_DATA, BRIDGE_EOF, BRIDGE_ERROR, BRIDGE_LISTEN,
    BRIDGE_LISTENER_READY, BRIDGE_OPEN, BRIDGE_SEND, BRIDGE_SHUTDOWN, FAMILY_IPV4, FAMILY_IPV6,
    FAMILY_NONE, MAX_FRAME_V2, MAX_PAYLOAD, PROTO_DNS, PROTO_TCP as ABI_TCP,
    PROTO_UDP as ABI_UDP, VERSION_V2,
};
use netd::{checked, Device};

use crate::{dns_runtime::DualDnsRuntime, nic::Nic};

const SOCKETS: usize = 16;
const REQUEST_TIMEOUT_MS: u64 = 10_000;
const DNS_REQUEST_TIMEOUT_MS: u64 = 15_000;
const RETRY_MS: u64 = 20;

#[derive(Clone, Copy)]
enum Transport {
    None,
    Udp(u16, Option<(DualAddress, u32)>),
    Tcp(StreamId),
    TcpBound(u16, Endpoint6),
    Listener(ListenerId),
}

struct Request {
    kind: u16,
    destination: Endpoint6,
    data: Vec<u8>,
    offset: usize,
    deadline: u64,
}

struct Socket {
    open: BridgeHeader6,
    transport: Transport,
    request: Option<Request>,
    local: Endpoint6,
    credit: bool,
    eof: bool,
    failed: bool,
    listener_notified: bool,
    rx: Option<(Endpoint6, Vec<u8>)>,
}

impl Socket {
    fn new(open: BridgeHeader6) -> Self {
        Self {
            open,
            transport: Transport::None,
            request: None,
            local: Endpoint6::NONE,
            credit: true,
            eof: false,
            failed: false,
            listener_notified: false,
            rx: None,
        }
    }

    fn bound_udp(
        &mut self,
        local_ep: Endpoint6,
        udp: &mut DualEndpoints,
    ) -> core::result::Result<u16, isize> {
        match self.transport {
            Transport::Udp(port, _) => Ok(port),
            Transport::None if self.open.protocol == ABI_UDP => {
                let port = udp.bind(0).map_err(net_errno)?;
                self.transport = Transport::Udp(port, None);
                self.local = Endpoint6 { port, ..local_ep };
                Ok(port)
            }
            _ => Err(errno::EINVAL),
        }
    }
}

struct Bridge {
    fd: u32,
    write_buffer: Box<[u8; MAX_FRAME_V2]>,
}

impl Bridge {
    fn write(
        &mut self,
        socket: BridgeHeader6,
        kind: u16,
        flags: u16,
        error: isize,
        endpoint: Endpoint6,
        data: &[u8],
    ) -> Result<()> {
        let header = BridgeHeader6 {
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
        socket: BridgeHeader6,
        kind: u16,
        error: isize,
        local: Endpoint6,
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
    read_buffer: Box<[u8; MAX_FRAME_V2]>,
    sockets: Box<[Option<Socket>; SOCKETS]>,
    pub router: MultiRouter,
    tcp: Box<Table>,
    udp: Box<DualEndpoints>,
    dns: Box<DualDnsRuntime>,
    wire: VecDeque<DualOutbound>,
    retry_at: Option<u64>,
    v4_ever_bound: bool,
    v4_revoked: bool,
}

impl SocketService {
    pub fn open(device: &Device, mtu: usize, seed: u32) -> Result<Option<Self>> {
        let rights = checked(libdunit::handle_rights(device.handle()))?;
        if rights & libdunit::RIGHT_NET_SERVICE as usize == 0 {
            return Ok(None);
        }
        let fd = checked(libdunit::net_service_register(
            device.handle(),
            &Options::encode_version(VERSION_V2),
        ))?;
        let fd = u32::try_from(fd).map_err(|_| Error(errno::EIO))?;
        let mut udp = Box::new(DualEndpoints::new());
        let dns = Box::new(DualDnsRuntime::open(&mut udp).map_err(Error)?);
        let tcp = Box::new(Table::new_dual(mtu, seed).map_err(|e| Error(tcp_errno(e)))?);
        Ok(Some(Self {
            bridge: Bridge {
                fd,
                write_buffer: Box::new([0; MAX_FRAME_V2]),
            },
            read_buffer: Box::new([0; MAX_FRAME_V2]),
            sockets: Box::new(core::array::from_fn(|_| None)),
            router: MultiRouter::new(),
            tcp,
            udp,
            dns,
            wire: VecDeque::with_capacity(MAX_STREAMS * 2),
            retry_at: None,
            v4_ever_bound: false,
            v4_revoked: false,
        }))
    }

    pub fn descriptor(&self) -> u32 {
        self.bridge.fd
    }

    pub fn sync_nics(&mut self, nics: &[Box<Nic>]) -> Result<()> {
        for nic in nics {
            self.router.set_interface(nic.view());
        }
        let v4_usable = self.router.has_usable_v4();
        if v4_usable {
            self.v4_ever_bound = true;
            self.v4_revoked = false;
        } else if self.v4_ever_bound || !nics.iter().any(|n| n.can_ever_v4()) {
            self.v4_revoked = true;
        }
        for slot in self.sockets.iter_mut() {
            let Some(socket) = slot.as_mut() else {
                continue;
            };
            if socket.failed {
                continue;
            }
            let local_lost = match socket.local.family {
                FAMILY_IPV4 => {
                    if socket.local.addr[..4] != [0; 4] {
                        !nics.iter().any(|n| {
                            n.link
                                && n.view()
                                    .ipv4
                                    .is_some_and(|(ip, _)| ip == socket.local.addr[..4])
                        })
                    } else {
                        self.v4_revoked
                    }
                }
                FAMILY_IPV6 if socket.local.addr != [0; 16] => !nics
                    .iter()
                    .any(|n| n.link && n.view().owns_ip(IpAddr::V6(socket.local.addr))),
                _ => false,
            };
            if local_lost {
                if let Some(req) = socket.request.take() {
                    self.bridge.complete(
                        socket.open,
                        req.kind,
                        errno::EPIPE,
                        Endpoint6::NONE,
                    )?;
                } else {
                    fail(&mut self.bridge, socket, errno::EPIPE)?;
                }
                retire(
                    socket,
                    &mut self.udp,
                    &mut self.tcp,
                    &mut self.dns,
                    &mut self.wire,
                );
                socket.failed = true;
            }
        }
        Ok(())
    }

    pub fn deadline(&self) -> Option<u64> {
        self.retry_at
            .into_iter()
            .chain(self.tcp.deadline())
            .chain(self.dns.deadline())
            .chain(
                self.sockets
                    .iter()
                    .flatten()
                    .filter_map(|slot| slot.request.as_ref().map(|req| req.deadline)),
            )
            .min()
    }

    pub fn ingress(&mut self, nic: &mut Nic, frame: &[u8], now: u64) -> Result<()> {
        if Nic::is_v6_ethertype(frame) {
            if let Some(peer6) = &mut nic.v6 {
                match peer6.receive(frame, now) {
                    Ok(dunit_net_core::ipv6_peer::RxEvent6::Reply(reply)) => {
                        let _ = nic.tx.try_push(&reply);
                    }
                    Ok(dunit_net_core::ipv6_peer::RxEvent6::Flush(frames)) => {
                        for f in frames {
                            let _ = nic.tx.try_push(&f);
                        }
                    }
                    Ok(dunit_net_core::ipv6_peer::RxEvent6::Udp {
                        source,
                        source_port,
                        destination_port,
                        payload,
                        ..
                    }) => {
                        if source_port == 53 && destination_port == self.dns.port() {
                            self.dns.receive_udp(IpAddr::V6(source), payload, now);
                        } else {
                            let _ = self.udp.deliver(
                                IpAddr::V6(source),
                                Udp {
                                    source: source_port,
                                    destination: destination_port,
                                    data: payload,
                                },
                            );
                        }
                    }
                    Ok(dunit_net_core::ipv6_peer::RxEvent6::Tcp {
                        source,
                        destination,
                        payload,
                    }) => {
                        let _ = self.tcp.receive_ip(
                            IpAddr::V6(source),
                            IpAddr::V6(destination),
                            payload,
                            now,
                        );
                    }
                    Ok(dunit_net_core::ipv6_peer::RxEvent6::None) | Err(_) => {}
                }
            }
            return Ok(());
        }

        if nic.handle_v4_probe_or_control(frame, now)? {
            return Ok(());
        }
        if let Some(peer4) = nic.v4_peer() {
            if let Ok(Some(ip4)) = peer4.ipv4_frame(frame) {
                match ip4.protocol {
                    6 => {
                        let _ = self.tcp.receive_ip(
                            IpAddr::V4(ip4.source),
                            IpAddr::V4(ip4.destination),
                            ip4.payload,
                            now,
                        );
                        return Ok(());
                    }
                    17 => {
                        if let Ok(datagram) = Udp::parse_ip(
                            IpAddr::V4(ip4.source),
                            IpAddr::V4(ip4.destination),
                            ip4.payload,
                        ) {
                            if datagram.source == 53 && datagram.destination == self.dns.port() {
                                self.dns.receive_udp(
                                    IpAddr::V4(ip4.source),
                                    datagram.data,
                                    now,
                                );
                            } else {
                                let _ = self.udp.deliver(IpAddr::V4(ip4.source), datagram);
                            }
                        }
                        return Ok(());
                    }
                    _ => {}
                }
            }
            if let Ok(Some(response)) = peer4.receive(frame, now) {
                let _ = nic.tx.try_push(&response);
            }
        }
        Ok(())
    }

    pub fn step(&mut self, nics: &mut [Box<Nic>], now: u64) -> Result<()> {
        self.sync_nics(nics)?;
        self.read_requests(now)?;
        let mut retry = false;

        for index in 0..SOCKETS {
            let Some(mut socket) = self.sockets[index].take() else {
                continue;
            };
            if socket.failed {
                self.sockets[index] = Some(socket);
                continue;
            }
            if let Some(mut request) = socket.request.take() {
                if now >= request.deadline {
                    retire(&mut socket, &mut self.udp, &mut self.tcp, &mut self.dns, &mut self.wire);
                    fail(&mut self.bridge, &mut socket, errno::EIO)?;
                    self.sockets[index] = Some(socket);
                    continue;
                }
                if request.kind == BRIDGE_ACCEPT {
                    let result = progress_accept(
                        index,
                        &socket,
                        &request,
                        &mut self.sockets,
                        &mut self.tcp,
                        &self.router,
                    );
                    socket.listener_notified = false;
                    match result {
                        Ok((child_slot, remote)) => {
                            self.bridge
                                .complete(socket.open, BRIDGE_ACCEPT, 0, remote)?;
                            if child_slot < index {
                                if let Some(child) = self.sockets[child_slot].as_mut() {
                                    receive(&mut self.bridge, child, &mut self.udp, &mut self.tcp)?;
                                }
                            }
                        }
                        Err(err) => {
                            self.bridge.complete(
                                socket.open,
                                BRIDGE_ACCEPT,
                                err,
                                Endpoint6::NONE,
                            )?;
                        }
                    }
                } else {
                    let status = if socket.open.protocol == PROTO_DNS {
                        if request.kind != BRIDGE_SEND {
                            Err(errno::EOPNOTSUPP)
                        } else if self.router.dns_servers().is_empty() && self.v4_revoked {
                            Err(errno::EPIPE)
                        } else if self.dns.is_busy_for(socket.open.token) {
                            Ok(false)
                        } else {
                            match self.dns.step(
                                socket.open.token,
                                &request.data,
                                &self.router,
                                nics,
                                &mut self.tcp,
                                &mut self.wire,
                                now,
                            ) {
                                Ok(Some(bytes)) => {
                                    socket.rx = Some((Endpoint6::NONE, bytes));
                                    receive(&mut self.bridge, &mut socket, &mut self.udp, &mut self.tcp)?;
                                    Ok(true)
                                }
                                Ok(None) => Ok(false),
                                Err(err) => Err(err),
                            }
                        }
                    } else {
                        progress(
                            &mut socket,
                            &mut request,
                            &self.router,
                            nics,
                            &mut self.udp,
                            &mut self.tcp,
                            &*self.sockets,
                            self.v4_revoked,
                            now,
                        )
                    };
                    match status {
                        Ok(true) => self.bridge.complete(
                            socket.open,
                            request.kind,
                            0,
                            socket.local,
                        )?,
                        Ok(false) => {
                            retry |= request.kind == BRIDGE_SEND
                                || (socket.open.protocol == ABI_UDP
                                    && request.kind == BRIDGE_CONNECT);
                            socket.request = Some(request);
                        }
                        Err(error) => {
                            self.bridge.complete(
                                socket.open,
                                request.kind,
                                error,
                                Endpoint6::NONE,
                            )?;
                            retire(
                                &mut socket,
                                &mut self.udp,
                                &mut self.tcp,
                                &mut self.dns,
                                &mut self.wire,
                            );
                            socket.failed = true;
                        }
                    }
                }
            }
            report_tcp_error(&mut self.bridge, &mut socket, &mut self.tcp, &mut self.wire)?;
            receive(&mut self.bridge, &mut socket, &mut self.udp, &mut self.tcp)?;
            if !socket.failed {
                if let Transport::Listener(id) = socket.transport {
                    let ready = self.tcp.listener_ready(id).map_err(|e| Error(tcp_errno(e)))?;
                    if ready != socket.listener_notified {
                        match self.bridge.write(
                            socket.open,
                            BRIDGE_LISTENER_READY,
                            u16::from(ready),
                            0,
                            Endpoint6::NONE,
                            &[],
                        ) {
                            Ok(()) => socket.listener_notified = ready,
                            Err(Error(errno::EAGAIN)) => retry = true,
                            Err(err) => {
                                self.sockets[index] = Some(socket);
                                return Err(err);
                            }
                        }
                    }
                }
            }
            self.sockets[index] = Some(socket);
        }

        self.wire.retain(|packet| self.tcp.outbound_current_ip(packet));
        for _ in 0..MAX_STREAMS {
            let Some(packet) = self
                .tcp
                .poll_deferred_ip(now)
                .map_err(|e| Error(tcp_errno(e)))?
            else {
                break;
            };
            if let Some(pending) = self.wire.iter_mut().find(|p| packet.same_pending_slot(p)) {
                *pending = packet;
            } else {
                self.wire.push_back(packet);
            }
        }
        for socket in self.sockets.iter_mut().flatten() {
            report_tcp_error(&mut self.bridge, socket, &mut self.tcp, &mut self.wire)?;
        }
        for _ in 0..self.wire.len() {
            let packet = self.wire.pop_front().unwrap();
            if !self.tcp.outbound_current_ip(&packet) {
                continue;
            }
            let (sent, handoff) = send_tcp_packet(&self.router, nics, &packet, now);
            if !sent {
                self.wire.push_back(packet);
            } else {
                self.tcp
                    .complete_egress_ip(&packet, now, handoff)
                    .map_err(|e| Error(tcp_errno(e)))?;
            }
        }
        for socket in self.sockets.iter_mut().flatten() {
            report_tcp_error(&mut self.bridge, socket, &mut self.tcp, &mut self.wire)?;
        }
        retry |= !self.wire.is_empty();
        self.retry_at = retry.then_some(now.saturating_add(RETRY_MS));
        Ok(())
    }

    fn read_requests(&mut self, now: u64) -> Result<()> {
        for _ in 0..(SOCKETS * 2) {
            let size = match checked(libdunit::read(
                self.bridge.fd as usize,
                self.read_buffer.as_mut(),
            )) {
                Ok(size) if size != 0 && size <= MAX_FRAME_V2 => size,
                Err(Error(errno::EAGAIN)) => break,
                Ok(_) => return Err(Error(errno::EIO)),
                Err(error) => return Err(error),
            };
            let (header, payload) =
                BridgeHeader6::decode(&self.read_buffer[..size]).map_err(|_| Error(errno::EIO))?;
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
                            Endpoint6::NONE,
                        )?;
                        continue;
                    };
                    *slot = Some(Socket::new(header));
                    self.bridge
                        .complete(header, BRIDGE_OPEN, 0, Endpoint6::NONE)?;
                }
                (BRIDGE_CLOSE, Some(index)) => {
                    let mut socket = self.sockets[index].take().unwrap();
                    retire(&mut socket, &mut self.udp, &mut self.tcp, &mut self.dns, &mut self.wire);
                }
                (BRIDGE_CREDIT, Some(index)) => self.sockets[index].as_mut().unwrap().credit = true,
                (BRIDGE_CLOSE | BRIDGE_CREDIT, None) => {}
                (
                    BRIDGE_CONNECT | BRIDGE_SEND | BRIDGE_SHUTDOWN | BRIDGE_BIND | BRIDGE_LISTEN
                    | BRIDGE_ACCEPT,
                    Some(index),
                ) => {
                    let socket = self.sockets[index].as_mut().unwrap();
                    if socket.failed {
                    } else if socket.request.is_some() {
                        return Err(Error(errno::EIO));
                    } else {
                        let timeout = if socket.open.protocol == PROTO_DNS {
                            DNS_REQUEST_TIMEOUT_MS
                        } else {
                            REQUEST_TIMEOUT_MS
                        };
                        socket.request = Some(Request {
                            kind: header.kind,
                            destination: header.endpoint,
                            data: payload.to_vec(),
                            offset: 0,
                            deadline: now.saturating_add(timeout),
                        });
                    }
                }
                _ => return Err(Error(errno::EIO)),
            }
        }
        Ok(())
    }
}

fn ep_to_dual(ep: Endpoint6) -> Option<(DualAddress, u32)> {
    match ep.family {
        FAMILY_IPV4 => {
            let ip: [u8; 4] = ep.addr[..4].try_into().ok()?;
            Some((
                DualAddress {
                    ip: IpAddr::V4(ip),
                    port: ep.port,
                },
                0,
            ))
        }
        FAMILY_IPV6 => Some((
            DualAddress {
                ip: IpAddr::V6(ep.addr),
                port: ep.port,
            },
            ep.scope_id,
        )),
        _ => None,
    }
}

fn dual_to_ep(ip: IpAddr, port: u16, scope_id: u32) -> Endpoint6 {
    match ip {
        IpAddr::V4(v4) => Endpoint6::ipv4(v4, port),
        IpAddr::V6(v6) => Endpoint6::ipv6(
            v6,
            port,
            if ipv6::is_link_local(v6) { scope_id } else { 0 },
        ),
    }
}

fn find_route_or_wait(
    router: &MultiRouter,
    nics: &[Box<Nic>],
    dst: IpAddr,
    scope_id: u32,
    v4_revoked: bool,
) -> core::result::Result<Option<RouteChoice>, isize> {
    match router.select(dst, scope_id) {
        Ok(choice) => Ok(Some(choice)),
        Err(NetError::NoRoute) => {
            let (possible, revoked) = match dst {
                IpAddr::V4(_) => (!v4_revoked && nics.iter().any(|n| n.can_ever_v4()), v4_revoked),
                IpAddr::V6(_) => (nics.iter().any(|n| n.can_ever_v6()), false),
            };
            if possible {
                Ok(None)
            } else if revoked || matches!(dst, IpAddr::V4(_)) {
                Err(errno::EPIPE)
            } else {
                Err(errno::EIO)
            }
        }
        Err(e) => Err(net_errno(e)),
    }
}

fn send_tcp_packet(
    router: &MultiRouter,
    nics: &mut [Box<Nic>],
    packet: &DualOutbound,
    now: u64,
) -> (bool, core::result::Result<(), NetError>) {
    let Some(owner_nic_idx) = nics
        .iter()
        .position(|n| n.link && n.view().owns_ip(packet.source.ip))
    else {
        return (true, Err(NetError::NoRoute));
    };
    let scope = (nics[owner_nic_idx].id + 1) as u32;
    let route = match packet.destination.ip {
        IpAddr::V4(_) => router.select(packet.destination.ip, 0),
        IpAddr::V6(_) => router.select(packet.destination.ip, scope),
    };
    let Ok(choice) = route else {
        return (true, Err(NetError::NoRoute));
    };
    let Some(nic) = nics.iter_mut().find(|n| n.id == choice.iface_id) else {
        return (true, Err(NetError::NoRoute));
    };
    match (packet.source.ip, packet.destination.ip, choice.next_hop) {
        (IpAddr::V4(_), IpAddr::V4(dst4), _) => {
            let Some(peer4) = nic.v4_peer() else {
                return (true, Err(NetError::NoRoute));
            };
            match peer4.route_egress(dst4, 6, &packet.bytes, now) {
                Ok(Egress::Sent(frame)) => (nic.tx.try_push(&frame).unwrap_or(false), Ok(())),
                Ok(Egress::Neighbor(frame)) => {
                    let _ = nic.tx.push(&frame);
                    (false, Ok(()))
                }
                Ok(Egress::Waiting) => (false, Ok(())),
                Err(err) => (true, Err(err)),
            }
        }
        (IpAddr::V6(src6), IpAddr::V6(dst6), IpAddr::V6(nh6)) => {
            let Some(peer6) = &mut nic.v6 else {
                return (true, Err(NetError::NoRoute));
            };
            match peer6.send_packet(src6, dst6, nh6, PROTO_TCP, &packet.bytes, now) {
                Ok(Egress6::Sent(frame)) => (nic.tx.try_push(&frame).unwrap_or(false), Ok(())),
                Ok(Egress6::Neighbor(frame)) => {
                    let _ = nic.tx.try_push(&frame);
                    (true, Ok(()))
                }
                Ok(Egress6::Queued) => (true, Ok(())),
                Err(err) => (true, Err(err)),
            }
        }
        _ => (true, Err(NetError::Address)),
    }
}

fn tcp_port_pending(sockets: &[Option<Socket>], port: u16) -> bool {
    sockets
        .iter()
        .flatten()
        .any(|s| matches!(s.transport, Transport::TcpBound(p, _) if p == port))
}

fn progress_accept(
    listener_index: usize,
    listener: &Socket,
    request: &Request,
    sockets: &mut [Option<Socket>; SOCKETS],
    tcp: &mut Table,
    router: &MultiRouter,
) -> core::result::Result<(usize, Endpoint6), isize> {
    if listener.open.protocol != ABI_TCP {
        return Err(errno::EOPNOTSUPP);
    }
    let Transport::Listener(id) = listener.transport else {
        return Err(errno::EINVAL);
    };
    let raw: [u8; 8] = request
        .data
        .as_slice()
        .try_into()
        .map_err(|_| errno::EINVAL)?;
    let child_token = u64::from_le_bytes(raw);
    let slot = sockets
        .iter()
        .enumerate()
        .position(|(i, s)| i != listener_index && s.is_none())
        .ok_or(errno::ENOBUFS)?;
    let stream_id = tcp.accept(id).map_err(tcp_errno)?;
    let (local, remote) = match tcp.endpoints_ip(stream_id) {
        Ok(endpoints) => endpoints,
        Err(err) => {
            let _ = tcp.close(stream_id);
            return Err(tcp_errno(err));
        }
    };
    let scope_id = match router.select(remote.ip, 0) {
        Ok(r) => (r.iface_id + 1) as u32,
        Err(_) => 0,
    };
    let mut child = Socket::new(BridgeHeader6 {
        token: child_token,
        owner: listener.open.owner,
        kind: BRIDGE_OPEN,
        flags: 0,
        error: 0,
        endpoint: Endpoint6::NONE,
        protocol: ABI_TCP,
        length: 0,
    });
    child.transport = Transport::Tcp(stream_id);
    child.local = dual_to_ep(local.ip, local.port, scope_id);
    sockets[slot] = Some(child);
    Ok((slot, dual_to_ep(remote.ip, remote.port, scope_id)))
}

#[allow(clippy::too_many_arguments)]
fn progress(
    socket: &mut Socket,
    request: &mut Request,
    router: &MultiRouter,
    nics: &mut [Box<Nic>],
    udp: &mut DualEndpoints,
    tcp: &mut Table,
    others: &[Option<Socket>],
    v4_revoked: bool,
    now: u64,
) -> core::result::Result<bool, isize> {
    let remote = request.destination;
    match (socket.open.protocol, request.kind) {
        (ABI_UDP, BRIDGE_BIND) => {
            if !matches!(socket.transport, Transport::None) {
                return Err(errno::EINVAL);
            }
            let bind_ep = match remote.family {
                FAMILY_IPV4 => {
                    let Some((_, IpAddr::V4(v4))) = router.primary_ip(false) else {
                        return if !v4_revoked && nics.iter().any(|n| n.can_ever_v4()) {
                            Ok(false)
                        } else {
                            Err(errno::EPIPE)
                        };
                    };
                    let req_v4 = &remote.addr[..4];
                    if req_v4 != [0; 4] && !nics.iter().any(|n| n.view().ipv4.is_some_and(|(ip, _)| ip == req_v4)) {
                        return Err(errno::EINVAL);
                    }
                    let chosen = if req_v4 == [0; 4] {
                        v4
                    } else {
                        req_v4.try_into().unwrap()
                    };
                    Endpoint6::ipv4(chosen, remote.port)
                }
                FAMILY_IPV6 => {
                    let Some((iface, prim)) = router.primary_ip(true) else {
                        return if nics.iter().any(|n| n.can_ever_v6()) {
                            Ok(false)
                        } else {
                            Err(errno::EIO)
                        };
                    };
                    if remote.addr != [0; 16] && !nics.iter().any(|n| n.view().owns_ip(IpAddr::V6(remote.addr))) {
                        return Err(errno::EINVAL);
                    }
                    if remote.addr == [0; 16] {
                        dual_to_ep(prim, remote.port, (iface + 1) as u32)
                    } else {
                        remote
                    }
                }
                _ => return Err(errno::EINVAL),
            };
            let port = udp.bind(bind_ep.port).map_err(net_errno)?;
            socket.transport = Transport::Udp(port, None);
            socket.local = Endpoint6 { port, ..bind_ep };
            Ok(true)
        }
        (ABI_UDP, BRIDGE_CONNECT) => {
            let Some((dst, scope_id)) = ep_to_dual(remote) else {
                return Err(errno::EINVAL);
            };
            let Some(choice) = find_route_or_wait(router, nics, dst.ip, scope_id, v4_revoked)? else {
                return Ok(false);
            };
            let local_ep = dual_to_ep(choice.source, 0, (choice.iface_id + 1) as u32);
            let port = socket.bound_udp(local_ep, udp)?;
            udp.connect(port, Some(dst)).map_err(net_errno)?;
            socket.transport = Transport::Udp(port, Some((dst, scope_id)));
            Ok(true)
        }
        (ABI_UDP, BRIDGE_SEND) => {
            let (dst, scope_id) = if remote.family != FAMILY_NONE {
                ep_to_dual(remote).ok_or(errno::EINVAL)?
            } else if let Transport::Udp(_, Some(saved)) = socket.transport {
                saved
            } else {
                return Err(errno::EINVAL);
            };
            let Some(choice) = find_route_or_wait(router, nics, dst.ip, scope_id, v4_revoked)? else {
                return Ok(false);
            };
            let local_ep = dual_to_ep(choice.source, 0, (choice.iface_id + 1) as u32);
            let port = socket.bound_udp(local_ep, udp)?;
            let payload = Udp::encode_ip(
                DualAddress {
                    ip: choice.source,
                    port,
                },
                dst,
                &request.data,
            )
            .map_err(net_errno)?;
            let nic = nics
                .iter_mut()
                .find(|n| n.id == choice.iface_id)
                .ok_or(errno::EIO)?;
            match (choice.source, dst.ip, choice.next_hop) {
                (IpAddr::V4(_), IpAddr::V4(dst4), _) => {
                    let peer4 = nic.v4_peer().ok_or(errno::EIO)?;
                    match peer4.route_egress(dst4, 17, &payload, now).map_err(net_errno)? {
                        Egress::Sent(frame) => nic.tx.try_push(&frame).map_err(|e| e.0),
                        Egress::Neighbor(frame) => {
                            nic.tx.push(&frame).map_err(|e| e.0)?;
                            Ok(false)
                        }
                        Egress::Waiting => Ok(false),
                    }
                }
                (IpAddr::V6(src6), IpAddr::V6(dst6), IpAddr::V6(nh6)) => {
                    let peer6 = nic.v6.as_mut().ok_or(errno::EIO)?;
                    match peer6
                        .send_packet(src6, dst6, nh6, V6_PROTO_UDP, &payload, now)
                        .map_err(net_errno)?
                    {
                        Egress6::Sent(frame) => nic.tx.try_push(&frame).map_err(|e| e.0),
                        Egress6::Neighbor(frame) => {
                            let _ = nic.tx.try_push(&frame);
                            Ok(true)
                        }
                        Egress6::Queued => Ok(true),
                    }
                }
                _ => Err(errno::EINVAL),
            }
        }
        (ABI_TCP, BRIDGE_BIND) => {
            if !matches!(socket.transport, Transport::None) {
                return Err(errno::EINVAL);
            }
            let bind_ep = match remote.family {
                FAMILY_IPV4 => {
                    let Some((_, IpAddr::V4(v4))) = router.primary_ip(false) else {
                        return if !v4_revoked && nics.iter().any(|n| n.can_ever_v4()) {
                            Ok(false)
                        } else {
                            Err(errno::EPIPE)
                        };
                    };
                    let req_v4 = &remote.addr[..4];
                    if req_v4 != [0; 4] && !nics.iter().any(|n| n.view().ipv4.is_some_and(|(ip, _)| ip == req_v4)) {
                        return Err(errno::EINVAL);
                    }
                    let chosen = if req_v4 == [0; 4] {
                        v4
                    } else {
                        req_v4.try_into().unwrap()
                    };
                    Endpoint6::ipv4(chosen, remote.port)
                }
                FAMILY_IPV6 => {
                    if remote.addr != [0; 16] {
                        if !nics.iter().any(|n| n.view().owns_ip(IpAddr::V6(remote.addr))) {
                            return Err(errno::EINVAL);
                        }
                        remote
                    } else if let Some((iface, prim)) = router.primary_ip(true) {
                        dual_to_ep(prim, remote.port, (iface + 1) as u32)
                    } else {
                        Endpoint6::ipv6([0; 16], remote.port, 0)
                    }
                }
                _ => return Err(errno::EINVAL),
            };
            let port = if bind_ep.port == 0 {
                let mut chosen = 0;
                for _ in 0..=SOCKETS {
                    let candidate = tcp.ephemeral_port().map_err(tcp_errno)?;
                    if !tcp_port_pending(others, candidate) {
                        chosen = candidate;
                        break;
                    }
                }
                if chosen == 0 {
                    return Err(errno::ENOBUFS);
                }
                chosen
            } else {
                if tcp.port_in_use(bind_ep.port) || tcp_port_pending(others, bind_ep.port) {
                    return Err(errno::EBUSY);
                }
                bind_ep.port
            };
            let final_ep = Endpoint6 { port, ..bind_ep };
            socket.transport = Transport::TcpBound(port, remote);
            socket.local = final_ep;
            Ok(true)
        }
        (ABI_TCP, BRIDGE_LISTEN) => {
            let Transport::TcpBound(port, bind_target) = socket.transport else {
                return Err(errno::EINVAL);
            };
            let raw: [u8; 4] = request
                .data
                .as_slice()
                .try_into()
                .map_err(|_| errno::EINVAL)?;
            let backlog = (u32::from_le_bytes(raw) as usize).clamp(1, MAX_PENDING);
            let family = match bind_target.family {
                FAMILY_IPV4 => Some(4),
                FAMILY_IPV6 if bind_target.addr != [0; 16] => Some(6),
                FAMILY_IPV6 => None, // [::] accepts both IPv6 and IPv4 cleanly.
                _ => None,
            };
            let id = tcp.listen_family(port, family, backlog).map_err(tcp_errno)?;
            socket.transport = Transport::Listener(id);
            Ok(true)
        }
        (ABI_TCP, kind) => {
            let id = match socket.transport {
                Transport::Tcp(id) => id,
                Transport::None if kind == BRIDGE_CONNECT => {
                    let Some((dst, scope_id)) = ep_to_dual(remote) else {
                        return Err(errno::EINVAL);
                    };
                    let Some(choice) = find_route_or_wait(router, nics, dst.ip, scope_id, v4_revoked)? else {
                        return Ok(false);
                    };
                    let id = tcp
                        .connect_ip(
                            choice.source,
                            DualEndpoint {
                                ip: dst.ip,
                                port: dst.port,
                            },
                            now,
                        )
                        .map_err(tcp_errno)?;
                    let (local, _) = tcp.endpoints_ip(id).map_err(tcp_errno)?;
                    socket.local = dual_to_ep(local.ip, local.port, (choice.iface_id + 1) as u32);
                    socket.transport = Transport::Tcp(id);
                    id
                }
                _ => return Err(errno::EINVAL),
            };
            match kind {
                BRIDGE_CONNECT => match tcp.state(id).map_err(tcp_errno)? {
                    State::Established | State::CloseWait => Ok(true),
                    State::Closed => Err(tcp
                        .failure(id)
                        .map_err(tcp_errno)?
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

fn report_tcp_error(
    bridge: &mut Bridge,
    socket: &mut Socket,
    tcp: &mut Table,
    wire: &mut VecDeque<DualOutbound>,
) -> Result<()> {
    let Transport::Tcp(id) = socket.transport else {
        return Ok(());
    };
    if let Some(error) = tcp.failure(id).map_err(|e| Error(tcp_errno(e)))? {
        fail(bridge, socket, tcp_errno(error))?;
        retire_tcp_only(socket, tcp, wire);
    }
    Ok(())
}

fn retire_tcp_only(
    socket: &mut Socket,
    tcp: &mut Table,
    wire: &mut VecDeque<DualOutbound>,
) {
    if let Transport::Tcp(id) = socket.transport {
        if let Ok((src, dst)) = tcp.endpoints_ip(id) {
            wire.retain(|p| (p.source, p.destination) != (src, dst));
        }
        let _ = tcp.close(id);
        socket.transport = Transport::None;
    }
}

fn receive(
    bridge: &mut Bridge,
    socket: &mut Socket,
    udp: &mut DualEndpoints,
    tcp: &mut Table,
) -> Result<()> {
    if socket.failed {
        return Ok(());
    }
    if socket.rx.is_none() && socket.credit && !socket.eof {
        match socket.transport {
            Transport::Udp(port, _) => {
                let mut data = alloc::vec![0; MAX_PAYLOAD];
                while let Ok(packet) = udp.recv_into(port, &mut data) {
                    let source = dual_to_ep(packet.source.ip, packet.source.port, 0);
                    if source.is_unicast() {
                        data.truncate(packet.copied);
                        socket.rx = Some((source, data));
                        break;
                    }
                }
            }
            Transport::Tcp(id) => {
                let ready = tcp.readiness(id).map_err(|e| Error(tcp_errno(e)))?;
                if ready.readable {
                    let mut data = alloc::vec![0; MAX_PAYLOAD];
                    match tcp.read(id, &mut data) {
                        Ok(0) => {
                            bridge.write(socket.open, BRIDGE_EOF, 0, 0, Endpoint6::NONE, &[])?;
                            socket.eof = true;
                        }
                        Ok(count) => {
                            data.truncate(count);
                            socket.rx = Some((Endpoint6::NONE, data));
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
    udp: &mut DualEndpoints,
    tcp: &mut Table,
    dns: &mut DualDnsRuntime,
    wire: &mut VecDeque<DualOutbound>,
) {
    if socket.open.protocol == PROTO_DNS {
        dns.cancel(socket.open.token, tcp, wire);
        return;
    }
    match socket.transport {
        Transport::Udp(port, _) => {
            let _ = udp.close(port);
        }
        Transport::Tcp(id) => {
            if let Ok((src, dst)) = tcp.endpoints_ip(id) {
                wire.retain(|p| (p.source, p.destination) != (src, dst));
            }
            let _ = tcp.close(id);
        }
        Transport::Listener(id) => {
            let _ = tcp.close_listener(id);
        }
        _ => {}
    }
    socket.transport = Transport::None;
}

fn fail(bridge: &mut Bridge, socket: &mut Socket, error: isize) -> Result<()> {
    bridge.write(socket.open, BRIDGE_ERROR, 0, error, Endpoint6::NONE, &[])?;
    socket.failed = true;
    socket.request = None;
    socket.rx = None;
    Ok(())
}

fn net_errno(error: NetError) -> isize {
    match error {
        NetError::Length => errno::EMSGSIZE,
        NetError::Capacity => errno::ENOBUFS,
        NetError::PortInUse => errno::EBUSY,
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
