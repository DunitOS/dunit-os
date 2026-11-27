//! Dual-stack (AAAA + A) DNS resolver over arbitrary IPv4/IPv6 servers.
use super::{
    normalize, parse_message_inner, stream, AddressData, Kind, Question, Response, ATTEMPTS,
    DNS_LIMIT, RETRY_MS, TCP_DNS_LIMIT, TCP_TIMEOUT_MS,
};
use crate::{Error, IpAddr};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FamilyState {
    Disabled,
    Pending {
        server_idx: usize,
        attempts: u8,
        next_ms: u64,
        id: u16,
    },
    Tcp {
        server_idx: usize,
        id: u16,
        sent: bool,
        deadline_ms: u64,
    },
    Done,
    Failed,
}

#[derive(Clone, Debug)]
struct DualCacheEntry {
    name: Vec<u8>,
    kind: Kind,
    answers: Vec<IpAddr>,
    negative: bool,
    expires: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DualPoll {
    SendUdp { server: IpAddr, payload: Vec<u8> },
    TcpFallback {
        server: IpAddr,
        kind: Kind,
        query: Vec<u8>,
        deadline_ms: u64,
    },
    Wait(Option<u64>),
    Resolved(Vec<IpAddr>),
}

pub struct DualResolver {
    servers: Vec<IpAddr>,
    name: Vec<u8>,
    v6_state: FamilyState,
    v4_state: FamilyState,
    v6_results: Vec<IpAddr>,
    v4_results: Vec<IpAddr>,
    active: bool,
    cache: Vec<DualCacheEntry>,
    last_ms: u64,
    next_id: u16,
    tcp: Option<stream::ResponseFrame>,
}

impl Default for DualResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl DualResolver {
    pub fn new() -> Self {
        Self {
            servers: Vec::new(),
            name: Vec::new(),
            v6_state: FamilyState::Disabled,
            v4_state: FamilyState::Disabled,
            v6_results: Vec::new(),
            v4_results: Vec::new(),
            active: false,
            cache: Vec::new(),
            last_ms: 0,
            next_id: 1,
            tcp: None,
        }
    }

    pub fn seed_id(&mut self, seed: u16) {
        self.next_id = seed.max(1);
    }

    fn alloc_id(&mut self) -> u16 {
        let id = self.next_id;
        // Mix via odd-multiplier Weyl step so consecutive live DNS txids aren't sequential.
        self.next_id = self.next_id.wrapping_mul(40503).wrapping_add(13);
        if self.next_id == 0 {
            self.next_id = 1;
        }
        id
    }

