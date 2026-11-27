//! Dual-stack (AAAA + A) DNS transaction runner across multiple NICs (UDP + TCP fallback).
use alloc::{collections::VecDeque, vec::Vec};
use dunit::errno;
use dunit_net_core::{
    dns::{
        dual::{DualPoll, DualResolver},
        Kind, TCP_DNS_LIMIT,
    },
    ipv6::PROTO_UDP,
    ipv6_peer::Egress6,
    router::MultiRouter,
    tcp::{
        self,
        table::{DualOutbound, StreamId, Table},
        DualEndpoint, State, StreamError,
    },
    udp::{DualAddress, DualEndpoints, Udp},
    Egress, IpAddr,
};
use dunit_socket_abi::{encode_dns_records, DnsRecord, FAMILY_IPV6, MAX_DNS_RECORDS};

use crate::nic::Nic;

const DNS_PORT: u16 = 53;
const POLL_STEP_MS: u64 = 50;

struct TcpFallback {
    id: StreamId,
    server: IpAddr,
    kind: Kind,
    query: Vec<u8>,
    offset: usize,
    deadline_ms: u64,
}

pub(crate) struct DualDnsRuntime {
    resolver: DualResolver,
    port: u16,
    active_token: Option<u64>,
    tcp: Option<TcpFallback>,
    next_poll: Option<u64>,
}

