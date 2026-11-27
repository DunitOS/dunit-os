//! Isolated Browser Network Service (`browser_netd`): policy, TLS 1.3, HTTP, cache, cookies over IPC/SHM.
#![no_std]
#![no_main]

extern crate alloc;

mod config;
mod storage;

use alloc::{format, string::String, vec::Vec};
use dunit::{
    browser::FAULT_FLAG_PATH,
    cert, errno, fs,
    io::{Read, Write},
    net::{NetGrant, TcpStream},
    process, rand,
    time::{self, realtime_secs, Duration, Instant},
    Error, Result,
};
use dunit_browser_net::{
    cache::{BoundedCache, CacheLookup},
    cookies::CookieJar,
    policy::{BrowserPolicy, Origin, PolicyViolation, ProxyRoute},
    wire::{
        ControlMessage, FetchRequest, FetchResponsePayload, ERR_DECOMPRESS_BOMB, ERR_FORBIDDEN,
        ERR_INVALID, ERR_POLICY, ERR_QUOTA, ERR_REDIRECT_LOOP, ERR_TLS_EXPIRED,
        ERR_TLS_HANDSHAKE, ERR_TLS_INVALID_CERT, ERR_TLS_UNTRUSTED, ERR_TRANSPORT,
        ERR_TRUNCATED, FLAG_BYPASS_CACHE, FLAG_CACHE_HIT, FLAG_CACHE_REPAIRED, FLAG_IS_DOWNLOAD,
        FLAG_TLS_VERIFIED, FLAG_VIA_PROXY, OP_FETCH_REQ, OP_FETCH_RESP, OP_PING, OP_STATS_REQ,
        OP_STATS_RESP, OP_TRIGGER_FAULT, SHM_HANDLE_OFFSET,
    },
};
use dunit_http_core::{
    Connector, HttpError, Request, Response, Scheme, Session, TlsFailure, Transport, Url,
};
use dunit_tls_core::{ByteStream, Certificate, TlsClient, TlsError};
use libdunit::{self as sys, wait_set};

fn checked(r: isize) -> Result<usize> {
    if r < 0 {
        Err(Error(r))
    } else {
        Ok(r as usize)
    }
}

const IO_TIMEOUT: Duration = Duration::from_secs(12);
const GRANT_TIMEOUT: Duration = Duration::from_secs(30);

struct TcpByteStream(TcpStream);

impl ByteStream for TcpByteStream {
    fn read_bytes(&mut self, buf: &mut [u8]) -> core::result::Result<usize, TlsError> {
        self.0
            .read(buf)
            .map_err(|err| TlsError::Transport(err.raw_os_error()))
    }

    fn write_all_bytes(&mut self, buf: &[u8]) -> core::result::Result<(), TlsError> {
        self.0
            .write_all(buf)
            .map_err(|err| TlsError::Transport(err.raw_os_error()))
    }
}

enum ProxyTransport {
    Plain(TcpStream),
    Tls(TlsClient<TcpByteStream>),
}

impl Transport for ProxyTransport {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, HttpError> {
        match self {
            Self::Plain(tcp) => tcp
                .read(buf)
                .map_err(|err| HttpError::Transport(err.raw_os_error())),
            Self::Tls(tls) => tls.read(buf).map_err(|err| match err {
                TlsError::UnexpectedEof => HttpError::Truncated,
                other => map_tls_error(other),
            }),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> core::result::Result<(), HttpError> {
        match self {
            Self::Plain(tcp) => tcp
                .write_all(buf)
                .map_err(|err| HttpError::Transport(err.raw_os_error())),
            Self::Tls(tls) => tls.write_all(buf).map_err(map_tls_error),
        }
    }
}

fn map_tls_error(err: TlsError) -> HttpError {
    match err {
        TlsError::InvalidCertificate => HttpError::Tls(TlsFailure::InvalidCertificate),
        TlsError::CertificateExpired => HttpError::Tls(TlsFailure::CertificateExpired),
        TlsError::UntrustedRoot => HttpError::Tls(TlsFailure::UntrustedRoot),
        TlsError::BadSignature => HttpError::Tls(TlsFailure::BadSignature),
        TlsError::Transport(code) => HttpError::Transport(code),
        _ => HttpError::Tls(TlsFailure::Handshake),
    }
}

struct BrowserConnector {
    grant: NetGrant,
    roots: Vec<Certificate>,
    policy: BrowserPolicy,
    last_tls: Option<String>,
    used_proxy: bool,
}

impl Connector for BrowserConnector {
    type Conn = ProxyTransport;

