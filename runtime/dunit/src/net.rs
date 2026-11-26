//! Native dual-stack (IPv4 & IPv6) TCP/UDP sockets over authorized `NET_CONNECT` handles.
use crate::{
    error::checked,
    event::{self, Interest},
    io::{Read, Write},
    time::{timeout_ns, Duration, Instant},
    Error, Result,
};
use alloc::{string::String, vec::Vec};
use core::{fmt, str::FromStr};
use dunit_socket_abi::{
    decode_dns_records, DnsRecord, Endpoint6, Endpoints6, Options, RecvHeader6,
    DNS_RECORDS_HEADER_LEN, DNS_RECORD_LEN, ENDPOINT6_LEN, ENDPOINTS6_LEN, FAMILY_IPV4,
    FAMILY_IPV6, MAX_DNS_NAME, MAX_DNS_RECORDS, MAX_ICMP_PAYLOAD, MAX_PAYLOAD, OP_ACCEPT,
    OP_BIND, OP_CONNECT, OP_ENDPOINTS, OP_ERROR, OP_LISTEN, OP_RECV_FROM, OP_SEND_TO,
    OP_SHUTDOWN_WRITE, PROTO_DNS, PROTO_ICMP, PROTO_STATUS, PROTO_TCP, PROTO_UDP,
    RECV_HEADER6_LEN, VERSION_V2,
};

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const DNS_TIMEOUT: Duration = Duration::from_secs(15);
const HAPPY_EYEBALLS_DELAY: Duration = Duration::from_millis(250);
const MAX_UDP_PAYLOAD: usize = 1472;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const UNSPECIFIED: Self = Self([0, 0, 0, 0]);
    pub const LOCALHOST: Self = Self([127, 0, 0, 1]);

    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }

    pub const fn octets(self) -> [u8; 4] {
        self.0
    }
}

impl From<[u8; 4]> for Ipv4Addr {
    fn from(octets: [u8; 4]) -> Self {
        Self(octets)
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

impl FromStr for Ipv4Addr {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut out = [0u8; 4];
        let mut parts = s.split('.');
        for slot in &mut out {
            let part = parts.next().ok_or(Error(libdunit::EINVAL))?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error(libdunit::EINVAL));
            }
            *slot = part.parse().map_err(|_| Error(libdunit::EINVAL))?;
        }
        if parts.next().is_some() {
            return Err(Error(libdunit::EINVAL));
        }
        Ok(Self(out))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ipv6Addr(pub [u8; 16]);

impl Ipv6Addr {
    pub const UNSPECIFIED: Self = Self([0; 16]);
    pub const LOCALHOST: Self = Self([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

    pub const fn new(a: u16, b: u16, c: u16, d: u16, e: u16, f: u16, g: u16, h: u16) -> Self {
        let ab = a.to_be_bytes();
        let cd = b.to_be_bytes();
        let ef = c.to_be_bytes();
        let gh = d.to_be_bytes();
        let ij = e.to_be_bytes();
        let kl = f.to_be_bytes();
        let mn = g.to_be_bytes();
        let op = h.to_be_bytes();
        Self([
            ab[0], ab[1], cd[0], cd[1], ef[0], ef[1], gh[0], gh[1], ij[0], ij[1], kl[0], kl[1],
            mn[0], mn[1], op[0], op[1],
        ])
    }

    pub const fn octets(self) -> [u8; 16] {
        self.0
    }

    pub fn segments(self) -> [u16; 8] {
        let mut out = [0u16; 8];
        for i in 0..8 {
            out[i] = u16::from_be_bytes([self.0[i * 2], self.0[i * 2 + 1]]);
        }
        out
    }

    pub const fn is_unspecified(self) -> bool {
        let s = self.0;
        s[0] == 0
            && s[1] == 0
            && s[2] == 0
            && s[3] == 0
            && s[4] == 0
            && s[5] == 0
            && s[6] == 0
            && s[7] == 0
            && s[8] == 0
            && s[9] == 0
            && s[10] == 0
            && s[11] == 0
            && s[12] == 0
            && s[13] == 0
            && s[14] == 0
            && s[15] == 0
    }
}

impl From<[u8; 16]> for Ipv6Addr {
    fn from(octets: [u8; 16]) -> Self {
        Self(octets)
    }
}

impl fmt::Display for Ipv6Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let segs = self.segments();
        write!(
            f,
            "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
            segs[0], segs[1], segs[2], segs[3], segs[4], segs[5], segs[6], segs[7]
        )
    }
}

impl FromStr for Ipv6Addr {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.is_empty() || s.len() > 45 || !s.contains(':') {
            return Err(Error(libdunit::EINVAL));
        }
        let (h, t, comp) = match s.split_once("::") {
            Some((h, t)) if !t.contains("::") => (h, t, true),
            Some(_) => return Err(Error(libdunit::EINVAL)),
            None => (s, "", false),
        };
        let mut head = Vec::new();
        if !h.is_empty() {
            for g in h.split(':') {
                if g.is_empty() || g.len() > 4 {
                    return Err(Error(libdunit::EINVAL));
                }
                head.push(u16::from_str_radix(g, 16).map_err(|_| Error(libdunit::EINVAL))?);
            }
        }
        let mut tail = Vec::new();
        if !t.is_empty() {
            for g in t.split(':') {
                if g.is_empty() || g.len() > 4 {
                    return Err(Error(libdunit::EINVAL));
                }
                tail.push(u16::from_str_radix(g, 16).map_err(|_| Error(libdunit::EINVAL))?);
            }
        }
        if (comp && head.len() + tail.len() >= 8)
            || (!comp && (head.len() != 8 || !tail.is_empty()))
        {
            return Err(Error(libdunit::EINVAL));
        }
        let mut words = [0u16; 8];
        words[..head.len()].copy_from_slice(&head);
        words[8 - tail.len()..].copy_from_slice(&tail);
        let mut out = [0u8; 16];
        for (i, w) in words.iter().enumerate() {
            let b = w.to_be_bytes();
            out[i * 2] = b[0];
            out[i * 2 + 1] = b[1];
        }
        Ok(Self(out))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IpAddr {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

impl From<Ipv4Addr> for IpAddr {
    fn from(ip: Ipv4Addr) -> Self {
        Self::V4(ip)
    }
}

impl From<Ipv6Addr> for IpAddr {
    fn from(ip: Ipv6Addr) -> Self {
        Self::V6(ip)
    }
}

impl From<[u8; 4]> for IpAddr {
    fn from(ip: [u8; 4]) -> Self {
        Self::V4(Ipv4Addr(ip))
    }
}

impl From<[u8; 16]> for IpAddr {
    fn from(ip: [u8; 16]) -> Self {
        Self::V6(Ipv6Addr(ip))
    }
}

impl fmt::Display for IpAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4(ip) => ip.fmt(f),
            Self::V6(ip) => ip.fmt(f),
        }
    }
}

