//! Native application listener authority checks over the isolated E1000 peer.
#![no_std]
#![no_main]

use dunit::{
    errno,
    io::{Read, Write},
    net::{Ipv4Addr, NetGrant, SocketAddrV4, TcpListener, TcpStream, UdpSocket},
    time::{self, Duration, Instant},
    Error, Result,
};
use libdunit::{self as sys, wait_set};

const TIMEOUT: Duration = Duration::from_secs(25);
const PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 2), 55111);

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NET-LISTEN] FAIL {check}")?;
    Err(Error(errno::EIO))
}

fn receive_grant(started: Instant) -> Result<NetGrant> {
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
            let rem = TIMEOUT.checked_sub(started.elapsed()?).unwrap_or_default();
            require(!rem.is_zero(), "grant deadline")?;
            wait_set::wait(
                &[wait_set::Interest::ipc(0)],
                &mut [wait_set::Event::default()],
                rem.as_nanos() as u64,
            )
            .map_err(Error)?;
            continue;
        }
        if got < 0 {
            return Err(Error(got));
        }
        if attachment != 0 {
            sys::handle_close(attachment);
        }
        if sender == supervisor && attachment == 0 && got == 4 {
            let handle = u32::from_le_bytes(payload[..4].try_into().unwrap());
            require(handle != 0, "zero grant")?;
            return NetGrant::from_raw_handle(handle);
        }
    }
}

fn bind_ready(grant: &NetGrant, addr: SocketAddrV4) -> Result<TcpListener> {
    loop {
        match TcpListener::bind(grant, addr) {
            Ok(l) => return Ok(l),
            Err(Error(errno::EPIPE | errno::EIO | errno::EAGAIN)) => {
                time::sleep(Duration::from_millis(20))?
            }
            Err(err) => return Err(err),
        }
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let started = Instant::now()?;
    let grant = receive_grant(started)?;

    let listener = bind_ready(&grant, SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8080))?;
    // Listener-only grant (scope = 12) must deny outbound connect and low ports.
    require(
        matches!(TcpStream::connect(&grant, PEER), Err(Error(errno::EPERM))),
        "outbound TCP denied without SCOPE_TCP",
    )?;
    require(
        listener.local_addr()? == SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 12), 8080),
        "listener local_addr",
    )?;
    require(
        matches!(
            TcpListener::bind(&grant, SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 80)),
            Err(Error(errno::EACCES))
        ),
        "low port denied without SCOPE_LOW_PORT",
    )?;
    require(
        matches!(
            TcpListener::bind(&grant, SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8080)),
            Err(Error(errno::EBUSY))
        ),
        "duplicate port rejected",
    )?;

    let eph = TcpListener::bind(&grant, SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))?;
    require(eph.local_addr()?.port() >= 49152, "ephemeral listener port")?;
    drop(eph);

    let mut udp = UdpSocket::bind(&grant, SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 9090))?;
    udp.set_read_timeout(Some(TIMEOUT))?;
    udp.set_write_timeout(Some(TIMEOUT))?;
    require(
        udp.local_addr()? == SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 12), 9090),
        "bound UDP local_addr",
    )?;
    dunit::println!("[NET-LISTEN] scope and bind checks passed")?;

    let (mut stream, remote) = listener.accept_timeout(TIMEOUT)?;
    require(remote == PEER && stream.peer_addr()? == PEER, "accepted peer endpoint")?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    let mut buf = [0u8; 32];
    let n = stream.read(&mut buf)?;
    require(&buf[..n] == b"hello-listen", "TCP accepted payload")?;
    stream.write_all(b"listen-ack")?;
    stream.shutdown_write()?;
    require(stream.read(&mut buf)? == 0, "TCP accepted EOF")?;
    drop(stream);
    dunit::println!("[NET-LISTEN] TCP accept and echo passed")?;

    let (un, src) = udp.recv_from(&mut buf)?;
    require(
        &buf[..un] == b"udp-listen-in" && src.ip() == PEER.ip(),
        "UDP bound recv_from",
    )?;
    udp.send_to(b"udp-listen-out", src)?;
    dunit::println!("[NET-LISTEN] UDP bound echo passed")?;

    loop {
        match listener.accept_timeout(Duration::from_millis(500)) {
            Err(Error(errno::EPIPE)) => break,
            Err(Error(errno::EAGAIN)) => time::sleep(Duration::from_millis(20))?,
            Ok(_) => return Err(Error(errno::EIO)),
            Err(err) => return Err(err),
        }
    }
    dunit::println!("[NET-LISTEN] revocation rejected listener with EPIPE")?;
    dunit::println!("[NET-LISTEN] complete")
}