    fn connect(
        &mut self,
        scheme: Scheme,
        host: &str,
        port: u16,
    ) -> core::result::Result<Self::Conn, HttpError> {
        let route = self
            .policy
            .proxy
            .resolve(host)
            .map_err(|_| HttpError::Transport(errno::EACCES))?;
        let mut stream = match route {
            ProxyRoute::Direct => {
                self.used_proxy = false;
                TcpStream::connect_host_timeout(&self.grant, host, port, IO_TIMEOUT)
                    .map_err(|err| HttpError::Transport(err.raw_os_error()))?
            }
            ProxyRoute::HttpForward {
                host: proxy_host,
                port: proxy_port,
            } => {
                self.used_proxy = true;
                let mut tunnel =
                    TcpStream::connect_host_timeout(&self.grant, &proxy_host, proxy_port, IO_TIMEOUT)
                        .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
                let _ = tunnel.set_read_timeout(Some(IO_TIMEOUT));
                let _ = tunnel.set_write_timeout(Some(IO_TIMEOUT));
                open_connect_tunnel(&mut tunnel, host, port)?;
                tunnel
            }
        };
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|err| HttpError::Transport(err.raw_os_error()))?;

        match scheme {
            Scheme::Http => {
                self.last_tls = Some(String::from("plaintext-http"));
                Ok(ProxyTransport::Plain(stream))
            }
            Scheme::Https => {
                let now_unix =
                    realtime_secs().map_err(|_| HttpError::Tls(TlsFailure::ClockUnavailable))?;
                let entropy =
                    rand::bytes::<32>().map_err(|_| HttpError::Tls(TlsFailure::EntropyUnavailable))?;
                let eph =
                    rand::bytes::<32>().map_err(|_| HttpError::Tls(TlsFailure::EntropyUnavailable))?;
                let tls =
                    TlsClient::connect(TcpByteStream(stream), host, &self.roots, now_unix, entropy, eph)
                        .map_err(map_tls_error)?;
                self.last_tls = Some(format!("TLSv1.3 {}", tls.cipher_suite_name()));
                Ok(ProxyTransport::Tls(tls))
            }
        }
    }
}

fn open_connect_tunnel(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
) -> core::result::Result<(), HttpError> {
    let req = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while head.len() < 4096 {
        let n = stream
            .read(&mut byte)
            .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
        if n == 0 {
            return Err(HttpError::Truncated);
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            let line = core::str::from_utf8(&head)
                .ok()
                .and_then(|s| s.lines().next())
                .unwrap_or("");
            if line.contains(" 200") {
                return Ok(());
            }
            return Err(HttpError::InvalidResponse);
        }
    }
    Err(HttpError::TooLarge)
}

fn receive_grant() -> Result<NetGrant> {
    let started = Instant::now()?;
    let supervisor = dunit::env::var("DUNIT_NET_SUPERVISOR")?
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid != 0)
        .ok_or(Error(errno::EACCES))?;
    let mut payload = [0u8; 256];
    loop {
        let mut sender = 0u32;
        let mut attachment = 0u32;
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
            let _ = sys::handle_close(attachment);
        }
        if sender == supervisor && attachment == 0 && got == 4 {
            let handle = u32::from_le_bytes(payload[..4].try_into().unwrap());
            return NetGrant::from_raw_handle(handle);
        }
    }
}

fn now_unix_or_fallback() -> u64 {
    realtime_secs().unwrap_or(0)
}