impl FromStr for IpAddr {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.contains(':') {
            s.parse::<Ipv6Addr>().map(Self::V6)
        } else {
            s.parse::<Ipv4Addr>().map(Self::V4)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SocketAddrV4 {
    ip: Ipv4Addr,
    port: u16,
}

impl SocketAddrV4 {
    pub const fn new(ip: Ipv4Addr, port: u16) -> Self {
        Self { ip, port }
    }

    pub const fn ip(self) -> Ipv4Addr {
        self.ip
    }

    pub const fn port(self) -> u16 {
        self.port
    }
}

impl fmt::Display for SocketAddrV4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}

impl FromStr for SocketAddrV4 {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let (ip_s, port_s) = s.split_once(':').ok_or(Error(libdunit::EINVAL))?;
        if port_s.is_empty() || !port_s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error(libdunit::EINVAL));
        }
        let ip = ip_s.parse()?;
        let port = port_s.parse().map_err(|_| Error(libdunit::EINVAL))?;
        Ok(Self::new(ip, port))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SocketAddrV6 {
    ip: Ipv6Addr,
    port: u16,
    scope_id: u32,
}

impl SocketAddrV6 {
    pub const fn new(ip: Ipv6Addr, port: u16, scope_id: u32) -> Self {
        Self { ip, port, scope_id }
    }

    pub const fn ip(self) -> Ipv6Addr {
        self.ip
    }

    pub const fn port(self) -> u16 {
        self.port
    }

    pub const fn scope_id(self) -> u32 {
        self.scope_id
    }
}

impl fmt::Display for SocketAddrV6 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scope_id != 0 {
            write!(f, "[{}%{}]:{}", self.ip, self.scope_id, self.port)
        } else {
            write!(f, "[{}]:{}", self.ip, self.port)
        }
    }
}

impl FromStr for SocketAddrV6 {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let rest = s.strip_prefix('[').ok_or(Error(libdunit::EINVAL))?;
        let (host_part, port_part) = rest.split_once("]:").ok_or(Error(libdunit::EINVAL))?;
        if port_part.is_empty() || !port_part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error(libdunit::EINVAL));
        }
        let port = port_part.parse().map_err(|_| Error(libdunit::EINVAL))?;
        let (ip_s, scope_id) = match host_part.split_once('%') {
            Some((ip, sc)) => {
                let id = sc.parse::<u32>().map_err(|_| Error(libdunit::EINVAL))?;
                (ip, id)
            }
            None => (host_part, 0),
        };
        let ip = ip_s.parse()?;
        Ok(Self::new(ip, port, scope_id))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SocketAddr {
    V4(SocketAddrV4),
    V6(SocketAddrV6),
}

impl SocketAddr {
    pub const fn new(ip: IpAddr, port: u16) -> Self {
        match ip {
            IpAddr::V4(v4) => Self::V4(SocketAddrV4::new(v4, port)),
            IpAddr::V6(v6) => Self::V6(SocketAddrV6::new(v6, port, 0)),
        }
    }

    pub const fn ip(self) -> IpAddr {
        match self {
            Self::V4(a) => IpAddr::V4(a.ip),
            Self::V6(a) => IpAddr::V6(a.ip),
        }
    }

    pub const fn port(self) -> u16 {
        match self {
            Self::V4(a) => a.port,
            Self::V6(a) => a.port,
        }
    }

    fn to_wire6(self) -> Endpoint6 {
        match self {
            Self::V4(a) => Endpoint6::ipv4(a.ip.0, a.port),
            Self::V6(a) => Endpoint6::ipv6(a.ip.0, a.port, a.scope_id),
        }
    }

    fn from_wire6(ep: Endpoint6) -> Result<Self> {
        match ep.family {
            FAMILY_IPV4 => {
                let ip: [u8; 4] = ep.addr[..4].try_into().map_err(|_| Error(libdunit::EIO))?;
                Ok(Self::V4(SocketAddrV4::new(Ipv4Addr(ip), ep.port)))
            }
            FAMILY_IPV6 => Ok(Self::V6(SocketAddrV6::new(
                Ipv6Addr(ep.addr),
                ep.port,
                ep.scope_id,
            ))),
            _ => Err(Error(libdunit::EIO)),
        }
    }

    fn into_v4(self) -> Result<SocketAddrV4> {
        match self {
            Self::V4(v4) => Ok(v4),
            Self::V6(_) => Err(Error(libdunit::EINVAL)),
        }
    }
}

impl From<SocketAddrV4> for SocketAddr {
    fn from(a: SocketAddrV4) -> Self {
        Self::V4(a)
    }
}

impl From<SocketAddrV6> for SocketAddr {
    fn from(a: SocketAddrV6) -> Self {
        Self::V6(a)
    }
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4(a) => a.fmt(f),
            Self::V6(a) => a.fmt(f),
        }
    }
}

impl FromStr for SocketAddr {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.starts_with('[') {
            s.parse::<SocketAddrV6>().map(Self::V6)
        } else {
            s.parse::<SocketAddrV4>().map(Self::V4)
        }
    }
}

