//! M8 Acceptance E2E: versioned config rollback, privilege isolation, dnet/ICMP,
//! impairment matrix, 24-cycle leak soak, and netd crash/restart recovery.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::str::FromStr;
use dunit::{
    cert, errno, fs,
    http::{Client, HttpError, Limits},
    io::{Read, Write},
    net::{
        query_routes, query_status, resolve_addrs_timeout, IcmpSocket, IpAddr, NetGrant,
        SocketAddr, TcpListener, TcpStream,
    },
    process::{Child, ExitStatus},
    time::{self, Duration, Instant},
    Error, Result,
};
use dunit_packet_abi::QueueOptions;
use libdunit::{self as sys, wait_set};

const TIMEOUT: Duration = Duration::from_secs(60);

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NET-ACCEPTANCE] FAIL {check}")?;
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

fn run_dnet(args: &[&str]) -> Result<ExitStatus> {
    let mut child = Child::spawn_with_env("/app/dnet", args, &[])?;
    child.wait()
}

fn poll_get(client: &mut Client<'_>, url: &str, started: Instant) -> Result<dunit::http::Response> {
    loop {
        match client.get(url) {
            Ok(resp) => return Ok(resp),
            Err(HttpError::Transport(code))
                if matches!(code, errno::ENOENT | errno::EPIPE | errno::EIO | errno::EAGAIN) =>
            {
                require(started.elapsed()? < TIMEOUT, url)?;
                time::sleep(Duration::from_millis(50))?;
            }
            Err(err) => {
                dunit::println!("[NET-ACCEPTANCE] FAIL unexpected {:?} on {}", err, url)?;
                return Err(Error(errno::EIO));
            }
        }
    }
}