struct Service {
    pid: u32,
    generation: u32,
    restarts: u16,
    policy: BrowserPolicy,
    cache: BoundedCache<storage::VfsCacheStorage>,
    cookies: CookieJar,
    session: Session<BrowserConnector>,
    fault_enabled: bool,
}

impl Service {
    fn base_ctrl(&self, op: u16, req_id: u64) -> ControlMessage {
        let stats = self.cache.stats();
        let mut msg = ControlMessage::empty(op, req_id);
        msg.service_pid = self.pid;
        msg.generation = self.generation;
        msg.cache_hits = stats.hits.min(u16::MAX as u32) as u16;
        msg.cache_repaired = stats.corrupt_repaired.min(u16::MAX as u32) as u16;
        msg.cookie_count = self.cookies.len().min(u16::MAX as usize) as u16;
        msg.restarts = self.restarts;
        msg
    }

    fn send_error(&self, client: u32, req_id: u64, code: i32, summary: &str, flags: u32) {
        let mut msg = self.base_ctrl(OP_FETCH_RESP, req_id);
        msg.status_or_error = code;
        msg.flags = flags;
        msg.summary = String::from(summary);
        let _ = sys::ipc_send(client, &msg.encode());
    }

    fn send_payload(
        &self,
        client: u32,
        req_id: u64,
        flags: u32,
        summary: &str,
        payload: &FetchResponsePayload,
    ) {
        let bytes = payload.encode();
        let shm_size = bytes.len().max(4096);
        let Ok(shm_raw) = checked(sys::handle_create_shared(shm_size)) else {
            self.send_error(client, req_id, ERR_QUOTA, "shared allocation failed", flags);
            return;
        };
        let shm = shm_raw as u32;
        let Ok(mapped) = checked(sys::handle_map(shm, 0, shm_size)) else {
            let _ = sys::handle_close(shm);
            self.send_error(client, req_id, ERR_QUOTA, "shared map failed", flags);
            return;
        };
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped as *mut u8, bytes.len());
        }
        let _ = sys::vm_unmap(mapped, (shm_size + 4095) & !4095);

        let mut msg = self.base_ctrl(OP_FETCH_RESP, req_id);
        msg.status_or_error = payload.status as i32;
        msg.payload_len = bytes.len() as u32;
        msg.flags = flags;
        msg.summary = String::from(summary);
        let _ = sys::ipc_send_handle(
            client,
            shm,
            sys::RIGHT_READ | sys::RIGHT_MAP,
            &msg.encode(),
            SHM_HANDLE_OFFSET,
        );
        let _ = sys::handle_close(shm);
    }

    fn handle_fetch(&mut self, client: u32, ctrl: ControlMessage, attachment: u32) {
        let req_bytes = match read_shm_bytes(attachment, ctrl.payload_len as usize, 64 * 1024) {
            Ok(bytes) => bytes,
            Err(_) => {
                self.send_error(client, ctrl.request_id, ERR_INVALID, "invalid request buffer", 0);
                return;
            }
        };
        let Ok(req) = FetchRequest::decode(&req_bytes) else {
            self.send_error(client, ctrl.request_id, ERR_INVALID, "malformed fetch request", 0);
            return;
        };
        let Ok(parsed_url) = Url::parse(&req.url) else {
            self.send_error(client, ctrl.request_id, ERR_INVALID, "invalid URL", 0);
            return;
        };
        if let Err(violation) = self.policy.check_request(req.method, &parsed_url, req.intent) {
            let detail = format!("{violation}");
            self.send_error(client, ctrl.request_id, ERR_POLICY, &detail, 0);
            return;
        }

        let mut extra_flags = 0u32;
        let now = now_unix_or_fallback();
        let bypass = req.bypass_cache || (ctrl.flags & FLAG_BYPASS_CACHE != 0);
        if !bypass {
            match self.cache.lookup(req.method, &parsed_url, now) {
                CacheLookup::Hit(entry) => {
                    let disp = match self.policy.downloads.classify(
                        &parsed_url,
                        &entry.headers,
                        entry.body.len(),
                        req.intent,
                    ) {
                        Ok(d) => d,
                        Err(v) => {
                            let detail = format!("{v}");
                            self.send_error(client, ctrl.request_id, ERR_POLICY, &detail, 0);
                            return;
                        }
                    };
                    let mut flags = FLAG_CACHE_HIT
                        | if parsed_url.scheme == Scheme::Https {
                            FLAG_TLS_VERIFIED
                        } else {
                            0
                        };
                    if disp.is_download {
                        flags |= FLAG_IS_DOWNLOAD;
                    }
                    let payload = FetchResponsePayload {
                        status: entry.status,
                        final_url: req.url,
                        content_type: disp.content_type,
                        download_filename: disp.filename,
                        tls_info: String::from("cached-verified"),
                        detail: String::from("cache-hit"),
                        headers: entry.headers,
                        body: entry.body,
                    };
                    self.send_payload(client, ctrl.request_id, flags, "cache-hit", &payload);
                    return;
                }
                CacheLookup::MissCorrupt => {
                    extra_flags |= FLAG_CACHE_REPAIRED;
                    let _ = dunit::println!(
                        "[BROWSER-NETD] corrupt cache entry repaired repaired={}",
                        self.cache.stats().corrupt_repaired
                    );
                }
                CacheLookup::Miss => {}
            }
        }

        let mut http_req = match Request::new(req.method, &req.url) {
            Ok(r) => r,
            Err(_) => {
                self.send_error(client, ctrl.request_id, ERR_INVALID, "invalid URL", extra_flags);
                return;
            }
        };
        let mut has_cookie = false;
        for h in req.headers {
            if h.name.eq_ignore_ascii_case("cookie") {
                has_cookie = true;
            }
            if let Ok(next_req) = http_req.clone().header(&h.name, &h.value) {
                http_req = next_req;
            }
        }
        if !has_cookie {
            if let Some(cookie_hdr) = self.cookies.cookie_header_for(&parsed_url, now) {
                if let Ok(next_req) = http_req.clone().header("Cookie", &cookie_hdr) {
                    http_req = next_req;
                }
            }
        }
        http_req.body = req.body;

        let mut redirect_violation: Option<PolicyViolation> = None;
        let policy = &self.policy;
        let intent = req.intent;
        let resp = match self.session.send_with_redirect_check(&http_req, |from, to| {
            if let Err(v) = check_redirect_hop(policy, from, to, intent) {
                redirect_violation = Some(v);
                return Err(HttpError::Transport(errno::EACCES));
            }
            Ok(())
        }) {
            Ok(r) => r,
            Err(err) => {
                if let Some(v) = redirect_violation {
                    let detail = format!("{v}");
                    self.send_error(client, ctrl.request_id, ERR_POLICY, &detail, extra_flags);
                } else {
                    let (code, label) = map_http_error(&self.policy, &err);
                    self.send_error(client, ctrl.request_id, code, label, extra_flags);
                }
                return;
            }
        };

        let tls_info = self
            .session
            .connector()
            .last_tls
            .clone()
            .unwrap_or_else(|| String::from("TLSv1.3 reused"));
        let used_proxy = self.session.connector().used_proxy;
        if let Err(v) = check_redirect_origin(&self.policy, &parsed_url, &resp) {
            let detail = format!("{v}");
            self.send_error(client, ctrl.request_id, ERR_POLICY, &detail, extra_flags);
            return;
        }
        if resp.body.len() > self.policy.quotas.max_shm_bytes {
            self.send_error(client, ctrl.request_id, ERR_QUOTA, "body exceeds shared buffer limit", extra_flags);
            return;
        }
        let disp = match self.policy.downloads.classify(
            &resp.url,
            &resp.headers,
            resp.body.len(),
            req.intent,
        ) {
            Ok(d) => d,
            Err(v) => {
                let detail = format!("{v}");
                self.send_error(client, ctrl.request_id, ERR_POLICY, &detail, extra_flags);
                return;
            }
        };

        let cookie_now = now_unix_or_fallback();
        if self
            .cookies
            .apply_response_headers(&resp.url, &resp.headers, cookie_now)
        {
            storage::write_cookies_blob(&self.cookies.encode());
        }
        let _ = self.cache.store(req.method, &parsed_url, &resp, cookie_now);

        let mut flags = extra_flags;
        if resp.url.scheme == Scheme::Https {
            flags |= FLAG_TLS_VERIFIED;
        }
        if disp.is_download {
            flags |= FLAG_IS_DOWNLOAD;
        }
        if used_proxy {
            flags |= FLAG_VIA_PROXY;
        }
        let final_url = format!(
            "{}://{}:{}{}",
            resp.url.scheme.as_str(),
            resp.url.host,
            resp.url.port,
            resp.url.path
        );
        let detail = if used_proxy { "live-proxied" } else { "live-direct" };
        let payload = FetchResponsePayload {
            status: resp.status,
            final_url,
            content_type: disp.content_type,
            download_filename: disp.filename,
            tls_info: tls_info.clone(),
            detail: String::from(detail),
            headers: resp.headers,
            body: resp.body,
        };
        self.send_payload(client, ctrl.request_id, flags, &tls_info, &payload);
    }
}