/// Process-local `NET_CONNECT` capability handle.
pub struct NetGrant(u32);

impl NetGrant {
    pub fn from_raw_handle(handle: u32) -> Result<Self> {
        let rights = checked(libdunit::handle_rights(handle))?;
        if rights & libdunit::RIGHT_NET_CONNECT as usize == 0 {
            return Err(Error(libdunit::EACCES));
        }
        Ok(Self(handle))
    }

    pub fn raw_handle(&self) -> u32 {
        self.0
    }

    pub fn into_raw_handle(self) -> u32 {
        let h = self.0;
        core::mem::forget(self);
        h
    }
}

impl Drop for NetGrant {
    fn drop(&mut self) {
        let _ = libdunit::handle_close(self.0);
    }
}

fn deadline_from(timeout: Option<Duration>) -> Result<Option<Instant>> {
    match timeout {
        None => Ok(None),
        Some(dur) => Instant::now()?
            .checked_add(dur)
            .ok_or(Error(libdunit::EINVAL))
            .map(Some),
    }
}

fn validate_timeout(timeout: Option<Duration>) -> Result<()> {
    if let Some(dur) = timeout {
        if dur.is_zero() {
            return Err(Error(libdunit::EINVAL));
        }
        let _ = timeout_ns(dur)?;
    }
    Ok(())
}

struct SocketFd {
    fd: usize,
    nonblocking: bool,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

impl SocketFd {
    fn open(grant: &NetGrant, protocol: u16, open_timeout: Duration) -> Result<Self> {
        let fd = checked(libdunit::net_socket_open(
            grant.0,
            protocol,
            &Options::encode_version(VERSION_V2),
        ))?;
        let sock = Self {
            fd,
            nonblocking: false,
            read_timeout: None,
            write_timeout: None,
        };
        let deadline = deadline_from(Some(open_timeout))?;
        sock.wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
        if let Some(err) = sock.take_error()? {
            return Err(err);
        }
        Ok(sock)
    }

    fn op(&self, op: u16, input: &[u8], output: &mut [u8]) -> isize {
        libdunit::net_socket_op(self.fd, op, input, output)
    }

    fn wait_for(&self, flags: u32, deadline: Option<Instant>, on_timeout: isize) -> Result<u32> {
        let timeout = match deadline {
            None => event::INFINITE,
            Some(end) => {
                let now = Instant::now()?;
                if now >= end {
                    return Err(Error(on_timeout));
                }
                let mut ready = [event::Event::default(); 2];
                let interest = [
                    Interest::descriptor(self.fd as u32, flags, 0),
                    end.deadline(1),
                ];
                let count = libdunit::wait_set::wait(&interest, &mut ready, event::INFINITE)
                    .map_err(Error)?;
                let mut fd_events = 0u32;
                let mut timed_out = false;
                for ev in &ready[..count] {
                    if ev.token == 0 {
                        fd_events = ev.events;
                    } else if ev.token == 1 && (ev.events & event::ELAPSED != 0) {
                        timed_out = true;
                    }
                }
                if fd_events != 0 {
                    return Ok(fd_events);
                }
                if timed_out {
                    return Err(Error(on_timeout));
                }
                return Err(Error(libdunit::EIO));
            }
        };
        let mut ready = [event::Event::default()];
        let count = libdunit::wait_set::wait(
            &[Interest::descriptor(self.fd as u32, flags, 0)],
            &mut ready,
            timeout,
        )
        .map_err(Error)?;
        if count == 0 {
            return Err(Error(on_timeout));
        }
        Ok(ready[0].events)
    }

    fn take_error(&self) -> Result<Option<Error>> {
        let mut raw = [0u8; 4];
        let n = checked(self.op(OP_ERROR, &[], &mut raw))?;
        if n != 4 {
            return Err(Error(libdunit::EIO));
        }
        let code = i32::from_le_bytes(raw);
        if code > 0 {
            return Err(Error(libdunit::EIO));
        }
        Ok((code < 0).then_some(Error(code as isize)))
    }

    fn endpoints6(&self) -> Result<Endpoints6> {
        let mut raw = [0u8; ENDPOINTS6_LEN];
        let n = checked(self.op(OP_ENDPOINTS, &[], &mut raw))?;
        if n != ENDPOINTS6_LEN {
            return Err(Error(libdunit::EIO));
        }
        Endpoints6::decode(&raw).map_err(|_| Error(libdunit::EIO))
    }

    fn wait_write_op(&self, op: u16, input: &[u8], deadline: Option<Instant>) -> Result<()> {
        loop {
            let res = self.op(op, input, &mut []);
            if res == 0 {
                return Ok(());
            }
            if res != libdunit::EAGAIN {
                return Err(Error(res));
            }
            self.wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
            if let Some(err) = self.take_error()? {
                return Err(err);
            }
        }
    }

    fn set_nonblocking(&mut self, nonblocking: bool) {
        self.nonblocking = nonblocking;
    }

    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        validate_timeout(timeout)?;
        self.read_timeout = timeout;
        Ok(())
    }

    fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        validate_timeout(timeout)?;
        self.write_timeout = timeout;
        Ok(())
    }
}

impl Drop for SocketFd {
    fn drop(&mut self) {
        let _ = libdunit::close(self.fd);
    }
}

pub struct TcpStream {
    inner: SocketFd,
}

impl TcpStream {
    pub fn connect(grant: &NetGrant, addr: impl Into<SocketAddr>) -> Result<Self> {
        Self::connect_inner(grant, addr.into(), None)
    }

    pub fn connect_timeout(
        grant: &NetGrant,
        addr: impl Into<SocketAddr>,
        timeout: Duration,
    ) -> Result<Self> {
        validate_timeout(Some(timeout))?;
        Self::connect_inner(grant, addr.into(), deadline_from(Some(timeout))?)
    }

    pub fn connect_host(grant: &NetGrant, host: &str, port: u16) -> Result<Self> {
        Self::connect_host_timeout(grant, host, port, DNS_TIMEOUT)
    }

