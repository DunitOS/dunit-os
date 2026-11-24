//! Native IPv4 TCP/UDP sockets over authorized `NET_CONNECT` handles.
use crate::{
    error::checked,
    event::{self, Interest},
    io::{Read, Write},
    time::{timeout_ns, Duration, Instant},
    Error, Result,
};
use core::{fmt, str::FromStr};
use dunit_socket_abi::{
    Endpoint, Endpoints, Options, RecvHeader, ENDPOINTS_LEN, ENDPOINT_LEN, MAX_PAYLOAD,
    OP_CONNECT, OP_ENDPOINTS, OP_ERROR, OP_RECV_FROM, OP_SEND_TO, OP_SHUTDOWN_WRITE, PROTO_TCP,
    PROTO_UDP, RECV_HEADER_LEN,
};

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
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

    fn to_wire(self) -> Endpoint {
        Endpoint {
            ip: self.ip.0,
            port: self.port,
        }
    }

    fn from_wire(endpoint: Endpoint) -> Self {
        Self::new(Ipv4Addr(endpoint.ip), endpoint.port)
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
            &Options.encode(),
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

    fn endpoints(&self) -> Result<Endpoints> {
        let mut raw = [0u8; ENDPOINTS_LEN];
        let n = checked(self.op(OP_ENDPOINTS, &[], &mut raw))?;
        if n != ENDPOINTS_LEN {
            return Err(Error(libdunit::EIO));
        }
        Endpoints::decode(&raw).map_err(|_| Error(libdunit::EIO))
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
    pub fn connect(grant: &NetGrant, addr: SocketAddrV4) -> Result<Self> {
        Self::connect_inner(grant, addr, None)
    }

    pub fn connect_timeout(grant: &NetGrant, addr: SocketAddrV4, timeout: Duration) -> Result<Self> {
        validate_timeout(Some(timeout))?;
        Self::connect_inner(grant, addr, Some(timeout))
    }

    fn connect_inner(
        grant: &NetGrant,
        addr: SocketAddrV4,
        timeout: Option<Duration>,
    ) -> Result<Self> {
        let open_limit = timeout.map_or(OPEN_TIMEOUT, |t| t.min(OPEN_TIMEOUT));
        let deadline = deadline_from(timeout)?;
        let inner = SocketFd::open(grant, PROTO_TCP, open_limit)?;
        let res = inner.op(OP_CONNECT, &addr.to_wire().encode(), &mut []);
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
            if inner.endpoints()?.remote.port != 0 {
                return Ok(Self { inner });
            }
        }
    }

    pub fn local_addr(&self) -> Result<SocketAddrV4> {
        Ok(SocketAddrV4::from_wire(self.inner.endpoints()?.local))
    }

    pub fn peer_addr(&self) -> Result<SocketAddrV4> {
        let ep = self.inner.endpoints()?.remote;
        if ep.port == 0 {
            return Err(Error(libdunit::EINVAL));
        }
        Ok(SocketAddrV4::from_wire(ep))
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

pub struct UdpSocket {
    inner: SocketFd,
}

impl UdpSocket {
    pub fn open(grant: &NetGrant) -> Result<Self> {
        Ok(Self {
            inner: SocketFd::open(grant, PROTO_UDP, OPEN_TIMEOUT)?,
        })
    }

    pub fn connect(&self, addr: SocketAddrV4) -> Result<()> {
        let deadline = if self.inner.nonblocking {
            None
        } else {
            deadline_from(self.inner.write_timeout)?
        };
        let wire = addr.to_wire().encode();
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
            if self.inner.endpoints()?.remote.port != 0 {
                return Ok(());
            }
        }
    }

    pub fn send_to(&self, buf: &[u8], addr: SocketAddrV4) -> Result<usize> {
        if buf.len() > MAX_UDP_PAYLOAD {
            return Err(Error(libdunit::EMSGSIZE));
        }
        let mut packet = [0u8; ENDPOINT_LEN + MAX_UDP_PAYLOAD];
        packet[..ENDPOINT_LEN].copy_from_slice(&addr.to_wire().encode());
        packet[ENDPOINT_LEN..ENDPOINT_LEN + buf.len()].copy_from_slice(buf);
        let deadline = if self.inner.nonblocking {
            None
        } else {
            deadline_from(self.inner.write_timeout)?
        };
        loop {
            let res = self
                .inner
                .op(OP_SEND_TO, &packet[..ENDPOINT_LEN + buf.len()], &mut []);
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
        let remote = self.inner.endpoints()?.remote;
        if remote.port == 0 {
            return Err(Error(libdunit::EINVAL));
        }
        if buf.is_empty() {
            return self.send_to(buf, SocketAddrV4::from_wire(remote));
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

    pub fn recv_from_full(&self, buf: &mut [u8]) -> Result<(usize, usize, SocketAddrV4)> {
        let cap = buf.len().min(MAX_UDP_PAYLOAD);
        let mut record = [0u8; RECV_HEADER_LEN + MAX_UDP_PAYLOAD];
        let deadline = if self.inner.nonblocking {
            None
        } else {
            deadline_from(self.inner.read_timeout)?
        };
        loop {
            let res = self
                .inner
                .op(OP_RECV_FROM, &[], &mut record[..RECV_HEADER_LEN + cap]);
            if res >= 0 {
                let n = res as usize;
                if n < RECV_HEADER_LEN || n > RECV_HEADER_LEN + cap {
                    return Err(Error(libdunit::EIO));
                }
                let header = RecvHeader::decode(&record[..RECV_HEADER_LEN])
                    .map_err(|_| Error(libdunit::EIO))?;
                let copied = n - RECV_HEADER_LEN;
                if copied > 0 {
                    buf[..copied].copy_from_slice(&record[RECV_HEADER_LEN..n]);
                }
                return Ok((
                    copied,
                    header.full_len as usize,
                    SocketAddrV4::from_wire(header.endpoint),
                ));
            }
            if res != libdunit::EAGAIN || self.inner.nonblocking {
                return Err(Error(res));
            }
            self.inner
                .wait_for(event::READ, deadline, libdunit::EAGAIN)?;
        }
    }

    pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddrV4)> {
        let (copied, _, addr) = self.recv_from_full(buf)?;
        Ok((copied, addr))
    }

    pub fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        let (copied, _, _) = self.recv_from_full(buf)?;
        Ok(copied)
    }

    pub fn local_addr(&self) -> Result<SocketAddrV4> {
        Ok(SocketAddrV4::from_wire(self.inner.endpoints()?.local))
    }

    pub fn peer_addr(&self) -> Result<Option<SocketAddrV4>> {
        let ep = self.inner.endpoints()?.remote;
        Ok((ep.port != 0).then(|| SocketAddrV4::from_wire(ep)))
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