impl DualDnsRuntime {
    pub(crate) fn open(udp: &mut DualEndpoints, seed: u32) -> core::result::Result<Self, isize> {
        let port = udp.bind(0).map_err(|_| errno::ENOBUFS)?;
        let mut resolver = DualResolver::new();
        resolver.seed_id(((seed >> 16) ^ seed) as u16);
        Ok(Self {
            resolver,
            port,
            active_token: None,
            tcp: None,
            next_poll: None,
        })
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn deadline(&self) -> Option<u64> {
        let tcp_deadline = self.tcp.as_ref().map(|t| t.deadline_ms);
        match (self.next_poll, tcp_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub(crate) fn is_busy_for(&self, token: u64) -> bool {
        self.active_token.is_some_and(|t| t != token)
    }

    pub(crate) fn cancel(
        &mut self,
        token: u64,
        tcp: &mut Table,
        wire: &mut VecDeque<DualOutbound>,
    ) {
        if self.active_token == Some(token) {
            self.retire_tcp(tcp, wire);
            self.resolver.abort();
            self.active_token = None;
            self.next_poll = None;
        }
    }

    fn retire_tcp(&mut self, tcp: &mut Table, wire: &mut VecDeque<DualOutbound>) {
        if let Some(fb) = self.tcp.take() {
            if let Ok((src, dst)) = tcp.endpoints_ip(fb.id) {
                wire.retain(|p| (p.source, p.destination) != (src, dst));
            }
            let _ = tcp.close(fb.id);
        }
    }

    pub(crate) fn receive_udp(&mut self, server: IpAddr, payload: &[u8], now: u64) {
        let _ = self.resolver.receive_udp(server, payload, now);
    }

    pub(crate) fn step(
        &mut self,
        token: u64,
        name: &[u8],
        router: &MultiRouter,
        nics: &mut [alloc::boxed::Box<Nic>],
        tcp: &mut Table,
        wire: &mut VecDeque<DualOutbound>,
        now: u64,
    ) -> core::result::Result<Option<Vec<u8>>, isize> {
        let servers: Vec<IpAddr> = router.dns_servers().into_iter().map(|c| c.server).collect();
        if servers.is_empty() {
            return Ok(None);
        }
        let want_v6 = router.has_routable_v6();
        let want_v4 = router.has_usable_v4();

        if self.active_token.is_none() {
            match self.resolver.start(name, &servers, want_v6, want_v4, now) {
                Ok(Some(ips)) => return Ok(Some(encode_result(&ips)?)),
                Ok(None) => {
                    self.active_token = Some(token);
                }
                Err(err) => return Err(map_dns_err(err)),
            }
        } else if self.active_token != Some(token) {
            return Ok(None);
        }

        if let Some(mut fb) = self.tcp.take() {
            match self.step_tcp(&mut fb, tcp, now) {
                Ok(()) => {
                    self.tcp = Some(fb);
                }
                Err(err) => {
                    if let Ok((src, dst)) = tcp.endpoints_ip(fb.id) {
                        wire.retain(|p| (p.source, p.destination) != (src, dst));
                    }
                    let _ = tcp.close(fb.id);
                    if err != errno::ENOENT {
                        self.resolver.tcp_eof(fb.kind);
                    }
                }
            }
        }

        loop {
            match self.resolver.poll(now) {
                Ok(DualPoll::SendUdp { server, payload }) => {
                    let route = router.select(server, 0).map_err(|_| errno::EIO)?;
                    let udp = Udp::encode_ip(
                        DualAddress {
                            ip: route.source,
                            port: self.port,
                        },
                        DualAddress {
                            ip: server,
                            port: DNS_PORT,
                        },
                        &payload,
                    )
                    .map_err(|_| errno::EINVAL)?;
                    let nic = nics
                        .iter_mut()
                        .find(|n| n.id == route.iface_id)
                        .ok_or(errno::EIO)?;
                    match (route.source, server, route.next_hop) {
                        (IpAddr::V4(_), IpAddr::V4(dst4), _) => {
                            if let Some(peer4) = nic.v4_peer() {
                                match peer4.route_egress(dst4, 17, &udp, now) {
                                    Ok(Egress::Sent(f) | Egress::Neighbor(f)) => {
                                        let _ = nic.tx.try_push(&f);
                                    }
                                    Ok(Egress::Waiting) => {}
                                    Err(e) => return Err(map_dns_err(e)),
                                }
                            }
                        }
                        (IpAddr::V6(src6), IpAddr::V6(dst6), IpAddr::V6(nh6)) => {
                            if let Some(peer6) = &mut nic.v6 {
                                match peer6.send_packet(src6, dst6, nh6, PROTO_UDP, &udp, now) {
                                    Ok(Egress6::Sent(f) | Egress6::Neighbor(f)) => {
                                        let _ = nic.tx.try_push(&f);
                                    }
                                    Ok(Egress6::Queued) => {}
                                    Err(e) => return Err(map_dns_err(e)),
                                }
                            }
                        }
                        _ => return Err(errno::EINVAL),
                    }
                    self.next_poll = Some(now.saturating_add(POLL_STEP_MS));
                }
                Ok(DualPoll::TcpFallback {
                    server,
                    kind,
                    query,
                    deadline_ms,
                }) => {
                    let route = router.select(server, 0).map_err(|_| errno::EIO)?;
                    let id = tcp
                        .connect_ip(
                            route.source,
                            DualEndpoint {
                                ip: server,
                                port: DNS_PORT,
                            },
                            now,
                        )
                        .map_err(tcp_errno)?;
                    self.tcp = Some(TcpFallback {
                        id,
                        server,
                        kind,
                        query,
                        offset: 0,
                        deadline_ms,
                    });
                    self.next_poll = Some(now.saturating_add(POLL_STEP_MS));
                    return Ok(None);
                }
                Ok(DualPoll::Wait(deadline)) => {
                    self.next_poll = deadline
                        .map(|d| d.min(now.saturating_add(POLL_STEP_MS)))
                        .or_else(|| Some(now.saturating_add(POLL_STEP_MS)));
                    return Ok(None);
                }
                Ok(DualPoll::Resolved(ips)) => {
                    self.retire_tcp(tcp, wire);
                    self.active_token = None;
                    self.next_poll = None;
                    return Ok(Some(encode_result(&ips)?));
                }
                Err(err) => {
                    self.retire_tcp(tcp, wire);
                    self.active_token = None;
                    self.next_poll = None;
                    return Err(map_dns_err(err));
                }
            }
        }
    }

    fn step_tcp(
        &mut self,
        fb: &mut TcpFallback,
        tcp: &mut Table,
        now: u64,
    ) -> core::result::Result<(), isize> {
        if now >= fb.deadline_ms {
            return Err(errno::EIO);
        }
        if let Some(err) = tcp.failure(fb.id).map_err(tcp_errno)? {
            return Err(tcp_errno(err));
        }
        let state = tcp.state(fb.id).map_err(tcp_errno)?;
        if matches!(state, State::Established | State::CloseWait) && fb.offset < fb.query.len() {
            match tcp.send(fb.id, &fb.query[fb.offset..]) {
                Ok(n) => fb.offset += n,
                Err(StreamError::WouldBlock) => {}
                Err(e) => return Err(tcp_errno(e)),
            }
        }
        let mut buf = [0u8; TCP_DNS_LIMIT + 2];
        loop {
            match tcp.read(fb.id, &mut buf) {
                Ok(0) => {
                    let _ = self.resolver.tcp_eof(fb.kind);
                    return Err(errno::ENOENT);
                }
                Ok(n) => {
                    self.resolver
                        .receive_tcp(fb.server, &buf[..n], fb.kind, now)
                        .map_err(map_dns_err)?;
                }
                Err(StreamError::WouldBlock) => return Ok(()),
                Err(e) => return Err(tcp_errno(e)),
            }
        }
    }
}

fn encode_result(ips: &[IpAddr]) -> core::result::Result<Vec<u8>, isize> {
    let mut recs = Vec::with_capacity(ips.len().min(MAX_DNS_RECORDS));
    for &ip in ips.iter().take(MAX_DNS_RECORDS) {
        let r = match ip {
            IpAddr::V4(v4) => DnsRecord::ipv4(v4),
            IpAddr::V6(v6) => DnsRecord {
                family: FAMILY_IPV6,
                scope_id: 0,
                addr: v6,
            },
        };
        if r.is_valid() {
            recs.push(r);
        }
    }
    if recs.is_empty() {
        return Err(errno::ENOENT);
    }
    let mut out = alloc::vec![0; 2 + recs.len() * dunit_socket_abi::DNS_RECORD_LEN];
    let n = encode_dns_records(&recs, &mut out).map_err(|_| errno::EINVAL)?;
    out.truncate(n);
    Ok(out)
}

fn map_dns_err(err: dunit_net_core::Error) -> isize {
    match err {
        dunit_net_core::Error::NameNotFound => errno::ENOENT,
        dunit_net_core::Error::Capacity => errno::ENOBUFS,
        dunit_net_core::Error::Unsupported => errno::EOPNOTSUPP,
        _ => errno::EIO,
    }
}

fn tcp_errno(error: tcp::StreamError) -> isize {
    match error {
        StreamError::Closed | StreamError::Reset => errno::EPIPE,
        StreamError::WouldBlock => errno::EAGAIN,
        _ => errno::EIO,
    }
}