    /// Connect using RFC 8305 Happy Eyeballs across all resolved IPv6/IPv4 endpoints.
    pub fn connect_host_timeout(
        grant: &NetGrant,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<Self> {
        validate_timeout(Some(timeout))?;
        let deadline = deadline_from(Some(timeout))?;
        let addrs = resolve_addrs_timeout(grant, host, timeout)?;
        if addrs.is_empty() {
            return Err(Error(libdunit::ENOENT));
        }
        if addrs.len() == 1 {
            return Self::connect_inner(grant, SocketAddr::new(addrs[0], port), deadline);
        }
        Self::connect_happy_eyeballs(grant, &addrs, port, deadline)
    }

    fn connect_happy_eyeballs(
        grant: &NetGrant,
        addrs: &[IpAddr],
        port: u16,
        deadline: Option<Instant>,
    ) -> Result<Self> {
        const MAX_INFLIGHT: usize = 4;
        let ordered = interleave_happy_eyeballs(addrs);
        let mut attempts: Vec<SocketFd> = Vec::with_capacity(MAX_INFLIGHT);
        let mut next_idx = 0usize;
        let mut next_start = Instant::now()?;
        let mut last_err: Option<Error> = None;

        loop {
            let now = Instant::now()?;
            if let Some(end) = deadline {
                if now >= end && attempts.is_empty() {
                    return Err(last_err.unwrap_or(Error(libdunit::EAGAIN)));
                }
            }

            while (attempts.is_empty() || now >= next_start)
                && next_idx < ordered.len()
                && attempts.len() < MAX_INFLIGHT
            {
                let target = SocketAddr::new(ordered[next_idx], port);
                next_idx += 1;
                let open_limit = remaining_capped(deadline, OPEN_TIMEOUT)?;
                let sock = match SocketFd::open(grant, PROTO_TCP, open_limit) {
                    Ok(s) => s,
                    Err(e) => {
                        last_err = Some(e);
                        next_start = now;
                        continue;
                    }
                };
                let wire = target.to_wire6().encode();
                let res = sock.op(OP_CONNECT, &wire, &mut []);
                if res == 0 {
                    return Ok(Self { inner: sock });
                }
                if res == libdunit::EAGAIN {
                    attempts.push(sock);
                    next_start = now.checked_add(HAPPY_EYEBALLS_DELAY).unwrap_or(now);
                    break;
                }
                last_err = Some(Error(res));
                next_start = now;
            }

            if attempts.is_empty() {
                if next_idx >= ordered.len() {
                    return Err(last_err.unwrap_or(Error(libdunit::EIO)));
                }
                continue;
            }

            let can_spawn_more = next_idx < ordered.len() && attempts.len() < MAX_INFLIGHT;
            let timer_target = match (deadline, can_spawn_more) {
                (Some(end), true) => Some(end.min(next_start)),
                (Some(end), false) => Some(end),
                (None, true) => Some(next_start),
                (None, false) => None,
            };

            let mut interests: [Interest; MAX_INFLIGHT + 1] =
                [Interest::default(); MAX_INFLIGHT + 1];
            let mut n_int = 0usize;
            for (i, sock) in attempts.iter().enumerate() {
                interests[n_int] = Interest::descriptor(sock.fd as u32, event::WRITE, i as u64);
                n_int += 1;
            }
            if let Some(t) = timer_target {
                interests[n_int] = t.deadline(MAX_INFLIGHT as u64);
                n_int += 1;
            }
            let mut events = [event::Event::default(); MAX_INFLIGHT + 1];
            let count = libdunit::wait_set::wait(
                &interests[..n_int],
                &mut events[..n_int],
                event::INFINITE,
            )
            .map_err(Error)?;

            let mut remove_indices = Vec::new();
            for ev in &events[..count] {
                let idx = ev.token as usize;
                if idx >= attempts.len() {
                    continue;
                }
                if let Some(err) = attempts[idx].take_error()? {
                    last_err = Some(err);
                    remove_indices.push(idx);
                    continue;
                }
                match attempts[idx].endpoints6() {
                    Ok(eps) if eps.remote.port != 0 => {
                        let winner = attempts.swap_remove(idx);
                        return Ok(Self { inner: winner });
                    }
                    Ok(_) => {
                        if ev.events & (event::ERROR | event::HANGUP) != 0 {
                            last_err = Some(Error(libdunit::EPIPE));
                            remove_indices.push(idx);
                        }
                    }
                    Err(e) => {
                        last_err = Some(e);
                        remove_indices.push(idx);
                    }
                }
            }
            remove_indices.sort_unstable();
            remove_indices.dedup();
            for &idx in remove_indices.iter().rev() {
                attempts.remove(idx);
            }
            if attempts.is_empty() && next_idx < ordered.len() {
                next_start = Instant::now()?;
            }
            if let Some(end) = deadline {
                if Instant::now()? >= end {
                    return Err(last_err.unwrap_or(Error(libdunit::EAGAIN)));
                }
            }
        }
    }

    fn connect_inner(
        grant: &NetGrant,
        addr: SocketAddr,
        deadline: Option<Instant>,
    ) -> Result<Self> {
        let open_limit = remaining_capped(deadline, OPEN_TIMEOUT)?;
        let inner = SocketFd::open(grant, PROTO_TCP, open_limit)?;
        let wire = addr.to_wire6().encode();
        let res = inner.op(OP_CONNECT, &wire, &mut []);
        if res == 0 {
            return Ok(Self { inner });
        }
        if res != libdunit::EAGAIN {
            return Err(Error(res));
        }
        loop {
            inner.wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
            if let Some(err) = inner.take_error()? {
                return Err(err);
            }
            if inner.endpoints6()?.remote.port != 0 {
                return Ok(Self { inner });
            }
        }
    }

    pub fn local_socket_addr(&self) -> Result<SocketAddr> {
        SocketAddr::from_wire6(self.inner.endpoints6()?.local)
    }

    pub fn peer_socket_addr(&self) -> Result<SocketAddr> {
        let ep = self.inner.endpoints6()?.remote;
        if ep.port == 0 {
            return Err(Error(libdunit::EINVAL));
        }
        SocketAddr::from_wire6(ep)
    }

    pub fn local_addr(&self) -> Result<SocketAddrV4> {
        self.local_socket_addr()?.into_v4()
    }

    pub fn peer_addr(&self) -> Result<SocketAddrV4> {
        self.peer_socket_addr()?.into_v4()
    }

    pub fn shutdown_write(&self) -> Result<()> {
        let deadline = deadline_from(self.inner.write_timeout)?;
        loop {
            let res = self.inner.op(OP_SHUTDOWN_WRITE, &[], &mut []);
            if res == 0 {
                return Ok(());
            }
            if res != libdunit::EAGAIN || self.inner.nonblocking {
                return Err(Error(res));
            }
            self.inner
                .wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
        }
    }

    pub fn set_nonblocking(&mut self, nonblocking: bool) {
        self.inner.set_nonblocking(nonblocking);
    }

    pub fn is_nonblocking(&self) -> bool {
        self.inner.nonblocking
    }

    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.inner.set_read_timeout(timeout)
    }

