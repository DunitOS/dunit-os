//! Native application socket checks for the isolated E1000 peer.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, vec};
use dunit::{
    errno,
    io::{Read, Write},
    net::{Ipv4Addr, NetGrant, SocketAddrV4, TcpStream},
    process::{self, Child},
    time::{self, Duration, Instant},
    Error, Result,
};
use dunit_socket_abi::{
    Endpoint, Options, RecvHeader, ENDPOINTS_LEN, ENDPOINT_LEN, OP_CONNECT, OP_ENDPOINTS, OP_ERROR,
    OP_RECV_FROM, OP_SEND_TO, PROTO_TCP, PROTO_UDP, RECV_HEADER_LEN, SCOPE_TCP, SCOPE_UDP,
};
use libdunit::{self as sys, wait_set};

const PATH: &str = "/system/libexec/net_socket_test";
const HOST: [u8; 4] = [192, 0, 2, 1];
const DELAYED_HOST: [u8; 4] = [192, 0, 2, 53];
const GUEST: [u8; 4] = [192, 0, 2, 12];
const UDP_PORT: u16 = 5433;
const TCP_PORT: u16 = 5432;
const UDP_REQUEST: &[u8] = b"fixture-udp-request-v1";
const UDP_REPLY: &[u8] = b"fixture-udp-truncated-v1";
const TCP_REQUEST_LEN: usize = 5000;
const TCP_REPLY: &[u8] = b"fixture-tcp-partial-read-and-EOF";
const TIMEOUT: Duration = Duration::from_secs(25);
static READ_ONLY: [u8; RECV_HEADER_LEN + 5] = [0xa5; RECV_HEADER_LEN + 5];

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NET-SOCKET] FAIL {check}")?;
    Err(Error(errno::EIO))
}

fn checked(result: isize, check: &str) -> Result<usize> {
    if result < 0 {
        dunit::println!("[NET-SOCKET] FAIL {check} errno={result}")?;
        return Err(Error(result));
    }
    Ok(result as usize)
}

fn left(started: Instant) -> Result<Duration> {
    let remaining = TIMEOUT.checked_sub(started.elapsed()?).unwrap_or_default();
    if remaining.is_zero() {
        require(false, "fixture deadline")?;
    }
    Ok(remaining)
}

struct Grant(u32);

impl Drop for Grant {
    fn drop(&mut self) {
        sys::handle_close(self.0);
    }
}

fn receive_grant(started: Instant) -> Result<Grant> {
    let supervisor = dunit::env::var("DUNIT_NET_SUPERVISOR")?
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid != 0)
        .ok_or(Error(errno::EACCES))?;
    let mut payload = [0; 256];
    loop {
        let mut sender = 0;
        let mut attachment = 0;
        let got = sys::ipc_recv_handle(&mut payload, &mut sender, &mut attachment);
        if got == errno::EAGAIN {
            wait_set::wait(
                &[wait_set::Interest::ipc(0)],
                &mut [wait_set::Event::default()],
                left(started)?.as_nanos() as u64,
            )
            .map_err(Error)?;
            continue;
        }
        let length = checked(got, "receive grant")?;
        if attachment != 0 {
            sys::handle_close(attachment);
        }
        if sender == supervisor && attachment == 0 && length == 4 {
            let handle = u32::from_le_bytes(payload[..4].try_into().unwrap());
            require(handle != 0, "zero grant")?;
            return Ok(Grant(handle));
        }
        left(started)?;
    }
}

struct Socket(usize);

impl Socket {
    fn open(grant: &Grant, protocol: u16, started: Instant) -> Result<Self> {
        loop {
            let fd = sys::net_socket_open(grant.0, protocol, &Options.encode());
            if matches!(fd, errno::ENOENT | errno::EAGAIN | errno::EPIPE) {
                time::sleep(left(started)?.min(Duration::from_millis(20)))?;
            } else {
                return checked(fd, "socket open").map(Self);
            }
        }
    }

    fn operation(&self, op: u16, input: &[u8], output: &mut [u8]) -> isize {
        sys::net_socket_op(self.0, op, input, output)
    }

