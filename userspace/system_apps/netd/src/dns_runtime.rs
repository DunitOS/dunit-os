//! Application DNS transaction runner backed by `dunit_net_core::dns::Resolver`.
use alloc::vec::Vec;
use dunit::errno;
use dunit_net_core::{
    dns::{AddressData, Answer, Kind, Poll, Resolver},
    tcp::{
        self,
        table::{Outbound as TcpPacket, StreamId, Table},
        State, StreamError,
    },
    udp::Address,
    Error as NetError, Ip, Peer,
};
use alloc::collections::VecDeque;

use crate::outbound::Outbound;

const DNS_PORT: u16 = 53;
const POLL_STEP_MS: u64 = 50;

struct TcpFallback {
    id: StreamId,
    server: Address,
    query: Vec<u8>,
    offset: usize,
    deadline_ms: u64,
}

struct ActiveQuery {
    token: u64,
    tcp: Option<TcpFallback>,
}

pub(crate) struct DnsRuntime {
    resolver: Resolver,
    active: Option<ActiveQuery>,
    next_poll: Option<u64>,
}

impl DnsRuntime {
    pub(crate) fn open(peer: &mut Peer, servers: [Option<Ip>; 2]) -> core::result::Result<Option<Self>, isize> {
        let Some(primary_ip) = servers[0] else {
            return Ok(None);
        };
        let primary = Address {
            ip: primary_ip,
            port: DNS_PORT,
        };
        let resolver = match servers[1] {
            Some(secondary_ip) => Resolver::with_secondary(
                peer,
                primary,
                Address {
                    ip: secondary_ip,
                    port: DNS_PORT,
                },
                true,
            ),
            None => Resolver::new_with_tcp(peer, primary),
        }
        .map_err(dns_errno)?;
        Ok(Some(Self {
            resolver,
            active: None,
            next_poll: None,
        }))
    }