    pub fn read_timeout(&self) -> Option<Duration> {
        self.inner.read_timeout
    }

    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.inner.set_write_timeout(timeout)
    }

    pub fn write_timeout(&self) -> Option<Duration> {
        self.inner.write_timeout
    }

    pub fn take_error(&self) -> Result<Option<Error>> {
        self.inner.take_error()
    }

    pub fn descriptor(&self) -> u32 {
        self.inner.fd as u32
    }
}

fn remaining_capped(deadline: Option<Instant>, cap: Duration) -> Result<Duration> {
    match deadline {
        None => Ok(cap),
        Some(end) => {
            let now = Instant::now()?;
            if now >= end {
                return Err(Error(libdunit::EAGAIN));
            }
            Ok(cap.min(end.saturating_duration_since(now)))
        }
    }
}

fn interleave_happy_eyeballs(addrs: &[IpAddr]) -> Vec<IpAddr> {
    let mut v6 = Vec::new();
    let mut v4 = Vec::new();
    for &a in addrs {
        match a {
            IpAddr::V6(_) => v6.push(a),
            IpAddr::V4(_) => v4.push(a),
        }
    }
    let mut out = Vec::with_capacity(addrs.len());
    let first_v6 = matches!(addrs.first(), Some(IpAddr::V6(_)) | None);
    let max_len = v6.len().max(v4.len());
    for i in 0..max_len {
        if first_v6 {
            if let Some(&a) = v6.get(i) {
                out.push(a);
            }
            if let Some(&a) = v4.get(i) {
                out.push(a);
            }
        } else {
            if let Some(&a) = v4.get(i) {
                out.push(a);
            }
            if let Some(&a) = v6.get(i) {
                out.push(a);
            }
        }
    }
    out
}

impl Read for TcpStream {
    fn read(&mut self, output: &mut [u8]) -> Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let deadline = if self.inner.nonblocking {
            None
        } else {
            deadline_from(self.inner.read_timeout)?
        };
        let cap = output.len().min(MAX_PAYLOAD);
        loop {
            let res = libdunit::read(self.inner.fd, &mut output[..cap]);
            if res >= 0 {
                return Ok(res as usize);
            }
            if res != libdunit::EAGAIN || self.inner.nonblocking {
                return Err(Error(res));
            }
            self.inner
                .wait_for(event::READ, deadline, libdunit::EAGAIN)?;
        }
    }
}

impl Write for TcpStream {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let deadline = if self.inner.nonblocking {
            None
        } else {
            deadline_from(self.inner.write_timeout)?
        };
        let chunk = &bytes[..bytes.len().min(MAX_PAYLOAD)];
        loop {
            let res = libdunit::write(self.inner.fd, chunk);
            if res >= 0 {
                return Ok(res as usize);
            }
            if res != libdunit::EAGAIN || self.inner.nonblocking {
                return Err(Error(res));
            }
            self.inner
                .wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
        }
    }
}

pub struct TcpListener {
    inner: SocketFd,
}

impl TcpListener {
    pub fn bind(grant: &NetGrant, addr: impl Into<SocketAddr>) -> Result<Self> {
        Self::bind_backlog(grant, addr, 4)
    }

    pub fn bind_backlog(
        grant: &NetGrant,
        addr: impl Into<SocketAddr>,
        backlog: u32,
    ) -> Result<Self> {
        let deadline = deadline_from(Some(OPEN_TIMEOUT))?;
        let inner = SocketFd::open(grant, PROTO_TCP, OPEN_TIMEOUT)?;
        let wire = addr.into().to_wire6().encode();
        inner.wait_write_op(OP_BIND, &wire, deadline)?;
        inner.wait_write_op(OP_LISTEN, &backlog.max(1).to_le_bytes(), deadline)?;
        Ok(Self { inner })
    }

    pub fn accept_addr(&self) -> Result<(TcpStream, SocketAddr)> {
        if self.inner.nonblocking {
            return self.try_accept_addr();
        }
        self.accept_loop(deadline_from(self.inner.read_timeout)?, false)
    }

    pub fn accept(&self) -> Result<(TcpStream, SocketAddrV4)> {
        let (stream, addr) = self.accept_addr()?;
        Ok((stream, addr.into_v4()?))
    }

    pub fn accept_timeout(&self, timeout: Duration) -> Result<(TcpStream, SocketAddrV4)> {
        validate_timeout(Some(timeout))?;
        let (stream, addr) = self.accept_loop(deadline_from(Some(timeout))?, false)?;
        Ok((stream, addr.into_v4()?))
    }

    pub fn accept_addr_timeout(&self, timeout: Duration) -> Result<(TcpStream, SocketAddr)> {
        validate_timeout(Some(timeout))?;
        self.accept_loop(deadline_from(Some(timeout))?, false)
    }

