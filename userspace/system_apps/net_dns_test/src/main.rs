//! Native application DNS checks for the isolated E1000 peer.
#![no_std]
#![no_main]

extern crate alloc;

use dunit::{
    errno,
    io::{Read, Write},
    net::{resolve_ipv4_timeout, Ipv4Addr, NetGrant, TcpStream},
    time::{self, Duration, Instant},
    Error, Result,
};
use libdunit::{self as sys, wait_set};

const TIMEOUT: Duration = Duration::from_secs(25);

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NET-DNS] FAIL {check}")?;
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

fn resolve_ready(grant: &NetGrant, host: &str) -> Result<Ipv4Addr> {
    loop {
        match resolve_ipv4_timeout(grant, host, TIMEOUT) {
            Ok(ip) => return Ok(ip),
            Err(Error(err)) if matches!(err, errno::ENOENT | errno::EAGAIN | errno::EPIPE) => {
                time::sleep(Duration::from_millis(20))?;
            }
            Err(error) => return Err(error),
        }
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let started = Instant::now()?;
    let grant = receive_grant(started)?;

    require(
        resolve_ipv4_timeout(&grant, "192.0.2.10", TIMEOUT)? == Ipv4Addr::new(192, 0, 2, 10),
        "numeric IPv4 bypasses network",
    )?;
    require(
        resolve_ready(&grant, "du.net.test")? == Ipv4Addr::new(192, 0, 2, 10),
        "primary UDP DNS resolution",
    )?;
    require(
        resolve_ipv4_timeout(&grant, "du.net.test", TIMEOUT)? == Ipv4Addr::new(192, 0, 2, 10),
        "cached UDP DNS resolution",
    )?;
    dunit::println!("[NET-DNS] UDP and cache passed")?;

    require(
        resolve_ipv4_timeout(&grant, "missing.du.net.test", TIMEOUT) == Err(Error(errno::ENOENT)),
        "NXDOMAIN returns ENOENT",
    )?;
    require(
        resolve_ipv4_timeout(&grant, "missing.du.net.test", TIMEOUT) == Err(Error(errno::ENOENT)),
        "negative DNS cache returns ENOENT",
    )?;
    dunit::println!("[NET-DNS] NXDOMAIN and negative cache passed")?;

    require(
        resolve_ipv4_timeout(&grant, "tcp.du.net.test", TIMEOUT)? == Ipv4Addr::new(192, 0, 2, 20),
        "TCP fallback DNS resolution",
    )?;
    dunit::println!("[NET-DNS] TCP fallback passed")?;

    require(
        resolve_ipv4_timeout(&grant, "failover.du.net.test", TIMEOUT)? == Ipv4Addr::new(192, 0, 2, 30),
        "secondary DNS failover resolution",
    )?;
    dunit::println!("[NET-DNS] secondary failover passed")?;

    let mut stream = TcpStream::connect_host_timeout(&grant, "du.net.test", 5432, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(b"dns-tcp-ok")?;
    let mut reply = [0u8; 32];
    let n = stream.read(&mut reply)?;
    require(&reply[..n] == b"dns-tcp-ack", "TCP connect_host exchange")?;
    require(stream.read(&mut reply)? == 0, "TCP EOF after host exchange")?;
    stream.shutdown_write()?;
    drop(stream);
    dunit::println!("[NET-DNS] connect_host passed")?;

    loop {
        match resolve_ipv4_timeout(&grant, "uncached.du.net.test", Duration::from_millis(500)) {
            Err(Error(errno::EPIPE)) => break,
            Err(Error(errno::EAGAIN)) | Err(Error(errno::ENOENT)) | Ok(_) => {
                time::sleep(Duration::from_millis(20))?;
            }
            Err(err) => return Err(err),
        }
    }
    dunit::println!("[NET-DNS] revocation rejected resolver with EPIPE")?;
    dunit::println!("[NET-DNS] complete")
}

