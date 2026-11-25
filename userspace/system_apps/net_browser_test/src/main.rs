//! End-to-end adversarial & restart test for sandboxed `browser_netd` (NO ambient sockets!).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String, vec};
use dunit::{
    browser::{
        BrowserClient, FetchOutcome, FetchRequest, Header, Method, ResourceIntent, CACHE_DIR,
        ERR_DECOMPRESS_BOMB, ERR_POLICY, ERR_REDIRECT_LOOP, ERR_TLS_EXPIRED,
        ERR_TLS_INVALID_CERT, ERR_TLS_UNTRUSTED, ERR_TRUNCATED, FLAG_CACHE_HIT,
        FLAG_CACHE_REPAIRED, FLAG_IS_DOWNLOAD, FLAG_TLS_VERIFIED, FLAG_VIA_PROXY,
    },
    env, errno, fs, process,
    time::{self, Duration, Instant},
    Error, Result,
};
use libdunit as sys;

const TIMEOUT: Duration = Duration::from_secs(30);

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NET-BROWSER] FAIL {check}")?;
    Err(Error(errno::EIO))
}

fn fetch_ok(
    client: &mut BrowserClient,
    url: &str,
    intent: ResourceIntent,
    bypass_cache: bool,
) -> Result<dunit::browser::FetchedResponse> {
    let req = FetchRequest {
        method: Method::Get,
        intent,
        bypass_cache,
        url: String::from(url),
        headers: vec![Header {
            name: String::from("Accept"),
            value: String::from("text/plain"),
        }],
        body: vec![],
    };
    match client.fetch(&req, Duration::from_secs(15))? {
        FetchOutcome::Ok(ok) => Ok(ok),
        FetchOutcome::Remote(err) => {
            dunit::println!("[NET-BROWSER] unexpected remote err {} for {}", err.code, url)?;
            Err(Error(errno::EIO))
        }
    }
}

fn require_err(got: i32, expected: i32, check: &str) -> Result<()> {
    if got == expected {
        return Ok(());
    }
    dunit::println!("[NET-BROWSER] FAIL {check}: got={} expected={}", got, expected)?;
    Err(Error(errno::EIO))
}

fn fetch_err(client: &mut BrowserClient, method: Method, url: &str) -> Result<i32> {
    let req = FetchRequest {
        method,
        intent: ResourceIntent::Document,
        bypass_cache: true,
        url: String::from(url),
        headers: vec![],
        body: vec![],
    };
    match client.fetch(&req, Duration::from_secs(15))? {
        FetchOutcome::Remote(err) => Ok(err.code),
        FetchOutcome::Ok(ok) => {
            dunit::println!("[NET-BROWSER] expected error for {} got HTTP {}", url, ok.payload.status)?;
            Err(Error(errno::EIO))
        }
    }
}