fn poll_status_ready(grant: &NetGrant, started: Instant) -> Result<String> {
    loop {
        match query_status(grant, Duration::from_secs(5)) {
            Ok(s) if s.contains("ipv4=192.0.2.12/24") && s.contains("ipv6=2001:db8:1:") => {
                return Ok(s);
            }
            Ok(_) | Err(Error(errno::EAGAIN | errno::ENOENT | errno::EPIPE | errno::EIO)) => {
                require(started.elapsed()? < TIMEOUT, "status ready deadline")?;
                time::sleep(Duration::from_millis(50))?;
            }
            Err(e) => return Err(e),
        }
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let started = Instant::now()?;
    let grant = receive_grant(started)?;

    // Wait for netd to finish config load/rollback, network configuration, and bridge startup.
    let status = poll_status_ready(&grant, started)?;
    require(status.contains("rx_pkt=") && status.contains("tx_pkt="), "status counters")?;

    // 1. Config rollback & offline CLI validation & privilege denials.
    let last_good = fs::read_to_string("/var/lib/netd/last-good.conf")?;
    require(!last_good.is_empty(), "last-good.conf persisted")?;
    let events = fs::read_to_string("/var/log/netd/events.log")?;
    require(events.contains("rolled back"), "events.log records rollback")?;

    require(
        run_dnet(&["/app/dnet", "validate", "/system/share/network/defaults.toml"])?.success(),
        "dnet validate defaults.toml",
    )?;
    require(
        !run_dnet(&["/app/dnet", "status"])?.success(),
        "unprivileged dnet status denied",
    )?;
    require(
        !run_dnet(&["/app/dnet", "capture"])?.success(),
        "unprivileged dnet capture denied",
    )?;
    require(
        matches!(
            TcpListener::bind(&grant, SocketAddr::from_str("0.0.0.0:80")?),
            Err(Error(errno::EACCES))
        ),
        "SCOPE_LOW_PORT enforced on port 80",
    )?;
    require(
        sys::net_queue_open(0, &QueueOptions.encode()) < 0,
        "ambient raw queue denied",
    )?;
    dunit::println!("[NET-ACCEPTANCE] config rollback and privilege isolation passed")?;

    // 2. Non-ambient routes & bounded ICMPv4 + ICMPv6 ping.
    let routes = query_routes(&grant, Duration::from_secs(5))?;
    require(
        routes.contains("v4 192.0.2.0/24")
            && routes.contains("v6 2001:db8:1:")
            && routes.contains("/64"),
        "route table shows IPv4 and IPv6 prefixes",
    )?;
    let mut ping_sock = IcmpSocket::open(&grant)?;
    let (from4, seq4, _) = ping_sock.ping(
        IpAddr::from_str("192.0.2.1")?,
        1,
        b"m8-v4-ping",
        Duration::from_secs(8),
    )?;
    require(from4 == IpAddr::from_str("192.0.2.1")? && seq4 == 1, "ICMPv4 echo")?;
    let (from6, seq6, _) = ping_sock.ping(
        IpAddr::from_str("2001:db8:1::90")?,
        2,
        b"m8-v6-ping",
        Duration::from_secs(8),
    )?;
    require(
        from6 == IpAddr::from_str("2001:db8:1::90")? && seq6 == 2,
        "ICMPv6 echo",
    )?;
    drop(ping_sock);
    dunit::println!("[NET-ACCEPTANCE] status, routes and ICMPv4/v6 ping passed")?;

    // 3. Impairment matrix (SYN loss + out-of-order TCP segments + dup ACK + corrupt csum + garbage) + TLS 1.3.
    let store = cert::load_store_blob("/system/etc/ssl/roots.dcrt")?;
    let mut client = Client::with_roots(
        &grant,
        store.anchors().to_vec(),
        Limits::default(),
        Duration::from_secs(12),
    );
    let imp = poll_get(&mut client, "http://impair.du.net.test/reorder", started)?;
    require(
        imp.status == 200 && imp.body == b"impairment-reordered-ok",
        "impairment HTTP reorder/loss body",
    )?;
    let tls_imp = poll_get(&mut client, "https://https.du.net.test/m8", started)?;
    require(
        tls_imp.status == 200 && tls_imp.body == b"tls13-under-impairment-ok",
        "TLS 1.3 under impairment",
    )?;
    dunit::println!("[NET-ACCEPTANCE] impairment loss/reorder/dup/corrupt and TLS 1.3 passed")?;

    // 4. 24-cycle connect-close / DNS soak (> SOCKETS_PER_OWNER=8 & SOCKETS=16).
    for i in 0..24usize {
        let host = if i % 2 == 0 {
            "soak0.du.net.test"
        } else {
            "soak1.du.net.test"
        };
        let addrs: Vec<IpAddr> = resolve_addrs_timeout(&grant, host, Duration::from_secs(8))?;
        let target = SocketAddr::new(*addrs.first().ok_or(Error(errno::ENOENT))?, 5432);
        let mut s = TcpStream::connect_timeout(&grant, target, Duration::from_secs(8))?;
        s.set_read_timeout(Some(Duration::from_secs(8)))?;
        s.write_all(b"soak")?;
        let mut buf = [0u8; 16];
        let n = s.read(&mut buf)?;
        require(&buf[..n] == b"soak-ack", "soak TCP echo")?;
    }
    dunit::println!("[NET-ACCEPTANCE] 24-cycle connect-close/DNS leak soak passed")?;

    // 5. Real netd crash/restart with active TCP stream + active listener.
    let mut active = TcpStream::connect_timeout(
        &grant,
        SocketAddr::from_str("192.0.2.2:5433")?,
        Duration::from_secs(8),
    )?;
    active.set_read_timeout(Some(Duration::from_secs(8)))?;
    active.write_all(b"arm-crash")?;
    let mut arm_buf = [0u8; 16];
    let n = active.read(&mut arm_buf)?;
    require(&arm_buf[..n] == b"armed", "active socket established before crash")?;
    let listener = TcpListener::bind(&grant, SocketAddr::from_str("0.0.0.0:8088")?)?;
    active.set_read_timeout(Some(Duration::from_millis(200)))?;

    fs::write("/run/netd/crash-once", b"1")?;

    let mut tmp = [0u8; 8];
    loop {
        match active.read(&mut tmp) {
            Err(Error(errno::EPIPE)) => break,
            Err(Error(errno::EAGAIN)) => {
                require(started.elapsed()? < TIMEOUT, "active TCP EPIPE timeout")?;
                time::sleep(Duration::from_millis(20))?;
            }
            other => {
                dunit::println!("[NET-ACCEPTANCE] FAIL active read={:?}", other)?;
                return Err(Error(errno::EIO));
            }
        }
    }
    loop {
        match listener.accept_timeout(Duration::from_millis(200)) {
            Err(Error(errno::EPIPE)) => break,
            Err(Error(errno::EAGAIN)) => {
                require(started.elapsed()? < TIMEOUT, "listener EPIPE timeout")?;
            }
            Ok((_, addr)) => {
                dunit::println!("[NET-ACCEPTANCE] FAIL listener accept ok from {}", addr)?;
                return Err(Error(errno::EIO));
            }
            Err(err) => {
                dunit::println!("[NET-ACCEPTANCE] FAIL listener accept={:?}", err)?;
                return Err(Error(errno::EIO));
            }
        }
    }
    drop(active);
    drop(listener);

    let mut recovered_client = Client::with_roots(
        &grant,
        store.anchors().to_vec(),
        Limits::default(),
        Duration::from_secs(12),
    );
    let rec = poll_get(
        &mut recovered_client,
        "http://impair.du.net.test/after-restart",
        started,
    )?;
    require(
        rec.status == 200 && rec.body == b"netd-restart-recovered",
        "post-restart HTTP recovery",
    )?;
    dunit::println!("[NET-ACCEPTANCE] netd crash/restart with active sockets passed")?;
    dunit::println!("[NET-ACCEPTANCE] complete")
}