fn check_redirect_hop(
    policy: &BrowserPolicy,
    from: &Url,
    to: &Url,
    intent: dunit_browser_net::policy::ResourceIntent,
) -> core::result::Result<(), PolicyViolation> {
    policy.origins.check_redirect(from, to)?;
    policy.origins.check_url(to, intent)?;
    let _ = policy.proxy.resolve(&to.host)?;
    Ok(())
}

fn check_redirect_origin(
    policy: &BrowserPolicy,
    initial: &Url,
    resp: &Response,
) -> core::result::Result<(), PolicyViolation> {
    if !policy.origins.allow_cross_origin_redirect
        && !Origin::from_url(initial).same_origin(&Origin::from_url(&resp.url))
    {
        return Err(PolicyViolation::CrossOriginDenied);
    }
    policy.origins.check_redirect(initial, &resp.url)
}

fn map_http_error(policy: &BrowserPolicy, err: &HttpError) -> (i32, &'static str) {
    let label = BrowserPolicy::map_http_error(err);
    let code = match err {
        HttpError::TooManyRedirects => ERR_REDIRECT_LOOP,
        HttpError::Truncated => ERR_TRUNCATED,
        HttpError::Decompress => ERR_DECOMPRESS_BOMB,
        HttpError::TooLarge => {
            if policy.quotas.max_decompressed_bytes > 0 {
                ERR_DECOMPRESS_BOMB
            } else {
                ERR_QUOTA
            }
        }
        HttpError::UnsupportedScheme => ERR_POLICY,
        HttpError::Tls(tls) => match tls {
            TlsFailure::CertificateExpired => ERR_TLS_EXPIRED,
            TlsFailure::InvalidCertificate => ERR_TLS_INVALID_CERT,
            TlsFailure::UntrustedRoot => ERR_TLS_UNTRUSTED,
            _ => ERR_TLS_HANDSHAKE,
        },
        HttpError::InvalidUrl | HttpError::InvalidHeader | HttpError::InvalidResponse | HttpError::UnsupportedEncoding => {
            ERR_INVALID
        }
        HttpError::Transport(_) => ERR_TRANSPORT,
    };
    (code, label)
}

