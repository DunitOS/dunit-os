//! E1000 HTTP/1.1 test: DNS resolve, redirects, keep-alive, chunked, gzip and bounded errors.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use dunit::{
    errno,
    http::{Client, HttpError, Limits, Method, Request},
    net::NetGrant,
    time::{self, Duration, Instant},
    Error, Result,
};
use libdunit::{self as sys, wait_set};

const TIMEOUT: Duration = Duration::from_secs(25);

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NET-HTTP] FAIL {check}")?;
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

fn wait_first_get(client: &mut Client<'_>, url: &str, started: Instant) -> Result<dunit::http::Response> {
    loop {
        match client.get(url) {
            Ok(resp) => return Ok(resp),
            Err(HttpError::Transport(code))
                if code == errno::ENOENT
                    || code == errno::EPIPE
                    || code == errno::EIO
                    || code == errno::EAGAIN =>
            {
                require(started.elapsed()? < TIMEOUT, "http ready timeout")?;
                time::sleep(Duration::from_millis(25))?;
            }
            Err(_) => return Err(Error(errno::EIO)),
        }
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let started = Instant::now()?;
    let grant = receive_grant(started)?;
    let mut client = Client::new(&grant);

    // 1. DNS + 302 redirect + keep-alive reuse + Transfer-Encoding: chunked.
    let r1 = wait_first_get(&mut client, "http://http.du.net.test/redirect", started)?;
    require(r1.status == 200, "redirect target status 200")?;
    require(r1.url.path == "/chunked", "redirect resolved path")?;
    require(r1.body == b"chunked-body-from-e1000-peer", "chunked body content")?;
    require(client.stats().opened == 1, "single connection for redirect")?;
    require(client.stats().reused == 1, "keep-alive reused on redirect")?;

    // 2. Second request reuses idle keep-alive socket for gzip payload.
    let r2 = client.get("http://http.du.net.test/gzip").map_err(|_| Error(errno::EIO))?;
    require(r2.status == 200, "gzip status 200")?;
    let mut expected_gz = Vec::new();
    for _ in 0..8 {
        expected_gz.extend_from_slice(b"dunit-os HTTP/1.1 keep-alive compressed! ");
    }
    require(r2.body == expected_gz, "gzip body content")?;
    require(client.stats().opened == 1, "still single TCP connection for same host")?;
    require(client.stats().reused == 2, "reused twice for gzip")?;

    // 3. POST with Content-Length and Connection: close.
    let post = Request::new(Method::Post, "http://192.0.2.2/echo")
        .map_err(|_| Error(errno::EINVAL))?
        .body(b"post-ping".to_vec());
    let r3 = client.request(&post).map_err(|_| Error(errno::EIO))?;
    require(r3.status == 200, "post status 200")?;
    require(r3.body == b"echo:post-ping", "post echo body")?;
    dunit::println!("[NET-HTTP] keep-alive, chunked, gzip and redirects passed")?;

    // 4. Tight bounds reject redirect loop, decompression bomb, and truncated chunk.
    let mut tight = Limits::default();
    tight.max_redirects = 2;
    tight.max_decompressed_bytes = 64;
    let mut bounded = Client::with_config(&grant, tight, Duration::from_secs(10));

    require(
        bounded.get("http://192.0.2.2/loop") == Err(HttpError::TooManyRedirects),
        "redirect loop rejected",
    )?;
    require(
        bounded.get("http://192.0.2.2/bomb") == Err(HttpError::TooLarge),
        "decompression bomb rejected",
    )?;
    require(
        bounded.get("http://192.0.2.2/truncated") == Err(HttpError::Truncated),
        "truncated chunk rejected",
    )?;
    dunit::println!("[NET-HTTP] bounded rejection checks passed")?;

    // 5. ARP address conflict triggers revocation (EPIPE).
    loop {
        match bounded.get("http://192.0.2.2/after-conflict") {
            Err(HttpError::Transport(code)) if code == errno::EPIPE => break,
            Err(HttpError::Transport(code)) if code == errno::EAGAIN || code == errno::EIO => {
                require(started.elapsed()? < TIMEOUT, "conflict timeout")?;
                time::sleep(Duration::from_millis(20))?;
            }
            _ => return Err(Error(errno::EIO)),
        }
    }
    dunit::println!("[NET-HTTP] complete")
}