    pub fn try_accept_addr(&self) -> Result<(TcpStream, SocketAddr)> {
        self.accept_loop(None, true)
    }

    pub fn try_accept(&self) -> Result<(TcpStream, SocketAddrV4)> {
        let (stream, addr) = self.try_accept_addr()?;
        Ok((stream, addr.into_v4()?))
    }

    fn accept_loop(
        &self,
        deadline: Option<Instant>,
        nonblocking: bool,
    ) -> Result<(TcpStream, SocketAddr)> {
        let pre = nonblocking && self.is_readable();
        let mut op_deadline = None;
        loop {
            let mut raw = [0u8; ENDPOINT6_LEN];
            let res = self.inner.op(OP_ACCEPT, &[], &mut raw);
            if res >= 0 {
                let remote = Endpoint6::decode(&raw).map_err(|_| Error(libdunit::EIO))?;
                return Ok((
                    TcpStream {
                        inner: SocketFd {
                            fd: res as usize,
                            nonblocking: false,
                            read_timeout: None,
                            write_timeout: None,
                        },
                    },
                    SocketAddr::from_wire6(remote)?,
                ));
            }
            if res != libdunit::EAGAIN {
                return Err(Error(res));
            }
            let wait_until = if nonblocking {
                if !pre {
                    return Err(Error(res));
                }
                if op_deadline.is_none() {
                    op_deadline = deadline_from(Some(OPEN_TIMEOUT))?;
                }
                op_deadline
            } else {
                deadline
            };
            self.inner
                .wait_for(event::READ, wait_until, libdunit::EAGAIN)?;
            if let Some(err) = self.inner.take_error()? {
                return Err(err);
            }
        }
    }

    fn is_readable(&self) -> bool {
        let mut ready = [event::Event::default()];
        libdunit::wait_set::wait(
            &[Interest::descriptor(self.inner.fd as u32, event::READ, 0)],
            &mut ready,
            0,
        )
        .is_ok_and(|n| n > 0 && (ready[0].events & event::READ != 0))
    }

    pub fn local_socket_addr(&self) -> Result<SocketAddr> {
        SocketAddr::from_wire6(self.inner.endpoints6()?.local)
    }

    pub fn local_addr(&self) -> Result<SocketAddrV4> {
        self.local_socket_addr()?.into_v4()
    }

    pub fn set_nonblocking(&mut self, nonblocking: bool) {
        self.inner.set_nonblocking(nonblocking);
    }

    pub fn is_nonblocking(&self) -> bool {
        self.inner.nonblocking
    }

    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.inner.set_read_timeout(timeout)
    }

    pub fn read_timeout(&self) -> Option<Duration> {
        self.inner.read_timeout
    }

    pub fn take_error(&self) -> Result<Option<Error>> {
        self.inner.take_error()
    }

    pub fn descriptor(&self) -> u32 {
        self.inner.fd as u32
    }
}

pub struct UdpSocket {
    inner: SocketFd,
}

impl UdpSocket {
    pub fn open(grant: &NetGrant) -> Result<Self> {
        Ok(Self {
            inner: SocketFd::open(grant, PROTO_UDP, OPEN_TIMEOUT)?,
        })
    }

    pub fn bind(grant: &NetGrant, addr: impl Into<SocketAddr>) -> Result<Self> {
        let deadline = deadline_from(Some(OPEN_TIMEOUT))?;
        let inner = SocketFd::open(grant, PROTO_UDP, OPEN_TIMEOUT)?;
        inner.wait_write_op(OP_BIND, &addr.into().to_wire6().encode(), deadline)?;
        Ok(Self { inner })
    }

    pub fn connect(&self, addr: impl Into<SocketAddr>) -> Result<()> {
        let deadline = if self.inner.nonblocking {
            None
        } else {
            deadline_from(self.inner.write_timeout)?
        };
        let wire = addr.into().to_wire6().encode();
        let res = self.inner.op(OP_CONNECT, &wire, &mut []);
        if res == 0 {
            return Ok(());
        }
        if res != libdunit::EAGAIN || self.inner.nonblocking {
            return Err(Error(res));
        }
        loop {
            self.inner
                .wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
            if let Some(err) = self.inner.take_error()? {
                return Err(err);
            }
            if self.inner.endpoints6()?.remote.port != 0 {
                return Ok(());
            }
        }
    }

    pub fn send_to(&self, buf: &[u8], addr: impl Into<SocketAddr>) -> Result<usize> {
        if buf.len() > MAX_UDP_PAYLOAD {
            return Err(Error(libdunit::EMSGSIZE));
        }
        let mut packet = [0u8; ENDPOINT6_LEN + MAX_UDP_PAYLOAD];
        packet[..ENDPOINT6_LEN].copy_from_slice(&addr.into().to_wire6().encode());
        packet[ENDPOINT6_LEN..ENDPOINT6_LEN + buf.len()].copy_from_slice(buf);
        let deadline = if self.inner.nonblocking {
            None
        } else {
            deadline_from(self.inner.write_timeout)?
        };
        loop {
            let res = self
                .inner
                .op(OP_SEND_TO, &packet[..ENDPOINT6_LEN + buf.len()], &mut []);
            if res >= 0 {
                return Ok(res as usize);
            }
            if res != libdunit::EAGAIN || self.inner.nonblocking {
                return Err(Error(res));
            }
            self.inner
                .wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
        }
    }

    pub fn send(&self, buf: &[u8]) -> Result<usize> {
        if buf.len() > MAX_UDP_PAYLOAD {
            return Err(Error(libdunit::EMSGSIZE));
        }
        let remote = self.inner.endpoints6()?.remote;
        if remote.port == 0 {
            return Err(Error(libdunit::EINVAL));
        }
        if buf.is_empty() {
            return self.send_to(buf, SocketAddr::from_wire6(remote)?);
        }
        let deadline = if self.inner.nonblocking {
            None
        } else {
            deadline_from(self.inner.write_timeout)?
        };
        loop {
            let res = libdunit::write(self.inner.fd, buf);
            if res >= 0 {
                return Ok(res as usize);
            }
            if res != libdunit::EAGAIN || self.inner.nonblocking {
                return Err(Error(res));
            }
            self.inner
                .wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
        }
    }