    fn wait(&self, flags: u32, started: Instant, check: &str) -> Result<()> {
        let mut ready = [wait_set::Event::default()];
        let count = wait_set::wait(
            &[wait_set::Interest::descriptor(self.0 as u32, flags, 0)],
            &mut ready,
            left(started)?.as_nanos() as u64,
        )
        .map_err(Error)?;
        if count != 1
            || ready[0].events & (flags | wait_set::HANGUP) == 0
            || ready[0].events & wait_set::ERROR != 0
        {
            dunit::println!(
                "[NET-SOCKET] FAIL {check} events={:#x} count={count}",
                ready[0].events
            )?;
            return Err(Error(errno::EIO));
        }
        Ok(())
    }

    fn poll_snapshot(&self, mask: u16) -> Result<u16> {
        let mut record = [0u8; 8];
        let fd = i32::try_from(self.0).map_err(|_| Error(errno::EINVAL))?;
        record[..4].copy_from_slice(&fd.to_le_bytes());
        record[4..6].copy_from_slice(&mask.to_le_bytes());
        checked(
            sys::syscall3(sys::SYSCALL_POLL, record.as_mut_ptr() as usize, 1, 0),
            "native poll",
        )?;
        Ok(u16::from_le_bytes(record[6..].try_into().unwrap()))
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        sys::close(self.0);
    }
}

fn deny_inherited(fd: usize, grant: u32) -> Result<()> {
    require(
        sys::read(fd, &mut [0; 1]) == errno::EBADF,
        "child inherited socket FD",
    )?;
    require(
        sys::net_socket_open(grant, PROTO_UDP, &Options.encode()) < 0,
        "child inherited connect grant",
    )?;
    dunit::println!("[NET-SOCKET] child denied inherited fd and grant")
}

fn check_isolation(socket: &Socket, grant: &Grant) -> Result<()> {
    require(
        sys::net_connect_grant(grant.0, process::id(), SCOPE_TCP | SCOPE_UDP) < 0,
        "connect grant used as supervisor root",
    )?;
    require(
        sys::handle_dup(grant.0, sys::RIGHT_NET_CONNECT | sys::RIGHT_TRANSFER) < 0,
        "grant right widening",
    )?;
    require(
        sys::ipc_send_handle(process::id(), grant.0, sys::RIGHT_NET_CONNECT, &[0; 4], 0) < 0,
        "grant transferred over IPC",
    )?;
    require(
        sys::syscall3(sys::SYSCALL_FCNTL, socket.0, 1, 0) == 0,
        "clear close-on-exec on socket FD",
    )?;
    let fd = format!("{}", socket.0);
    let handle = format!("{}", grant.0);
    let mut child = Child::spawn_with_env(PATH, &[PATH, "deny", &fd, &handle], &[])?;
    require(child.wait()?.success(), "child authority denial")
}

