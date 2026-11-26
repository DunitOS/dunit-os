//! Native network diagnostics and config validation CLI (`/app/dnet`).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use dunit::{
    args::args,
    cert, env, errno, fs,
    http::{Client, Limits},
    net::{query_routes, query_status, resolve_addrs_timeout, IcmpSocket, IpAddr, NetGrant, SocketAddr, TcpStream},
    time::{Duration, Instant},
    Error, Result,
};
use dunit_packet_abi::{QueueOptions, Records, MAX_BATCH_LEN};
use libdunit::{self as sys, wait_set};

const GRANT_TIMEOUT: Duration = Duration::from_secs(15);
const CMD_TIMEOUT: Duration = Duration::from_secs(10);

fn acquire_grant() -> Result<NetGrant> {
    if let Ok(Some(raw)) = env::var("DUNIT_NET_GRANT") {
        if let Ok(h) = raw.parse::<u32>() {
            if h != 0 {
                return NetGrant::from_raw_handle(h);
            }
        }
    }
    let supervisor = env::var("DUNIT_NET_SUPERVISOR")?
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|pid| *pid != 0)
        .ok_or(Error(errno::EACCES))?;
    let started = Instant::now()?;
    let mut payload = [0u8; 256];
    loop {
        let mut sender = 0;
        let mut attachment = 0;
        let got = sys::ipc_recv_handle(&mut payload, &mut sender, &mut attachment);
        if got == errno::EAGAIN {
            let rem = GRANT_TIMEOUT.checked_sub(started.elapsed()?).unwrap_or_default();
            if rem.is_zero() {
                return Err(Error(errno::EAGAIN));
            }
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
            return NetGrant::from_raw_handle(handle);
        }
    }
}

fn validate_config_text(text: &str) -> core::result::Result<(), &'static str> {
    if text.len() > 2048 {
        return Err("config exceeds 2048 bytes");
    }
    let mut version: Option<u32> = None;
    let mut in_section = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(sec) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let idx = sec.strip_prefix("nic").and_then(|s| s.parse::<u32>().ok()).ok_or("invalid section")?;
            if idx >= 4 {
                return Err("nic index out of range");
            }
            in_section = true;
            continue;
        }
        let (k, v) = line.split_once('=').ok_or("expected key=value")?;
        let k = k.trim();
        let v = v.trim().trim_matches('"');
        if !in_section {
            if k == "version" {
                let ver = v.parse::<u32>().map_err(|_| "invalid version")?;
                if ver != 1 {
                    return Err("unsupported schema version");
                }
                version = Some(ver);
                continue;
            }
            return Err("unexpected top-level key");
        }
        match k {
            "metric" => {
                v.parse::<u32>().map_err(|_| "invalid metric")?;
            }
            "dad_probes" => {
                v.parse::<u8>().map_err(|_| "invalid dad_probes")?;
            }
            "ipv4" => {
                if v != "dhcp" && v != "none" {
                    let (a, m) = v.split_once('/').ok_or("invalid ipv4 CIDR")?;
                    a.trim().parse::<dunit::net::Ipv4Addr>().map_err(|_| "invalid ipv4 address")?;
                    let pfx = m.trim().parse::<u8>().map_err(|_| "invalid ipv4 prefix")?;
                    if pfx == 0 || pfx > 32 {
                        return Err("ipv4 prefix out of range");
                    }
                }
            }
            "gateway4" => {
                v.parse::<dunit::net::Ipv4Addr>().map_err(|_| "invalid gateway4")?;
            }
            "dns4" => {
                for p in v.split(',').map(str::trim) {
                    p.parse::<dunit::net::Ipv4Addr>().map_err(|_| "invalid dns4")?;
                }
            }
            "ipv6" => {
                if v != "slaac" && v != "none" {
                    let (a, m) = v.split_once('/').ok_or("invalid ipv6 CIDR")?;
                    a.trim().parse::<dunit::net::Ipv6Addr>().map_err(|_| "invalid ipv6 address")?;
                    let pfx = m.trim().parse::<u8>().map_err(|_| "invalid ipv6 prefix")?;
                    if pfx == 0 || pfx > 128 {
                        return Err("ipv6 prefix out of range");
                    }
                }
            }
            "gateway6" => {
                v.parse::<dunit::net::Ipv6Addr>().map_err(|_| "invalid gateway6")?;
            }
            "dns6" => {
                for p in v.split(',').map(str::trim) {
                    p.parse::<dunit::net::Ipv6Addr>().map_err(|_| "invalid dns6")?;
                }
            }
            _ => return Err("unknown interface option"),
        }
    }
    if version != Some(1) {
        return Err("missing version = 1");
    }
    Ok(())
}

fn cmd_validate(path: &str) -> Result<()> {
    let text = fs::read_to_string(path)?;
    match validate_config_text(&text) {
        Ok(()) => dunit::println!("[DNET] config valid: {}", path),
        Err(msg) => {
            dunit::println!("[DNET] invalid config {}: {}", path, msg)?;
            Err(Error(errno::EINVAL))
        }
    }
}

fn cmd_status() -> Result<()> {
    let grant = acquire_grant()?;
    let out = query_status(&grant, CMD_TIMEOUT)?;
    dunit::print!("{}", out)
}

fn cmd_route() -> Result<()> {
    let grant = acquire_grant()?;
    let out = query_routes(&grant, CMD_TIMEOUT)?;
    dunit::print!("{}", out)
}

fn cmd_dns(host: &str) -> Result<()> {
    let grant = acquire_grant()?;
    let addrs = resolve_addrs_timeout(&grant, host, CMD_TIMEOUT)?;
    for ip in addrs {
        dunit::println!("{} -> {}", host, ip)?;
    }
    Ok(())
}