    pub fn recv_from_addr_full(&self, buf: &mut [u8]) -> Result<(usize, usize, SocketAddr)> {
        let cap = buf.len().min(MAX_UDP_PAYLOAD);
        let mut record = [0u8; RECV_HEADER6_LEN + MAX_UDP_PAYLOAD];
        let deadline = if self.inner.nonblocking {
            None
        } else {
            deadline_from(self.inner.read_timeout)?
        };
        loop {
            let res = self
                .inner
                .op(OP_RECV_FROM, &[], &mut record[..RECV_HEADER6_LEN + cap]);
            if res >= 0 {
                let n = res as usize;
                if n < RECV_HEADER6_LEN || n > RECV_HEADER6_LEN + cap {
                    return Err(Error(libdunit::EIO));
                }
                let header = RecvHeader6::decode(&record[..RECV_HEADER6_LEN])
                    .map_err(|_| Error(libdunit::EIO))?;
                let copied = n - RECV_HEADER6_LEN;
                if copied > 0 {
                    buf[..copied].copy_from_slice(&record[RECV_HEADER6_LEN..n]);
                }
                return Ok((
                    copied,
                    header.full_len as usize,
                    SocketAddr::from_wire6(header.endpoint)?,
                ));
            }
            if res != libdunit::EAGAIN || self.inner.nonblocking {
                return Err(Error(res));
            }
            self.inner
                .wait_for(event::READ, deadline, libdunit::EAGAIN)?;
        }
    }

    pub fn recv_from_full(&self, buf: &mut [u8]) -> Result<(usize, usize, SocketAddrV4)> {
        let (copied, full, addr) = self.recv_from_addr_full(buf)?;
        Ok((copied, full, addr.into_v4()?))
    }

    pub fn recv_from_addr(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        let (copied, _, addr) = self.recv_from_addr_full(buf)?;
        Ok((copied, addr))
    }

    pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddrV4)> {
        let (copied, _, addr) = self.recv_from_full(buf)?;
        Ok((copied, addr))
    }

    pub fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        let (copied, _, _) = self.recv_from_addr_full(buf)?;
        Ok(copied)
    }

    pub fn local_socket_addr(&self) -> Result<SocketAddr> {
        SocketAddr::from_wire6(self.inner.endpoints6()?.local)
    }

    pub fn local_addr(&self) -> Result<SocketAddrV4> {
        self.local_socket_addr()?.into_v4()
    }

    pub fn peer_socket_addr(&self) -> Result<Option<SocketAddr>> {
        let ep = self.inner.endpoints6()?.remote;
        if ep.port == 0 {
            Ok(None)
        } else {
            Ok(Some(SocketAddr::from_wire6(ep)?))
        }
    }

    pub fn peer_addr(&self) -> Result<Option<SocketAddrV4>> {
        match self.peer_socket_addr()? {
            None => Ok(None),
            Some(addr) => Ok(Some(addr.into_v4()?)),
        }
    }

    pub fn set_nonblocking(&mut self, nonblocking: bool) {
        self.inner.set_nonblocking(nonblocking);
    }

    pub fn is_nonblocking(&self) -> bool {
        self.inner.nonblocking
    }

    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.inner.set_read_timeout(timeout)
    }

    pub fn read_timeout(&self) -> Option<Duration> {
        self.inner.read_timeout
    }

    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.inner.set_write_timeout(timeout)
    }

    pub fn write_timeout(&self) -> Option<Duration> {
        self.inner.write_timeout
    }

    pub fn take_error(&self) -> Result<Option<Error>> {
        self.inner.take_error()
    }

    pub fn descriptor(&self) -> u32 {
        self.inner.fd as u32
    }
}

pub fn resolve_addrs(grant: &NetGrant, host: &str) -> Result<Vec<IpAddr>> {
    resolve_addrs_timeout(grant, host, DNS_TIMEOUT)
}

pub fn resolve_addrs_timeout(
    grant: &NetGrant,
    host: &str,
    timeout: Duration,
) -> Result<Vec<IpAddr>> {
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = unbracketed.parse::<IpAddr>() {
        return Ok(alloc::vec![ip]);
    }
    if host.is_empty() || host.len() > MAX_DNS_NAME {
        return Err(Error(libdunit::EINVAL));
    }
    validate_timeout(Some(timeout))?;
    let deadline = deadline_from(Some(timeout))?;
    let sock = SocketFd::open(grant, PROTO_DNS, timeout.min(OPEN_TIMEOUT))?;
    let query = host.as_bytes();
    let mut written = 0;
    while written < query.len() {
        let res = libdunit::write(sock.fd, &query[written..]);
        if res > 0 {
            written += res as usize;
        } else if res == libdunit::EAGAIN {
            sock.wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
        } else {
            return Err(Error(res));
        }
    }
    let mut buf = [0u8; DNS_RECORDS_HEADER_LEN + MAX_DNS_RECORDS * DNS_RECORD_LEN];
    loop {
        let res = libdunit::read(sock.fd, &mut buf);
        if res == 4 {
            let v4: [u8; 4] = buf[..4].try_into().map_err(|_| Error(libdunit::EIO))?;
            return Ok(alloc::vec![IpAddr::V4(Ipv4Addr(v4))]);
        }
        if res > 0 {
            let len = res as usize;
            let mut records = [DnsRecord::default(); MAX_DNS_RECORDS];
            let count = decode_dns_records(&buf[..len], &mut records)
                .map_err(|_| Error(libdunit::EIO))?;
            let mut out = Vec::with_capacity(count);
            for rec in &records[..count] {
                match rec.family {
                    FAMILY_IPV6 => out.push(IpAddr::V6(Ipv6Addr(rec.addr))),
                    FAMILY_IPV4 => {
                        let v4: [u8; 4] =
                            rec.addr[..4].try_into().map_err(|_| Error(libdunit::EIO))?;
                        out.push(IpAddr::V4(Ipv4Addr(v4)));
                    }
                    _ => {}
                }
            }
            return if out.is_empty() {
                Err(Error(libdunit::ENOENT))
            } else {
                Ok(out)
            };
        }
        if res == 0 {
            return Err(Error(libdunit::EIO));
        }
        if res != libdunit::EAGAIN {
            return Err(Error(res));
        }
        sock.wait_for(event::READ, deadline, libdunit::EAGAIN)?;
        if let Some(err) = sock.take_error()? {
            return Err(err);
        }
    }
}

