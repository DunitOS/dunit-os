//! E1000 HTTPS (TLS 1.3) + GetRandom CSPRNG + versioned cert store end-to-end test.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use dunit::{
    cert::{self, StoreError},
    errno,
    fs,
    http::{Client, HttpError, TlsFailure},
    net::NetGrant,
    rand,
    time::{self, Duration, Instant},
    Error, Result,
};
use libdunit::{self as sys, wait_set};

const TIMEOUT: Duration = Duration::from_secs(30);

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NET-HTTPS] FAIL {check}")?;
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

fn verify_entropy() -> Result<()> {
    let a = rand::bytes::<32>()?;
    let b = rand::bytes::<32>()?;
    require(a != [0u8; 32] && b != [0u8; 32] && a != b, "GetRandom uniqueness")?;
    let mut empty = [];
    require(sys::get_random(&mut empty, 0) == 0, "GetRandom zero-len")?;
    let mut scratch = [0u8; 8];
    require(sys::get_random(&mut scratch, 2) == errno::EINVAL, "GetRandom bad flags")?;
    dunit::println!("[NET-HTTPS] kernel GetRandom CSPRNG verified")
}

fn verify_cert_store() -> Result<()> {
    let mut store = cert::load_store_blob("/system/etc/ssl/roots.dcrt")?;
    require(store.version() == 1 && store.anchors().len() == 1, "cert store v1")?;
    let mut v2 = fs::read_bounded("/system/etc/ssl/roots-v2.dcrt", 256 * 1024)?;
    let mut bad = v2.clone();
    bad[10] ^= 1;
    require(store.rotate(&bad) == Err(StoreError::ChecksumMismatch), "reject corrupt bundle")?;
    require(store.rotate(&v2) == Ok(2), "rotate to v2")?;
    require(store.version() == 2, "active v2")?;
    require(store.rollback() == Ok(1), "rollback to v1")?;
    require(store.version() == 1, "restored v1")?;
    v2.clear();
    dunit::println!("[NET-HTTPS] versioned cert store rotation and rollback passed")
}

fn wait_first_https(client: &mut Client<'_>, url: &str, started: Instant) -> Result<dunit::http::Response> {
    loop {
        match client.get(url) {
            Ok(resp) => return Ok(resp),
            Err(HttpError::Transport(code))
                if matches!(code, errno::ENOENT | errno::EPIPE | errno::EIO | errno::EAGAIN) =>
            {
                require(started.elapsed()? < TIMEOUT, "https ready timeout")?;
                time::sleep(Duration::from_millis(25))?;
            }
            Err(_) => return Err(Error(errno::EIO)),
        }
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let started = Instant::now()?;
    verify_entropy()?;
    verify_cert_store()?;

    let grant = receive_grant(started)?;
    let mut client = Client::new(&grant);

    // 1. Real OpenSSL TLS 1.3 HTTPS + DNS + 302 redirect + keep-alive reuse + chunked + gzip.
    let r1 = wait_first_https(&mut client, "https://https.du.net.test/redirect", started)?;
    require(r1.status == 200, "HTTPS redirect target 200")?;
    require(r1.url.path == "/tls-gzip", "HTTPS resolved path")?;
    let mut expected = Vec::new();
    for _ in 0..4 {
        expected.extend_from_slice(b"tls13-chunked-gzip-over-e1000!");
    }
    require(r1.body == expected, "HTTPS gzip+chunked body")?;
    require(client.stats().opened == 1, "single TLS connection for redirect")?;
    require(client.stats().reused == 1, "reused TLS connection on redirect")?;

    // 2. Reject HTTPS -> HTTP downgrade redirect.
    require(
        client.get("https://https.du.net.test/downgrade") == Err(HttpError::UnsupportedScheme),
        "reject insecure https->http downgrade redirect",
    )?;
    dunit::println!("[NET-HTTPS] TLS 1.3 OpenSSL HTTPS chunked, gzip and keep-alive passed")?;

    // 3. Reject wrong SAN hostname, expired cert (via boot UTC), and untrusted CA.
    require(
        client.get("https://https.du.net.test:8444/") == Err(HttpError::Tls(TlsFailure::InvalidCertificate)),
        "reject wrong hostname SAN",
    )?;
    require(
        client.get("https://expired.du.net.test:8445/") == Err(HttpError::Tls(TlsFailure::CertificateExpired)),
        "reject expired cert",
    )?;
    require(
        client.get("https://https.du.net.test:8446/") == Err(HttpError::Tls(TlsFailure::UntrustedRoot)),
        "reject untrusted root CA",
    )?;
    dunit::println!("[NET-HTTPS] certificate SAN, expiry and untrusted-CA rejections passed")?;

    // 4. ARP conflict revokes address -> EPIPE.
    loop {
        match client.get("https://192.0.2.2/after-conflict") {
            Err(HttpError::Transport(code)) if code == errno::EPIPE => break,
            Err(HttpError::Transport(code)) if code == errno::EAGAIN || code == errno::EIO => {
                require(started.elapsed()? < TIMEOUT, "conflict timeout")?;
                time::sleep(Duration::from_millis(20))?;
            }
            _ => return Err(Error(errno::EIO)),
        }
    }
    dunit::println!("[NET-HTTPS] complete")
}