fn udp(grant: &Grant) -> Result<Socket> {
    let started = Instant::now()?;
    let original = Socket::open(grant, PROTO_UDP, started)?;
    check_isolation(&original, grant)?;
    let socket = Socket(checked(
        sys::syscall3(sys::SYSCALL_FCNTL, original.0, 5, 3),
        "same-process UDP dup",
    )?);
    require(socket.0 != original.0, "UDP dup reused original descriptor")?;
    drop(original);

    let endpoint = Endpoint {
        ip: HOST,
        port: UDP_PORT,
    };
    let mut packet = [0u8; ENDPOINT_LEN + UDP_REQUEST.len()];
    packet[..ENDPOINT_LEN].copy_from_slice(&endpoint.encode());
    packet[ENDPOINT_LEN..].copy_from_slice(UDP_REQUEST);
    loop {
        let sent = socket.operation(OP_SEND_TO, &packet, &mut []);
        if sent == errno::EAGAIN {
            socket.wait(wait_set::WRITE, started, "UDP writable")?;
            continue;
        }
        require(
            checked(sent, "UDP send-to")? == UDP_REQUEST.len(),
            "UDP send-to count",
        )?;
        break;
    }
    socket.wait(wait_set::READ, started, "UDP readable")?;
    require(
        socket.poll_snapshot(wait_set::READ as u16)? & wait_set::READ as u16 != 0,
        "UDP poll READ",
    )?;
    let fault = sys::syscall6(
        sys::SYSCALL_NET_SOCKET_OP,
        socket.0,
        OP_RECV_FROM as usize,
        0,
        0,
        READ_ONLY.as_ptr() as usize,
        READ_ONLY.len(),
    );
    require(fault == errno::EFAULT, "UDP read-only copyout errno")?;
    require(
        socket.poll_snapshot(wait_set::READ as u16)? & wait_set::READ as u16 != 0,
        "failed UDP copyout consumed datagram",
    )?;

    let mut output = [0u8; RECV_HEADER_LEN + 5];
    require(
        checked(
            socket.operation(OP_RECV_FROM, &[], &mut output),
            "UDP recv-from",
        )? == output.len(),
        "UDP truncated receive count",
    )?;
    let header = RecvHeader::decode(&output[..RECV_HEADER_LEN]).map_err(|_| Error(errno::EIO))?;
    require(
        header.endpoint == endpoint
            && header.full_len as usize == UDP_REPLY.len()
            && output[RECV_HEADER_LEN..] == UDP_REPLY[..5],
        "UDP source or full length",
    )?;
    require(
        socket.operation(OP_RECV_FROM, &[], &mut output) == errno::EAGAIN,
        "UDP truncation left trailing bytes",
    )?;
    dunit::println!("[NET-SOCKET] UDP validated truncation and copyout retry")?;
    Ok(socket)
}

fn connect_tcp(grant: &Grant, ip: [u8; 4], started: Instant) -> Result<Socket> {
    let socket = Socket::open(grant, PROTO_TCP, started)?;
    let endpoint = Endpoint {
        ip,
        port: TCP_PORT,
    };
    loop {
        let result = socket.operation(OP_CONNECT, &endpoint.encode(), &mut []);
        if result != errno::EAGAIN {
            require(checked(result, "TCP connect")? == 0, "TCP connect result")?;
            break;
        }
        // OPEN can still be processing when the first CONNECT is issued.
        // A short wait lets that operation retire before submitting CONNECT.
        wait_set::wait(
            &[wait_set::Interest::descriptor(
                socket.0 as u32,
                wait_set::WRITE,
                0,
            )],
            &mut [wait_set::Event::default()],
            left(started)?.min(Duration::from_millis(20)).as_nanos() as u64,
        )
        .map_err(Error)?;
    }
    socket.wait(wait_set::WRITE, started, "TCP connect completion")?;
    let mut error = [0; 4];
    require(
        checked(
            socket.operation(OP_ERROR, &[], &mut error),
            "TCP error query",
        )? == 4
            && i32::from_le_bytes(error) == 0,
        "TCP connect error",
    )?;
    Ok(socket)
}

fn tcp_reset(grant: &Grant) -> Result<Socket> {
    let started = Instant::now()?;
    let socket = connect_tcp(grant, DELAYED_HOST, started)?;
    require(
        checked(sys::write(socket.0, b"r"), "TCP reset synchronization")? == 1,
        "TCP reset synchronization byte",
    )?;
    let mut events = [wait_set::Event::default()];
    let count = wait_set::wait(
        &[wait_set::Interest::descriptor(
            socket.0 as u32,
            wait_set::READ,
            0,
        )],
        &mut events,
        left(started)?.as_nanos() as u64,
    )
    .map_err(Error)?;
    let terminal = wait_set::HANGUP | wait_set::ERROR;
    require(
        count == 1 && events[0].events & terminal == terminal,
        "TCP reset wait-set",
    )?;
    let mut error = [0; 4];
    require(
        socket.operation(OP_ERROR, &[], &mut error) == 4
            && i32::from_le_bytes(error) as isize == errno::EPIPE
            && sys::read(socket.0, &mut [0; 1]) == errno::EPIPE,
        "TCP reset error",
    )?;
    dunit::println!("[NET-SOCKET] TCP reset woke wait-set and returned EPIPE")?;
    Ok(socket)
}

