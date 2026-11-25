//! TCP Reno congestion-control E1000 test: pipelined multi-MSS transfer with packet loss.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use dunit::{
    errno,
    io::{Read, Write},
    net::{Ipv4Addr, NetGrant, SocketAddrV4, TcpStream},
    time::{self, Duration, Instant},
    Error, Result,
};
use libdunit::{self as sys, wait_set};

const TIMEOUT: Duration = Duration::from_secs(25);
const PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 2), 5432);
const TOTAL_BYTES: usize = 16 * 1460;

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NET-RENO] FAIL {check}")?;
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

fn connect_ready(grant: &NetGrant, addr: SocketAddrV4) -> Result<TcpStream> {
    loop {
        match TcpStream::connect_timeout(grant, addr, TIMEOUT) {
            Ok(s) => return Ok(s),
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
    let mut stream = connect_ready(&grant, PEER)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    let mut payload = Vec::with_capacity(TOTAL_BYTES);
    for i in 0..TOTAL_BYTES {
        payload.push(((i * 31) ^ (i >> 8)) as u8);
    }
    stream.write_all(&payload)?;
    stream.shutdown_write()?;

    let mut reply = [0u8; 32];
    let n = stream.read(&mut reply)?;
    require(&reply[..n] == b"reno-23360-ok", "Reno server acknowledgement")?;
    require(stream.read(&mut reply)? == 0, "EOF after Reno transfer")?;
    dunit::println!("[NET-RENO] pipelined transfer and fast retransmit passed")?;

    loop {
        match TcpStream::connect_timeout(&grant, PEER, Duration::from_millis(500)) {
            Err(Error(errno::EPIPE)) => break,
            Err(Error(errno::EAGAIN | errno::EIO)) => time::sleep(Duration::from_millis(20))?,
            Ok(_) => return Err(Error(errno::EIO)),
            Err(err) => return Err(err),
        }
    }
    dunit::println!("[NET-RENO] complete")
}