pub fn resolve_ipv4(grant: &NetGrant, host: &str) -> Result<Ipv4Addr> {
    resolve_ipv4_timeout(grant, host, DNS_TIMEOUT)
}

pub fn resolve_ipv4_timeout(grant: &NetGrant, host: &str, timeout: Duration) -> Result<Ipv4Addr> {
    for addr in resolve_addrs_timeout(grant, host, timeout)? {
        if let IpAddr::V4(v4) = addr {
            return Ok(v4);
        }
    }
    Err(Error(libdunit::ENOENT))
}

fn query_status_cmd(grant: &NetGrant, cmd: &[u8], timeout: Duration) -> Result<String> {
    validate_timeout(Some(timeout))?;
    let deadline = deadline_from(Some(timeout))?;
    let sock = SocketFd::open(grant, PROTO_STATUS, timeout.min(OPEN_TIMEOUT))?;
    let mut written = 0;
    while written < cmd.len() {
        let res = libdunit::write(sock.fd, &cmd[written..]);
        if res > 0 {
            written += res as usize;
        } else if res == libdunit::EAGAIN {
            sock.wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
        } else {
            return Err(Error(res));
        }
    }
    let mut buf = alloc::vec![0u8; MAX_PAYLOAD];
    loop {
        let res = libdunit::read(sock.fd, &mut buf);
        if res > 0 {
            let text = core::str::from_utf8(&buf[..res as usize])
                .map_err(|_| Error(libdunit::EIO))?;
            return Ok(String::from(text));
        }
        if res == 0 {
            return Err(Error(libdunit::EIO));
        }
        if res != libdunit::EAGAIN {
            return Err(Error(res));
        }
        sock.wait_for(event::READ, deadline, libdunit::EAGAIN)?;
        if let Some(err) = sock.take_error()? {
            return Err(err);
        }
    }
}

pub fn query_status(grant: &NetGrant, timeout: Duration) -> Result<String> {
    query_status_cmd(grant, b"status", timeout)
}

pub fn query_routes(grant: &NetGrant, timeout: Duration) -> Result<String> {
    query_status_cmd(grant, b"routes", timeout)
}

pub struct IcmpSocket {
    sock: SocketFd,
}

impl IcmpSocket {
    pub fn open(grant: &NetGrant) -> Result<Self> {
        Ok(Self {
            sock: SocketFd::open(grant, PROTO_ICMP, OPEN_TIMEOUT)?,
        })
    }

    pub fn ping(
        &mut self,
        target: IpAddr,
        sequence: u16,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<(IpAddr, u16, Duration)> {
        if payload.len() > MAX_ICMP_PAYLOAD {
            return Err(Error(libdunit::EMSGSIZE));
        }
        validate_timeout(Some(timeout))?;
        let started = Instant::now()?;
        let deadline = deadline_from(Some(timeout))?;
        let ep = match target {
            IpAddr::V4(v4) => Endpoint6::ipv4(v4.0, sequence),
            IpAddr::V6(v6) => Endpoint6::ipv6(v6.0, sequence, 0),
        };
        let mut req = [0u8; ENDPOINT6_LEN + MAX_ICMP_PAYLOAD];
        req[..ENDPOINT6_LEN].copy_from_slice(&ep.encode());
        req[ENDPOINT6_LEN..ENDPOINT6_LEN + payload.len()].copy_from_slice(payload);
        let send_len = ENDPOINT6_LEN + payload.len();
        loop {
            let res = self.sock.op(OP_SEND_TO, &req[..send_len], &mut []);
            if res >= 0 {
                break;
            }
            if res != libdunit::EAGAIN {
                return Err(Error(res));
            }
            self.sock.wait_for(event::WRITE, deadline, libdunit::EAGAIN)?;
        }
        let mut rx = [0u8; RECV_HEADER6_LEN + MAX_ICMP_PAYLOAD];
        loop {
            let res = self.sock.op(OP_RECV_FROM, &[], &mut rx);
            if res > 0 {
                let n = res as usize;
                if n < RECV_HEADER6_LEN {
                    return Err(Error(libdunit::EIO));
                }
                let hdr = RecvHeader6::decode(&rx[..RECV_HEADER6_LEN])
                    .map_err(|_| Error(libdunit::EIO))?;
                let body_len = n - RECV_HEADER6_LEN;
                if body_len != payload.len() || &rx[RECV_HEADER6_LEN..n] != payload {
                    continue;
                }
                if hdr.endpoint.port != sequence {
                    continue;
                }
                let from = match hdr.endpoint.family {
                    FAMILY_IPV4 => {
                        let v4: [u8; 4] = hdr.endpoint.addr[..4]
                            .try_into()
                            .map_err(|_| Error(libdunit::EIO))?;
                        IpAddr::V4(Ipv4Addr(v4))
                    }
                    FAMILY_IPV6 => IpAddr::V6(Ipv6Addr(hdr.endpoint.addr)),
                    _ => return Err(Error(libdunit::EIO)),
                };
                return Ok((from, hdr.endpoint.port, started.elapsed()?));
            }
            if res == 0 {
                return Err(Error(libdunit::EIO));
            }
            if res != libdunit::EAGAIN {
                return Err(Error(res));
            }
            self.sock.wait_for(event::READ, deadline, libdunit::EAGAIN)?;
            if let Some(err) = self.sock.take_error()? {
                return Err(err);
            }
        }
    }
}