fn wait_network_ready(client: &mut BrowserClient, started: Instant) -> Result<dunit::browser::FetchedResponse> {
    let req = FetchRequest {
        method: Method::Get,
        intent: ResourceIntent::Document,
        bypass_cache: false,
        url: String::from("https://https.du.net.test/viewer"),
        headers: vec![],
        body: vec![],
    };
    loop {
        match client.fetch(&req, Duration::from_secs(8))? {
            FetchOutcome::Ok(ok) => return Ok(ok),
            FetchOutcome::Remote(_) => {
                require(started.elapsed()? < TIMEOUT, "browser_netd initial HTTPS timeout")?;
                time::sleep(Duration::from_millis(40))?;
            }
        }
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let started = Instant::now()?;

    // 1. Prove this client has zero ambient network capabilities.
    require(env::var("DUNIT_NET_ROOT")?.is_none(), "no DUNIT_NET_ROOT")?;
    require(sys::net_socket_open(0, 1, &[]) < 0, "net_socket_open denied")?;
    require(sys::net_connect_grant(0, process::id(), 3) < 0, "net_connect_grant denied")?;
    require(sys::net_queue_open(0, &[]) < 0, "net_queue_open denied")?;
    dunit::println!("[NET-BROWSER] client ambient socket denial passed")?;

    let mut client = BrowserClient::new();
    let initial_ep = client.wait_ready(TIMEOUT)?;
    require(initial_ep.pid != 0 && initial_ep.generation >= 1, "valid initial endpoint")?;

    // 2. Origin/scheme/host/method policy checks.
    require(
        fetch_err(&mut client, Method::Get, "http://https.du.net.test/viewer")? == ERR_POLICY,
        "plaintext HTTP blocked",
    )?;
    require(
        fetch_err(&mut client, Method::Get, "https://169.254.169.254/meta")? == ERR_POLICY,
        "metadata IP blocked",
    )?;
    require(
        fetch_err(&mut client, Method::Get, "https://192.168.1.10/admin")? == ERR_POLICY,
        "private IP blocked",
    )?;
    require(
        fetch_err(&mut client, Method::Delete, "https://https.du.net.test/viewer")? == ERR_POLICY,
        "DELETE method blocked",
    )?;
    dunit::println!("[NET-BROWSER] origin, private-IP and method policy passed")?;

    // 3. Initial OpenSSL HTTPS TLS 1.3 fetch via SharedFrames + cache store + cache hit + cookies + proxy + download.
    let first = wait_network_ready(&mut client, started)?;
    require(first.payload.status == 200, "viewer status 200")?;
    require(first.meta.flags & FLAG_TLS_VERIFIED != 0, "TLS verified flag")?;
    require(first.meta.flags & FLAG_CACHE_HIT == 0, "first fetch miss")?;
    require(
        first.payload.body == b"Dunit OS Isolated Browser Viewer: TLS 1.3 verified response body!",
        "viewer body matches",
    )?;

    let second = fetch_ok(&mut client, "https://https.du.net.test/viewer", ResourceIntent::Document, false)?;
    require(second.meta.flags & FLAG_CACHE_HIT != 0, "second fetch cache hit")?;
    require(second.meta.cache_hits >= 1, "cache_hits incremented")?;

    let echoed = fetch_ok(&mut client, "https://https.du.net.test/echo-cookie", ResourceIntent::Document, true)?;
    require(echoed.payload.body == b"cookie=sid=alpha42", "cookie sent over HTTPS")?;

    let dl = fetch_ok(
        &mut client,
        "https://https.du.net.test/download/raw.bin",
        ResourceIntent::Download,
        true,
    )?;
    require(dl.meta.flags & FLAG_IS_DOWNLOAD != 0, "download flag")?;
    require(dl.payload.download_filename.as_deref() == Some("report.bin"), "sanitized download filename")?;
    require(dl.payload.body == b"safe-download-payload", "download payload")?;

    let proxied = fetch_ok(&mut client, "https://192.0.2.2/via-proxy", ResourceIntent::Document, true)?;
    require(proxied.payload.status == 200, "proxied HTTPS 200")?;
    require(proxied.meta.flags & FLAG_VIA_PROXY != 0, "CONNECT proxy flag")?;
    require(proxied.payload.body == b"https-via-connect-proxy-ok", "proxied payload")?;
    dunit::println!("[NET-BROWSER] HTTPS cache, cookies, download policy and CONNECT proxy passed")?;

    // 4. Adversarial TLS certs, redirect loop, downgrade, truncated framing & decompression bomb.
    require_err(
        fetch_err(&mut client, Method::Get, "https://https.du.net.test:8444/")?,
        ERR_TLS_INVALID_CERT,
        "wrong SAN rejected",
    )?;
    require_err(
        fetch_err(&mut client, Method::Get, "https://expired.du.net.test:8445/")?,
        ERR_TLS_EXPIRED,
        "expired cert rejected",
    )?;
    require_err(
        fetch_err(&mut client, Method::Get, "https://https.du.net.test:8446/")?,
        ERR_TLS_UNTRUSTED,
        "untrusted CA rejected",
    )?;
    require_err(
        fetch_err(&mut client, Method::Get, "https://https.du.net.test/loop")?,
        ERR_REDIRECT_LOOP,
        "redirect loop rejected",
    )?;
    require_err(
        fetch_err(&mut client, Method::Get, "https://https.du.net.test/downgrade")?,
        ERR_POLICY,
        "downgrade rejected",
    )?;
    require_err(
        fetch_err(&mut client, Method::Get, "https://https.du.net.test/truncated")?,
        ERR_TRUNCATED,
        "truncated framing rejected",
    )?;
    require_err(
        fetch_err(&mut client, Method::Get, "https://https.du.net.test/bomb")?,
        ERR_DECOMPRESS_BOMB,
        "decompression bomb rejected",
    )?;
    dunit::println!("[NET-BROWSER] adversarial certs, redirect loop, truncated framing and bomb rejected")?;

    // 5. Corrupt cache detection & self-repair.
    let mut corrupted = 0usize;
    for entry in fs::read_dir(CACHE_DIR)? {
        if entry.name.ends_with(".dnwc") {
            let path = format!("{CACHE_DIR}/{}", entry.name);
            let mut data = fs::read(&path)?;
            if data.len() > 20 {
                data[16] ^= 0xff;
                fs::write(&path, &data)?;
                corrupted += 1;
            }
        }
    }
    require(corrupted >= 1, "at least one cache entry existed to corrupt")?;
    let repaired = fetch_ok(&mut client, "https://https.du.net.test/viewer", ResourceIntent::Document, false)?;
    require(repaired.meta.flags & FLAG_CACHE_REPAIRED != 0, "corrupt cache repaired")?;
    require(repaired.meta.cache_repaired >= 1, "repaired counter incremented")?;
    let cached_again = fetch_ok(&mut client, "https://https.du.net.test/viewer", ResourceIntent::Document, false)?;
    require(cached_again.meta.flags & FLAG_CACHE_HIT != 0, "repaired cache hits again")?;
    dunit::println!("[NET-BROWSER] corrupt cache detection and repair passed")?;

    // 6. Real service crash & init restart + durable cache/cookie survival.
    let old_pid = initial_ep.pid;
    let old_gen = initial_ep.generation;
    client.trigger_fault_for_test(Duration::from_millis(100))?;
    let new_ep = client.wait_restart(old_pid, Duration::from_secs(20))?;
    require(new_ep.pid != old_pid && new_ep.generation > old_gen, "new browser_netd pid and gen")?;

    let post_cache = fetch_ok(&mut client, "https://https.du.net.test/viewer", ResourceIntent::Document, false)?;
    require(post_cache.meta.flags & FLAG_CACHE_HIT != 0, "persisted cache survived restart")?;
    require(post_cache.meta.restarts >= 1, "restart counter reported")?;
    let post_cookie = fetch_ok(&mut client, "https://https.du.net.test/echo-cookie", ResourceIntent::Document, true)?;
    require(post_cookie.payload.body == b"cookie=sid=alpha42", "persisted cookie survived restart")?;
    let done = fetch_ok(&mut client, "https://https.du.net.test/done", ResourceIntent::Document, true)?;
    require(done.payload.body == b"done", "final done ack")?;
    dunit::println!("[NET-BROWSER] service crash recovery and persisted state passed")?;
    dunit::println!("[NET-BROWSER] complete")
}