    pub(crate) fn deadline(&self) -> Option<u64> {
        let tcp_deadline = self
            .active
            .as_ref()
            .and_then(|q| q.tcp.as_ref().map(|t| t.deadline_ms));
        match (self.next_poll, tcp_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub(crate) fn is_active_for(&self, token: u64) -> bool {
        self.active.as_ref().is_some_and(|q| q.token == token)
    }

    pub(crate) fn is_busy_for(&self, token: u64) -> bool {
        self.active.as_ref().is_some_and(|q| q.token != token)
    }

    pub(crate) fn cancel(
        &mut self,
        token: u64,
        tcp: Option<&mut Table>,
        wire: &mut VecDeque<TcpPacket>,
    ) {
        if !self.is_active_for(token) {
            return;
        }
        if let Some(active) = self.active.take() {
            if let (Some(fb), Some(tcp)) = (active.tcp, tcp) {
                if let Ok((src, dst)) = tcp.endpoints(fb.id) {
                    wire.retain(|p| (p.source, p.destination) != (src, dst));
                }
                let _ = tcp.close(fb.id);
            }
            self.resolver.abort();
        }
        self.next_poll = None;
    }

    pub(crate) fn start(
        &mut self,
        token: u64,
        name: &[u8],
        now: u64,
    ) -> core::result::Result<Option<Ip>, isize> {
        if self.active.is_some() {
            return Err(errno::EAGAIN);
        }
        match self.resolver.start(name, Kind::A, now) {
            Ok(Some(Answer {
                address: AddressData::V4(ip),
                ..
            })) => Ok(Some(ip)),
            Ok(Some(_)) => Err(errno::ENOENT),
            Ok(None) => {
                self.active = Some(ActiveQuery { token, tcp: None });
                self.next_poll = Some(now);
                Ok(None)
            }
            Err(error) => Err(dns_errno(error)),
        }
    }

    pub(crate) fn drive(
        &mut self,
        token: u64,
        peer: &mut Peer,
        tcp: &mut Table,
        tx: &mut Outbound,
        wire: &mut VecDeque<TcpPacket>,
        now: u64,
    ) -> core::result::Result<Option<Ip>, isize> {
        if !self.is_active_for(token) {
            return Err(errno::EINVAL);
        }

        if let Some(mut fb) = self.active.as_mut().and_then(|q| q.tcp.take()) {
            if now >= fb.deadline_ms {
                let _ = tcp.close(fb.id);
                self.cancel(token, Some(tcp), wire);
                return Err(errno::EAGAIN);
            }
            match tcp.state(fb.id).map_err(|_| errno::EIO)? {
                State::Established | State::CloseWait => {
                    if fb.offset < fb.query.len() {
                        match tcp.send(fb.id, &fb.query[fb.offset..]) {
                            Ok(n) => fb.offset += n,
                            Err(StreamError::WouldBlock) => {}
                            Err(_) => {
                                let _ = tcp.close(fb.id);
                                self.cancel(token, Some(tcp), wire);
                                return Err(errno::EIO);
                            }
                        }
                    }
                    if fb.offset == fb.query.len() {
                        let mut buf = [0u8; 512];
                        match tcp.read(fb.id, &mut buf) {
                            Ok(0) => {
                                let _ = self.resolver.tcp_eof(fb.server, now);
                                let _ = tcp.close(fb.id);
                                self.cancel(token, Some(tcp), wire);
                                return Err(errno::EIO);
                            }
                            Ok(n) => match self.resolver.receive_tcp(fb.server, &buf[..n], now) {
                                Ok(Some(Answer {
                                    address: AddressData::V4(ip),
                                    ..
                                })) => {
                                    let _ = tcp.close(fb.id);
                                    self.active = None;
                                    self.next_poll = None;
                                    return Ok(Some(ip));
                                }
                                Ok(Some(_)) => {
                                    let _ = tcp.close(fb.id);
                                    self.active = None;
                                    self.next_poll = None;
                                    return Err(errno::ENOENT);
                                }
                                Ok(None) => {}
                                Err(err) => {
                                    let _ = tcp.close(fb.id);
                                    self.cancel(token, Some(tcp), wire);
                                    return Err(dns_errno(err));
                                }
                            },
                            Err(StreamError::WouldBlock) => {}
                            Err(_) => {
                                let _ = tcp.close(fb.id);
                                self.cancel(token, Some(tcp), wire);
                                return Err(errno::EIO);
                            }
                        }
                    }
                }
                State::Closed => {
                    let _ = self.resolver.tcp_eof(fb.server, now);
                    let _ = tcp.close(fb.id);
                    self.cancel(token, Some(tcp), wire);
                    return Err(errno::EIO);
                }
                _ => {}
            }
            if let Some(active) = &mut self.active {
                active.tcp = Some(fb);
            }
            self.next_poll = Some(now.saturating_add(POLL_STEP_MS));
            return Ok(None);
        }

        loop {
            match self.resolver.poll(peer, now) {
                Ok(Poll::Frame(frame)) => {
                    tx.push(&frame).map_err(|e| e.0)?;
                    self.next_poll = Some(now.saturating_add(POLL_STEP_MS));
                    return Ok(None);
                }
                Ok(Poll::Waiting) => {
                    self.next_poll = Some(now.saturating_add(POLL_STEP_MS));
                    return Ok(None);
                }
                Ok(Poll::Answer(Answer {
                    address: AddressData::V4(ip),
                    ..
                })) => {
                    self.active = None;
                    self.next_poll = None;
                    return Ok(Some(ip));
                }
                Ok(Poll::Answer(_)) => {
                    self.active = None;
                    self.next_poll = None;
                    return Err(errno::ENOENT);
                }
                Ok(Poll::TcpFallback {
                    server,
                    query,
                    deadline_ms,
                }) => {
                    let id = tcp
                        .connect(
                            tcp::Endpoint {
                                ip: server.ip,
                                port: server.port,
                            },
                            now,
                        )
                        .map_err(|_| errno::EIO)?;
                    if let Some(active) = &mut self.active {
                        active.tcp = Some(TcpFallback {
                            id,
                            server,
                            query,
                            offset: 0,
                            deadline_ms,
                        });
                    }
                    self.next_poll = Some(now.saturating_add(POLL_STEP_MS));
                    return Ok(None);
                }
                Err(NetError::Unsolicited | NetError::Header | NetError::Length) => continue,
                Err(err) => {
                    self.cancel(token, Some(tcp), wire);
                    return Err(dns_errno(err));
                }
            }
        }
    }
}

pub(crate) fn dns_errno(error: NetError) -> isize {
    match error {
        NetError::NameNotFound => errno::ENOENT,
        NetError::Header => errno::EINVAL,
        NetError::Timeout | NetError::NoRoute => errno::EAGAIN,
        NetError::WouldBlock => errno::EAGAIN,
        _ => errno::EIO,
    }
}