fn cmd_ping(host: &str, count: u16) -> Result<()> {
    let grant = acquire_grant()?;
    let target = if let Ok(ip) = host.parse::<IpAddr>() {
        ip
    } else {
        *resolve_addrs_timeout(&grant, host, CMD_TIMEOUT)?
            .first()
            .ok_or(Error(errno::ENOENT))?
    };
    let mut sock = IcmpSocket::open(&grant)?;
    let count = count.clamp(1, 16);
    for seq in 1..=count {
        let (_from, rseq, rtt) = sock.ping(target, seq, b"dunit-ping-v1", CMD_TIMEOUT)?;
        dunit::println!(
            "[DNET] ping {} seq={} time={}ms",
            target,
            rseq,
            rtt.as_millis()
        )?;
    }
    Ok(())
}

fn cmd_connect(target: &str, port_arg: Option<&str>) -> Result<()> {
    let grant = acquire_grant()?;
    if target.starts_with("http://") || target.starts_with("https://") {
        let roots = cert::load_store_blob("/system/etc/ssl/roots.dcrt")
            .map(|s| s.anchors().to_vec())
            .unwrap_or_default();
        let mut client = Client::with_roots(&grant, roots, Limits::default(), CMD_TIMEOUT);
        let resp = client.get(target).map_err(|_| Error(errno::EIO))?;
        return dunit::println!("[DNET] {} -> status={} bytes={}", target, resp.status, resp.body.len());
    }
    let port = port_arg
        .and_then(|p| p.parse::<u16>().ok())
        .ok_or(Error(errno::EINVAL))?;
    let stream = if let Ok(ip) = target.parse::<IpAddr>() {
        TcpStream::connect_timeout(&grant, SocketAddr::new(ip, port), CMD_TIMEOUT)?
    } else {
        TcpStream::connect_host_timeout(&grant, target, port, CMD_TIMEOUT)?
    };
    let peer = stream.peer_addr()?;
    dunit::println!("[DNET] connected {} -> {}", target, peer)
}

fn checked(rc: isize) -> Result<usize> {
    if rc < 0 {
        Err(Error(rc))
    } else {
        Ok(rc as usize)
    }
}

fn cmd_capture(limit: usize) -> Result<()> {
    let handle = env::var("DUNIT_NET_DEVICE")?
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|h| *h != 0)
        .ok_or(Error(errno::EACCES))?;
    let rights = sys::handle_rights(handle);
    if rights < 0 || ((rights as usize) & (sys::RIGHT_NET_RAW as usize)) == 0 {
        return Err(Error(errno::EACCES));
    }
    let qfd = checked(sys::net_queue_open(handle, &QueueOptions.encode()))?;
    let mut buf = alloc::vec![0u8; MAX_BATCH_LEN];
    let limit = limit.clamp(1, 16);
    let mut captured = 0usize;
    let started = Instant::now()?;
    while captured < limit && started.elapsed()? < Duration::from_secs(5) {
        let n = sys::read(qfd, &mut buf);
        if n > 0 {
            if let Ok(records) = Records::new(&buf[..n as usize]) {
                for frame in records {
                    captured += 1;
                    let etype = if frame.len() >= 14 {
                        u16::from_be_bytes([frame[12], frame[13]])
                    } else {
                        0
                    };
                    dunit::println!("[DNET] cap #{} len={} eth=0x{:04x}", captured, frame.len(), etype)?;
                    if captured >= limit {
                        break;
                    }
                }
            }
        } else if n == errno::EAGAIN {
            let _ = wait_set::wait(
                &[wait_set::Interest::descriptor(qfd as u32, wait_set::READ, 0)],
                &mut [wait_set::Event::default()],
                200_000_000,
            );
        } else {
            sys::close(qfd);
            return Err(Error(n));
        }
    }
    sys::close(qfd);
    Ok(())
}

#[dunit::main]
fn main() -> Result<()> {
    let argv: Vec<String> = args()?;
    let cmd = argv.get(1).map(String::as_str).unwrap_or("help");
    match cmd {
        "status" => cmd_status(),
        "route" => cmd_route(),
        "dns" => {
            let host = argv.get(2).map(String::as_str).ok_or(Error(errno::EINVAL))?;
            cmd_dns(host)
        }
        "ping" => {
            let (count, target) = if argv.get(2).map(String::as_str) == Some("-c") {
                let c = argv
                    .get(3)
                    .and_then(|s| s.parse::<u16>().ok())
                    .ok_or(Error(errno::EINVAL))?;
                let t = argv.get(4).map(String::as_str).ok_or(Error(errno::EINVAL))?;
                (c, t)
            } else {
                let t = argv.get(2).map(String::as_str).ok_or(Error(errno::EINVAL))?;
                (1, t)
            };
            cmd_ping(target, count)
        }
        "connect" => {
            let target = argv.get(2).map(String::as_str).ok_or(Error(errno::EINVAL))?;
            cmd_connect(target, argv.get(3).map(String::as_str))
        }
        "capture" => {
            let limit = if argv.get(2).map(String::as_str) == Some("-c") {
                argv.get(3)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(4)
            } else {
                4
            };
            cmd_capture(limit)
        }
        "validate" => {
            let path = argv.get(2).map(String::as_str).ok_or(Error(errno::EINVAL))?;
            cmd_validate(path)
        }
        _ => dunit::println!("usage: dnet <status|route|ping [-c N] host|dns host|connect host [port]|capture [-c N]|validate path>"),
    }
}