    fn advance(&mut self, now: u64) {
        if now >= self.last_ms {
            self.last_ms = now;
            self.cache.retain(|e| e.expires > now);
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn abort(&mut self) {
        self.active = false;
        self.v6_state = FamilyState::Disabled;
        self.v4_state = FamilyState::Disabled;
        self.v6_results.clear();
        self.v4_results.clear();
        self.tcp = None;
    }

    pub fn start(
        &mut self,
        name: &[u8],
        servers: &[IpAddr],
        want_v6: bool,
        want_v4: bool,
        now: u64,
    ) -> Result<Option<Vec<IpAddr>>, Error> {
        self.advance(now);
        if self.active {
            return Err(Error::Capacity);
        }
        let norm = normalize(name)?;
        if servers.is_empty() || (!want_v6 && !want_v4) {
            return Err(Error::NoRoute);
        }
        self.v6_results.clear();
        self.v4_results.clear();
        self.servers = servers.to_vec();
        self.name = norm;

        let mut v6_cached = false;
        let mut v4_cached = false;
        if want_v6 {
            if let Some(entry) = self
                .cache
                .iter()
                .find(|c| c.name == self.name && c.kind == Kind::Aaaa)
            {
                v6_cached = true;
                self.v6_results.extend_from_slice(&entry.answers);
                self.v6_state = if entry.negative {
                    FamilyState::Failed
                } else {
                    FamilyState::Done
                };
            } else {
                let id = self.alloc_id();
                self.v6_state = FamilyState::Pending {
                    server_idx: 0,
                    attempts: 0,
                    next_ms: now,
                    id,
                };
            }
        } else {
            self.v6_state = FamilyState::Disabled;
        }

        if want_v4 {
            if let Some(entry) = self
                .cache
                .iter()
                .find(|c| c.name == self.name && c.kind == Kind::A)
            {
                v4_cached = true;
                self.v4_results.extend_from_slice(&entry.answers);
                self.v4_state = if entry.negative {
                    FamilyState::Failed
                } else {
                    FamilyState::Done
                };
            } else {
                let id = self.alloc_id();
                self.v4_state = FamilyState::Pending {
                    server_idx: 0,
                    attempts: 0,
                    next_ms: now,
                    id,
                };
            }
        } else {
            self.v4_state = FamilyState::Disabled;
        }

        if (!want_v6 || v6_cached) && (!want_v4 || v4_cached) {
            let merged = self.merged_results()?;
            self.abort();
            return Ok(Some(merged));
        }

        self.active = true;
        Ok(None)
    }

    fn merged_results(&self) -> Result<Vec<IpAddr>, Error> {
        let mut out = Vec::with_capacity(self.v6_results.len() + self.v4_results.len());
        // RFC 8305 style ordering: IPv6 addresses first, then IPv4.
        out.extend_from_slice(&self.v6_results);
        out.extend_from_slice(&self.v4_results);
        if out.is_empty() {
            Err(Error::NameNotFound)
        } else {
            Ok(out)
        }
    }

    pub fn poll(&mut self, now: u64) -> Result<DualPoll, Error> {
        self.advance(now);
        if !self.active {
            return Err(Error::NotBound);
        }
        if let Some(send) = self.poll_family(true, now)? {
            return Ok(send);
        }
        if let Some(send) = self.poll_family(false, now)? {
            return Ok(send);
        }
        let v6_done = !matches!(
            self.v6_state,
            FamilyState::Pending { .. } | FamilyState::Tcp { .. }
        );
        let v4_done = !matches!(
            self.v4_state,
            FamilyState::Pending { .. } | FamilyState::Tcp { .. }
        );
        if v6_done && v4_done {
            let res = self.merged_results();
            self.abort();
            return res.map(DualPoll::Resolved);
        }
        let mut deadline = None;
        for st in [self.v6_state, self.v4_state] {
            match st {
                FamilyState::Pending { next_ms, .. } => {
                    deadline = Some(deadline.map_or(next_ms, |d: u64| d.min(next_ms)));
                }
                FamilyState::Tcp { deadline_ms, .. } => {
                    deadline = Some(deadline.map_or(deadline_ms, |d: u64| d.min(deadline_ms)));
                }
                _ => {}
            }
        }
        Ok(DualPoll::Wait(deadline))
    }

    fn poll_family(&mut self, is_v6: bool, now: u64) -> Result<Option<DualPoll>, Error> {
        let kind = if is_v6 { Kind::Aaaa } else { Kind::A };
        let next_id = self.alloc_id();
        let servers_len = self.servers.len();
        let state = if is_v6 {
            &mut self.v6_state
        } else {
            &mut self.v4_state
        };
        if let FamilyState::Tcp {
            server_idx,
            id,
            sent,
            deadline_ms,
        } = state
        {
            if now >= *deadline_ms {
                *state = FamilyState::Failed;
                return Ok(None);
            }
            if !*sent {
                let server = self.servers[*server_idx];
                let query = stream::encode(&Question::new(*id, &self.name, kind)?.encode()?)?;
                let deadline = *deadline_ms;
                *sent = true;
                return Ok(Some(DualPoll::TcpFallback {
                    server,
                    kind,
                    query,
                    deadline_ms: deadline,
                }));
            }
            return Ok(None);
        }
        let FamilyState::Pending {
            server_idx,
            attempts,
            next_ms,
            id,
        } = state
        else {
            return Ok(None);
        };
        if now < *next_ms {
            return Ok(None);
        }
        if *attempts >= ATTEMPTS {
            if *server_idx + 1 < servers_len {
                *server_idx += 1;
                *attempts = 0;
                *id = next_id;
            } else {
                *state = FamilyState::Failed;
                return Ok(None);
            }
        }
        let server = self.servers[*server_idx];
        let q = Question::new(*id, &self.name, kind)?;
        let payload = q.encode()?;
        *attempts += 1;
        *next_ms = now.saturating_add(RETRY_MS);
        Ok(Some(DualPoll::SendUdp { server, payload }))
    }

    pub fn receive_udp(&mut self, server: IpAddr, payload: &[u8], now: u64) -> Result<(), Error> {
        self.advance(now);
        if !self.active || payload.len() < 2 {
            return Ok(());
        }
        let rx_id = u16::from_be_bytes([payload[0], payload[1]]);
        let (is_v6, qid, sidx) = match (self.v6_state, self.v4_state) {
            (FamilyState::Pending { id, server_idx, .. }, _) if id == rx_id => {
                (true, id, server_idx)
            }
            (_, FamilyState::Pending { id, server_idx, .. }) if id == rx_id => {
                (false, id, server_idx)
            }
            _ => return Ok(()),
        };
        if self.servers.get(sidx).copied() != Some(server) {
            return Ok(());
        }
        let kind = if is_v6 { Kind::Aaaa } else { Kind::A };
        let question = Question::new(qid, &self.name, kind)?;
        let result = parse_message_inner(payload, &question, DNS_LIMIT)?;
        self.apply_parsed(kind, result, now)
    }

    pub fn receive_tcp(&mut self, server: IpAddr, bytes: &[u8], kind: Kind, now: u64) -> Result<(), Error> {
        self.advance(now);
        if !self.active {
            return Ok(());
        }
        let frame = self.tcp.get_or_insert_with(stream::ResponseFrame::default);
        let Some(body) = frame.feed(bytes)? else {
            return Ok(());
        };
        let state = if kind == Kind::Aaaa {
            self.v6_state
        } else {
            self.v4_state
        };
        let (id, server_idx) = match state {
            FamilyState::Pending { id, server_idx, .. }
            | FamilyState::Tcp { id, server_idx, .. } => (id, server_idx),
            _ => return Ok(()),
        };
        if self.servers.get(server_idx).copied() != Some(server) {
            return Err(Error::Unsolicited);
        }
        let question = Question::new(id, &self.name, kind)?;
        let result = parse_message_inner(body, &question, TCP_DNS_LIMIT)?;
        self.apply_parsed(kind, result, now)
    }

    pub fn tcp_eof(&mut self, kind: Kind) {
        let state = if kind == Kind::Aaaa {
            &mut self.v6_state
        } else {
            &mut self.v4_state
        };
        if matches!(state, FamilyState::Tcp { .. }) {
            *state = FamilyState::Failed;
        }
    }

    fn apply_parsed(&mut self, kind: Kind, result: Response, now: u64) -> Result<(), Error> {
        let is_v6 = kind == Kind::Aaaa;
        match result {
            Response::Answers(answers) => {
                let mut addrs = Vec::new();
                let mut ttl = u32::MAX;
                for ans in answers {
                    ttl = ttl.min(ans.ttl);
                    match ans.address {
                        AddressData::V4(v4) => addrs.push(IpAddr::V4(v4)),
                        AddressData::V6(v6) => addrs.push(IpAddr::V6(v6)),
                    }
                }
                self.store_cache(kind, &addrs, false, ttl, now);
                if is_v6 {
                    self.v6_results = addrs;
                    self.v6_state = FamilyState::Done;
                } else {
                    self.v4_results = addrs;
                    self.v4_state = FamilyState::Done;
                }
            }
            Response::Answer(ans) => {
                let ip = match ans.address {
                    AddressData::V4(v4) => IpAddr::V4(v4),
                    AddressData::V6(v6) => IpAddr::V6(v6),
                };
                let addrs = alloc::vec![ip];
                self.store_cache(kind, &addrs, false, ans.ttl, now);
                if is_v6 {
                    self.v6_results = addrs;
                    self.v6_state = FamilyState::Done;
                } else {
                    self.v4_results = addrs;
                    self.v4_state = FamilyState::Done;
                }
            }
            Response::Negative { ttl, .. } => {
                self.store_cache(kind, &[], true, ttl, now);
                if is_v6 {
                    self.v6_state = FamilyState::Failed;
                } else {
                    self.v4_state = FamilyState::Failed;
                }
            }
            Response::Truncated => {
                let state = if is_v6 {
                    &mut self.v6_state
                } else {
                    &mut self.v4_state
                };
                if let FamilyState::Pending { server_idx, id, .. } = *state {
                    self.tcp = None;
                    *state = FamilyState::Tcp {
                        server_idx,
                        id,
                        sent: false,
                        deadline_ms: now.saturating_add(TCP_TIMEOUT_MS),
                    };
                } else {
                    *state = FamilyState::Failed;
                }
            }
            Response::ServerFailure => {
                let servers_len = self.servers.len();
                let next_id = self.alloc_id();
                let state = if is_v6 {
                    &mut self.v6_state
                } else {
                    &mut self.v4_state
                };
                if let FamilyState::Pending {
                    server_idx,
                    attempts,
                    next_ms,
                    id,
                } = state
                {
                    if *server_idx + 1 < servers_len {
                        *server_idx += 1;
                        *attempts = 0;
                        *next_ms = now;
                        *id = next_id;
                    } else {
                        *state = FamilyState::Failed;
                    }
                }
            }
        }
        Ok(())
    }

    fn store_cache(
        &mut self,
        kind: Kind,
        addrs: &[IpAddr],
        negative: bool,
        ttl: u32,
        now: u64,
    ) {
        if ttl == 0 {
            return;
        }
        self.cache.retain(|c| !(c.name == self.name && c.kind == kind));
        if self.cache.len() >= 16 {
            self.cache.remove(0);
        }
        self.cache.push(DualCacheEntry {
            name: self.name.clone(),
            kind,
            answers: addrs.to_vec(),
            negative,
            expires: now.saturating_add(u64::from(ttl) * 1000),
        });
    }
}
