//! Dual-NIC IPv6 SLAAC/RA/RDNSS/DAD, TLS 1.3 HTTPS, Happy Eyeballs, and link failover E2E test.
#![no_std]
#![no_main]

extern crate alloc;

use core::str::FromStr;
use dunit::{
    cert, errno,
    http::{Client, HttpError, Limits},
    io::{Read, Write},
    net::{IpAddr, NetGrant, SocketAddr, TcpListener, UdpSocket},
    time::{self, Duration, Instant},
    Error, Result,
};
use libdunit::{self as sys, wait_set};

const TIMEOUT: Duration = Duration::from_secs(40);

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NET-IPV6] FAIL {check}")?;
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
                dunit::println!("[NET-IPV6] FAIL unexpected error {:?} on {}", err, url)?;
                return Err(Error(errno::EIO));
            }
        }
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let started = Instant::now()?;
    let grant = receive_grant(started)?;
    let store = cert::load_store_blob("/system/etc/ssl/roots.dcrt")?;
    let mut client = Client::with_roots(
        &grant,
        store.anchors().to_vec(),
        Limits::default(),
        Duration::from_secs(10),
    );

    // 1. HTTPS (TLS 1.3 OpenSSL) via IPv6-only RDNSS AAAA on nic0
    let https_resp = poll_get(&mut client, "https://https.du.net.test/ipv6", started)?;
    require(
        https_resp.status == 200 && https_resp.body == b"ipv6-tls13-ok",
        "ipv6 https tls13 body",
    )?;
    dunit::println!("[NET-IPV6] IPv6 SLAAC/RDNSS + OpenSSL TLS 1.3 HTTPS passed")?;

    // 2. Bracketed IPv6 literal HTTP GET on nic0
    let lit_resp = poll_get(&mut client, "http://[2001:db8:1::443]:8082/v6lit", started)?;
    require(
        lit_resp.status == 200 && lit_resp.body == b"ipv6-http-literal-ok",
        "ipv6 bracketed literal URL",
    )?;
    dunit::println!("[NET-IPV6] bracketed IPv6 URL HTTP passed")?;

    // 3. IPv6 UDP datagram roundtrip + IPv6 TCP listener accept
    let listener = TcpListener::bind(&grant, SocketAddr::from_str("[::]:8081")?)?;
    let mut udp = UdpSocket::bind(&grant, SocketAddr::from_str("[::]:9091")?)?;
    udp.set_read_timeout(Some(Duration::from_secs(10)))?;
    let target_udp = SocketAddr::from_str("[2001:db8:1::90]:7001")?;
    udp.send_to(b"listener6-ready", target_udp)?;
    let mut ubuf = [0u8; 64];
    let (un, ufrom) = udp.recv_from_addr(&mut ubuf)?;
    require(
        &ubuf[..un] == b"udp6-pong" && matches!(ufrom.ip(), IpAddr::V6(_)),
        "udp6 recv_from_addr",
    )?;

    let (mut acc, peer) = listener.accept_addr()?;
    require(matches!(peer.ip(), IpAddr::V6(_)), "accept_addr IPv6 remote")?;
    let mut abuf = [0u8; 64];
    let an = acc.read(&mut abuf)?;
    require(&abuf[..an] == b"hello-listener6", "tcp6 listener read")?;
    acc.write_all(b"listener6-ack")?;
    acc.shutdown_write()?;
    dunit::println!("[NET-IPV6] IPv6 UDP + TCP listener accept passed")?;

    // 4. RFC 8305 Happy Eyeballs: AAAA on nic0 blackholes, A on nic1 connects after 250ms
    let happy_resp = poll_get(&mut client, "http://happy.du.net.test/eyeballs", started)?;
    require(
        happy_resp.status == 200 && happy_resp.body == b"happy-eyeballs-v4-fallback",
        "happy eyeballs fallback",
    )?;
    dunit::println!("[NET-IPV6] RFC 8305 Happy Eyeballs fallback passed")?;

    // Signal host QMP action to take nic0 link down
    dunit::println!("[NET-IPV6] phase1 ready-for-failover")?;
    time::sleep(Duration::from_millis(1_500))?;

    // 5. Multi-NIC failover: nic0 goes link-down; failover6 resolves & connects via nic1 IPv6 + IPv4
    let mut failover_client = Client::with_roots(
        &grant,
        store.anchors().to_vec(),
        Limits::default(),
        Duration::from_secs(4),
    );
    let fo6_resp = poll_get(&mut failover_client, "http://failover6.du.net.test/failover", started)?;
    require(
        fo6_resp.status == 200 && fo6_resp.body == b"nic1-v6-failover-ok",
        "nic1 ipv6 failover",
    )?;
    let fo4_resp = poll_get(&mut failover_client, "http://203.0.113.81/v4", started)?;
    require(
        fo4_resp.status == 200 && fo4_resp.body == b"nic1-v4-failover-ok",
        "nic1 ipv4 failover",
    )?;
    dunit::println!("[NET-IPV6] multi-NIC link failover passed")?;
    dunit::println!("[NET-IPV6] complete")
}