fn read_shm_bytes(attachment: u32, len: usize, limit: usize) -> Result<Vec<u8>> {
    if attachment == 0 || len == 0 || len > limit {
        if attachment != 0 {
            let _ = sys::handle_close(attachment);
        }
        return Err(Error(errno::EINVAL));
    }
    let real_len = match checked(sys::handle_shared_len(attachment)) {
        Ok(n) => n,
        Err(err) => {
            let _ = sys::handle_close(attachment);
            return Err(err);
        }
    };
    if len > real_len {
        let _ = sys::handle_close(attachment);
        return Err(Error(errno::EINVAL));
    }
    let ptr = match checked(sys::handle_map(attachment, 0, len)) {
        Ok(p) => p,
        Err(err) => {
            let _ = sys::handle_close(attachment);
            return Err(err);
        }
    };
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        out.push(unsafe { core::ptr::read_volatile((ptr as *const u8).add(i)) });
    }
    let _ = sys::shared_buffer::release(attachment, ptr, len);
    Ok(out)
}

#[dunit::main]
fn main() -> Result<()> {
    let grant = receive_grant()?;
    storage::ensure_dirs();
    let generation = storage::next_generation();
    let restarts = generation.saturating_sub(1).min(u16::MAX as u32) as u16;
    let pid = process::id();
    let policy = config::load_policy();
    let roots = cert::load_default_roots();

    let mut cache = BoundedCache::open(
        storage::VfsCacheStorage::new(dunit::browser::CACHE_DIR),
        policy.quotas.max_cache_entries,
        policy.quotas.max_cache_entry_bytes,
        policy.quotas.max_cache_total_bytes,
    );
    let startup_repaired = cache.purge_corrupt();
    let mut cookies = CookieJar::new(
        policy.quotas.max_cookies_per_origin,
        policy.quotas.max_total_cookies,
    );
    if let Some(blob) = storage::read_cookies_blob() {
        let _ = cookies.load_blob(&blob, now_unix_or_fallback());
    }
    let fault_enabled = fs::File::open(FAULT_FLAG_PATH).is_ok();
    storage::publish_endpoint(pid, generation)?;

    dunit::println!(
        "[BROWSER-NETD] ready pid={} gen={} roots={} cookies={} cached={} repaired={}",
        pid,
        generation,
        roots.len(),
        cookies.len(),
        cache.stats().active_entries,
        startup_repaired
    )?;

    let limits = policy.quotas.to_http_limits();
    let connector = BrowserConnector {
        grant,
        roots,
        policy: policy.clone(),
        last_tls: None,
        used_proxy: false,
    };
    let mut service = Service {
        pid,
        generation,
        restarts,
        policy,
        cache,
        cookies,
        session: Session::new(connector, limits),
        fault_enabled,
    };

    let mut buf = [0u8; 256];
    loop {
        let mut sender = 0u32;
        let mut attachment = 0u32;
        let got = sys::ipc_recv_handle(&mut buf, &mut sender, &mut attachment);
        if got == errno::EAGAIN {
            wait_set::wait(
                &[wait_set::Interest::ipc(0)],
                &mut [wait_set::Event::default()],
                Duration::from_millis(250).as_nanos() as u64,
            )
            .map_err(Error)?;
            continue;
        }
        if got < 0 {
            let _ = time::sleep(Duration::from_millis(5));
            continue;
        }
        let Ok(ctrl) = ControlMessage::decode(&buf[..got as usize]) else {
            if attachment != 0 {
                let _ = sys::handle_close(attachment);
            }
            continue;
        };
        match ctrl.op {
            OP_PING => {
                if attachment != 0 {
                    let _ = sys::handle_close(attachment);
                }
                let mut reply = service.base_ctrl(OP_PING, ctrl.request_id);
                reply.summary = String::from("pong");
                let _ = sys::ipc_send(sender, &reply.encode());
            }
            OP_STATS_REQ => {
                if attachment != 0 {
                    let _ = sys::handle_close(attachment);
                }
                let mut reply = service.base_ctrl(OP_STATS_RESP, ctrl.request_id);
                reply.summary = String::from("stats");
                let _ = sys::ipc_send(sender, &reply.encode());
            }
            OP_FETCH_REQ => service.handle_fetch(sender, ctrl, attachment),
            OP_TRIGGER_FAULT => {
                if attachment != 0 {
                    let _ = sys::handle_close(attachment);
                }
                if service.fault_enabled {
                    dunit::println!("[BROWSER-NETD] simulated crash exit=42 pid={}", service.pid)?;
                    process::exit(42);
                } else {
                    service.send_error(sender, ctrl.request_id, ERR_FORBIDDEN, "fault disabled", 0);
                }
            }
            _ => {
                if attachment != 0 {
                    let _ = sys::handle_close(attachment);
                }
                service.send_error(sender, ctrl.request_id, ERR_INVALID, "unknown op", 0);
            }
        }
    }
}