fn tcp(grant_handle: u32) -> Result<()> {
    let grant = NetGrant::from_raw_handle(grant_handle)?;
    let remote = SocketAddrV4::new(Ipv4Addr::from(HOST), TCP_PORT);
    let mut stream = loop {
        match TcpStream::connect_timeout(&grant, remote, TIMEOUT) {
            Ok(stream) => break stream,
            Err(Error(err)) if matches!(err, errno::ENOENT | errno::EAGAIN | errno::EPIPE) => {
                time::sleep(Duration::from_millis(20))?;
            }
            Err(error) => {
                let _ = grant.into_raw_handle();
                return Err(error);
            }
        }
    };
    let _ = grant.into_raw_handle();
    require(stream.take_error()?.is_none(), "TCP connect error")?;
    let peer = stream.peer_addr()?;
    let local = stream.local_addr()?;
    require(
        peer == remote && local.ip() == Ipv4Addr::from(GUEST) && local.port() >= 49152,
        "TCP endpoint identity",
    )?;

    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    let mut request = vec![0u8; TCP_REQUEST_LEN];
    for (index, byte) in request.iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }
    stream.set_nonblocking(true);
    let first = stream.write(&request)?;
    require(first != 0 && first < request.len(), "TCP never returned a partial write")?;
    stream.set_nonblocking(false);
    stream.write_all(&request[first..])?;

    let mut received = 0;
    let mut reads = 0;
    let mut chunk = [0; 7];
    loop {
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        require(
            count <= chunk.len() && received + count <= TCP_REPLY.len(),
            "TCP read overflow",
        )?;
        require(
            chunk[..count] == TCP_REPLY[received..received + count],
            "TCP stream data",
        )?;
        received += count;
        reads += 1;
    }
    require(
        received == TCP_REPLY.len() && reads > 1,
        "TCP partial data before EOF",
    )?;
    stream.shutdown_write()?;
    dunit::println!("[NET-SOCKET] TCP validated partial I/O and EOF")
}

fn revoked(socket: &Socket) -> Result<()> {
    let mut events = [wait_set::Event::default()];
    let count = wait_set::wait(
        &[wait_set::Interest::descriptor(
            socket.0 as u32,
            wait_set::READ,
            0,
        )],
        &mut events,
        TIMEOUT.as_nanos() as u64,
    )
    .map_err(Error)?;
    let terminal = wait_set::HANGUP | wait_set::ERROR;
    require(
        count == 1 && events[0].events & terminal == terminal,
        "interface withdrawal did not wake idle socket with HANGUP and ERROR",
    )?;
    let mut address = [0u8; ENDPOINTS_LEN];
    require(
        socket.operation(OP_ENDPOINTS, &[], &mut address) == errno::EPIPE,
        "old socket endpoint query survived service-generation revoke",
    )?;
    require(
        sys::read(socket.0, &mut [0; 1]) == errno::EPIPE,
        "old socket read survived service-generation revoke",
    )?;
    dunit::println!("[NET-SOCKET] address withdrawal revoked socket and woke wait-set")
}

#[dunit::main]
fn main() -> Result<()> {
    let args = dunit::args::args()?;
    if args.get(1).is_some_and(|mode| mode == "deny") {
        let fd = args
            .get(2)
            .and_then(|fd| fd.parse().ok())
            .ok_or(Error(errno::EINVAL))?;
        let grant = args
            .get(3)
            .and_then(|grant| grant.parse().ok())
            .ok_or(Error(errno::EINVAL))?;
        return deny_inherited(fd, grant);
    }
    require(
        sys::net_socket_open(0, PROTO_UDP, &Options.encode()) < 0,
        "socket open without connect grant",
    )?;
    let grant = receive_grant(Instant::now()?)?;
    let idle_udp = udp(&grant)?;
    let reset = tcp_reset(&grant)?;
    tcp(grant.0)?;
    drop(reset);
    revoked(&idle_udp)?;
    dunit::println!("[NET-SOCKET] complete")
}
