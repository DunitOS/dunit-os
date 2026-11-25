//! Bounded HTTP/1.1 protocol client core (`no_std + alloc`).
#![no_std]
extern crate alloc;

use alloc::{
    collections::VecDeque,
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

mod inflate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Delete,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }

    pub const fn is_idempotent(self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Put | Self::Delete)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    InvalidUrl,
    UnsupportedScheme,
    InvalidHeader,
    InvalidResponse,
    Truncated,
    TooLarge,
    TooManyRedirects,
    Decompress,
    UnsupportedEncoding,
    Transport(isize),
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl => f.write_str("invalid URL"),
            Self::UnsupportedScheme => f.write_str("unsupported URL scheme"),
            Self::InvalidHeader => f.write_str("invalid HTTP header"),
            Self::InvalidResponse => f.write_str("malformed HTTP response"),
            Self::Truncated => f.write_str("truncated HTTP response"),
            Self::TooLarge => f.write_str("HTTP message exceeds limit"),
            Self::TooManyRedirects => f.write_str("too many HTTP redirects"),
            Self::Decompress => f.write_str("HTTP body decompression failed"),
            Self::UnsupportedEncoding => f.write_str("unsupported Content-Encoding"),
            Self::Transport(code) => write!(f, "transport error ({code})"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_header_bytes: usize,
    pub max_headers: usize,
    pub max_body_bytes: usize,
    pub max_decompressed_bytes: usize,
    pub max_chunk_line: usize,
    pub max_redirects: usize,
    pub max_idle_connections: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_header_bytes: 16 * 1024,
            max_headers: 64,
            max_body_bytes: 4 * 1024 * 1024,
            max_decompressed_bytes: 4 * 1024 * 1024,
            max_chunk_line: 128,
            max_redirects: 5,
            max_idle_connections: 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Url {
    pub fn parse(raw: &str) -> Result<Self, HttpError> {
        if raw.is_empty() || has_ctl(raw) {
            return Err(HttpError::InvalidUrl);
        }
        let rest = if let Some(after) = strip_scheme(raw, "http://") {
            after
        } else if raw.contains("://") {
            return Err(HttpError::UnsupportedScheme);
        } else {
            return Err(HttpError::InvalidUrl);
        };
        let no_frag = rest.split('#').next().unwrap_or(rest);
        let (authority, path_raw) = match no_frag.find(['/', '?']) {
            Some(idx) => (&no_frag[..idx], &no_frag[idx..]),
            None => (no_frag, ""),
        };
        if authority.is_empty() || authority.contains('@') {
            return Err(HttpError::InvalidUrl);
        }
        let (host, port) = if let Some((h, p)) = authority.split_once(':') {
            if h.is_empty() || p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(HttpError::InvalidUrl);
            }
            let port: u16 = p.parse().map_err(|_| HttpError::InvalidUrl)?;
            if port == 0 {
                return Err(HttpError::InvalidUrl);
            }
            (h, port)
        } else {
            (authority, 80)
        };
        if !valid_host(host) {
            return Err(HttpError::InvalidUrl);
        }
        let path = if path_raw.is_empty() {
            "/".to_string()
        } else if path_raw.starts_with('?') {
            format!("/{path_raw}")
        } else {
            path_raw.to_string()
        };
        if path.bytes().any(|b| b <= 0x20 || b == 0x7f) {
            return Err(HttpError::InvalidUrl);
        }
        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
            path,
        })
    }

    pub fn host_header(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn resolve_location(&self, location: &str) -> Result<Self, HttpError> {
        let loc = location.trim();
        if loc.is_empty() || has_ctl(loc) {
            return Err(HttpError::InvalidResponse);
        }
        if loc.contains("://") {
            return Self::parse(loc);
        }
        if loc.starts_with("//") {
            return Self::parse(&format!("http:{loc}"));
        }
        let no_frag = loc.split('#').next().unwrap_or(loc);
        let path = if no_frag.starts_with('/') {
            normalize_path(no_frag)?
        } else if no_frag.starts_with('?') {
            let base = self.path.split('?').next().unwrap_or("/");
            format!("{base}{no_frag}")
        } else {
            let base_path = self.path.split('?').next().unwrap_or("/");
            let dir = match base_path.rfind('/') {
                Some(idx) => &base_path[..=idx],
                None => "/",
            };
            normalize_path(&format!("{dir}{no_frag}"))?
        };
        if path.bytes().any(|b| b <= 0x20 || b == 0x7f) {
            return Err(HttpError::InvalidResponse);
        }
        Ok(Self {
            host: self.host.clone(),
            port: self.port,
            path,
        })
    }
}

fn strip_scheme<'a>(s: &'a str, scheme: &str) -> Option<&'a str> {
    if s.len() >= scheme.len() && s[..scheme.len()].eq_ignore_ascii_case(scheme) {
        Some(&s[scheme.len()..])
    } else {
        None
    }
}

fn has_ctl(s: &str) -> bool {
    s.bytes().any(|b| b < 0x20 || b == 0x7f)
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}

fn normalize_path(raw: &str) -> Result<String, HttpError> {
    let (path_only, query) = match raw.find('?') {
        Some(idx) => (&raw[..idx], Some(&raw[idx..])),
        None => (raw, None),
    };
    let trailing_slash = path_only.ends_with('/') && path_only.len() > 1;
    let mut segs: Vec<&str> = Vec::new();
    for seg in path_only.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    let mut out = String::from("/");
    for (i, seg) in segs.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(seg);
    }
    if trailing_slash && out.len() > 1 {
        out.push('/');
    }
    if let Some(q) = query {
        out.push_str(q);
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub method: Method,
    pub url: Url,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
    pub keep_alive: bool,
}

impl Request {
    pub fn new(method: Method, url: &str) -> Result<Self, HttpError> {
        Ok(Self {
            method,
            url: Url::parse(url)?,
            headers: Vec::new(),
            body: Vec::new(),
            keep_alive: true,
        })
    }

    pub fn header(mut self, name: &str, value: &str) -> Result<Self, HttpError> {
        validate_header(name, value)?;
        self.headers.push(Header {
            name: name.to_string(),
            value: value.to_string(),
        });
        Ok(self)
    }

    pub fn body(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.body = bytes.into();
        self
    }

    pub fn keep_alive(mut self, keep_alive: bool) -> Self {
        self.keep_alive = keep_alive;
        self
    }
}

fn validate_header(name: &str, value: &str) -> Result<(), HttpError> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(HttpError::InvalidHeader);
    }
    if value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return Err(HttpError::InvalidHeader);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub url: Url,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }
}

pub trait Transport {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, HttpError>;
    fn write_all(&mut self, buf: &[u8]) -> Result<(), HttpError>;
}

pub trait Connector {
    type Conn: Transport;
    fn connect(&mut self, host: &str, port: u16) -> Result<Self::Conn, HttpError>;
}

struct BufferedConn<T> {
    inner: T,
    buf: Vec<u8>,
    pos: usize,
}

impl<T: Transport> BufferedConn<T> {
    fn new(inner: T, leftover: Vec<u8>) -> Self {
        Self {
            inner,
            buf: leftover,
            pos: 0,
        }
    }

    fn take_leftover(mut self) -> (T, Vec<u8>) {
        let rest = if self.pos < self.buf.len() {
            self.buf.split_off(self.pos)
        } else {
            Vec::new()
        };
        (self.inner, rest)
    }

    fn fill(&mut self) -> Result<bool, HttpError> {
        if self.pos < self.buf.len() {
            return Ok(true);
        }
        self.buf.clear();
        self.pos = 0;
        let mut tmp = [0u8; 4096];
        let n = self.inner.read(&mut tmp)?;
        if n == 0 {
            return Ok(false);
        }
        self.buf.extend_from_slice(&tmp[..n]);
        Ok(true)
    }

    fn read_exact_limited(&mut self, out: &mut Vec<u8>, count: usize) -> Result<(), HttpError> {
        let mut rem = count;
        while rem > 0 {
            if self.pos < self.buf.len() {
                let take = rem.min(self.buf.len() - self.pos);
                out.extend_from_slice(&self.buf[self.pos..self.pos + take]);
                self.pos += take;
                rem -= take;
            } else if !self.fill()? {
                return Err(HttpError::Truncated);
            }
        }
        Ok(())
    }

    fn read_to_eof(&mut self, out: &mut Vec<u8>, limit: usize) -> Result<(), HttpError> {
        loop {
            if self.pos < self.buf.len() {
                let avail = self.buf.len() - self.pos;
                if out.len() + avail > limit {
                    return Err(HttpError::TooLarge);
                }
                out.extend_from_slice(&self.buf[self.pos..]);
                self.pos = self.buf.len();
            }
            if !self.fill()? {
                return Ok(());
            }
        }
    }

    fn read_line_crlf(&mut self, max_len: usize) -> Result<String, HttpError> {
        let mut line = Vec::new();
        loop {
            while self.pos < self.buf.len() {
                let b = self.buf[self.pos];
                self.pos += 1;
                if line.len() >= max_len {
                    return Err(HttpError::TooLarge);
                }
                line.push(b);
                if line.ends_with(b"\r\n") {
                    line.truncate(line.len() - 2);
                    return String::from_utf8(line).map_err(|_| HttpError::InvalidResponse);
                }
            }
            if !self.fill()? {
                return Err(HttpError::Truncated);
            }
        }
    }

    fn read_headers_block(&mut self, limit: usize) -> Result<(Vec<u8>, bool), HttpError> {
        let mut raw = Vec::new();
        let mut any_bytes = false;
        loop {
            if self.pos == self.buf.len() && !self.fill()? {
                return Err(if !any_bytes {
                    HttpError::Truncated
                } else {
                    HttpError::InvalidResponse
                });
            }
            any_bytes = true;
            while self.pos < self.buf.len() {
                if raw.len() >= limit {
                    return Err(HttpError::TooLarge);
                }
                raw.push(self.buf[self.pos]);
                self.pos += 1;
                if raw.ends_with(b"\r\n\r\n") {
                    raw.truncate(raw.len() - 4);
                    return Ok((raw, true));
                }
            }
        }
    }
}

struct PooledConn<T> {
    host: String,
    port: u16,
    conn: T,
    leftover: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionStats {
    pub opened: usize,
    pub reused: usize,
}

pub struct Session<C: Connector> {
    connector: C,
    limits: Limits,
    pool: VecDeque<PooledConn<C::Conn>>,
    stats: SessionStats,
}

impl<C: Connector> Session<C> {
    pub fn new(connector: C, limits: Limits) -> Self {
        Self {
            connector,
            limits,
            pool: VecDeque::new(),
            stats: SessionStats::default(),
        }
    }

    pub fn with_defaults(connector: C) -> Self {
        Self::new(connector, Limits::default())
    }

    pub const fn stats(&self) -> SessionStats {
        self.stats
    }

    pub fn idle_connections(&self) -> usize {
        self.pool.len()
    }

    pub fn send(&mut self, request: &Request) -> Result<Response, HttpError> {
        let mut method = request.method;
        let mut url = request.url.clone();
        let mut body = request.body.clone();

        for hop in 0..=self.limits.max_redirects {
            let (raw_resp, conn_for_reuse) =
                self.exchange_once(method, &url, &request.headers, &body, request.keep_alive)?;
            if let Some(pc) = conn_for_reuse {
                self.return_to_pool(pc);
            }

            if is_redirect(raw_resp.status) {
                if let Some(location) = raw_resp.header("Location") {
                    if hop == self.limits.max_redirects {
                        return Err(HttpError::TooManyRedirects);
                    }
                    url = url.resolve_location(location)?;
                    if raw_resp.status == 303
                        || (matches!(raw_resp.status, 301 | 302) && method == Method::Post)
                    {
                        method = Method::Get;
                        body.clear();
                    }
                    continue;
                }
            }

            let encoding = raw_resp.header("Content-Encoding").map(str::to_string);
            let decoded = decode_body(
                encoding.as_deref(),
                raw_resp.body,
                self.limits.max_decompressed_bytes,
            )?;
            return Ok(Response {
                status: raw_resp.status,
                reason: raw_resp.reason,
                url: raw_resp.url,
                headers: raw_resp.headers,
                body: decoded,
            });
        }
        Err(HttpError::TooManyRedirects)
    }

    fn take_from_pool(&mut self, host: &str, port: u16) -> Option<(C::Conn, Vec<u8>)> {
        let idx = self
            .pool
            .iter()
            .position(|entry| entry.host == host && entry.port == port)?;
        let entry = self.pool.remove(idx)?;
        Some((entry.conn, entry.leftover))
    }

    fn return_to_pool(&mut self, entry: PooledConn<C::Conn>) {
        if self.limits.max_idle_connections == 0 {
            return;
        }
        while self.pool.len() >= self.limits.max_idle_connections {
            self.pool.pop_front();
        }
        self.pool.push_back(entry);
    }

    fn exchange_once(
        &mut self,
        method: Method,
        url: &Url,
        extra_headers: &[Header],
        body: &[u8],
        keep_alive: bool,
    ) -> Result<(Response, Option<PooledConn<C::Conn>>), HttpError> {
        let mut from_pool = self.take_from_pool(&url.host, url.port);
        loop {
            let reused = from_pool.is_some();
            let (mut conn, leftover) = match from_pool.take() {
                Some(pair) => {
                    self.stats.reused += 1;
                    pair
                }
                None => {
                    let conn = self.connector.connect(&url.host, url.port)?;
                    self.stats.opened += 1;
                    (conn, Vec::new())
                }
            };
            let wire = build_request(method, url, extra_headers, body, keep_alive)?;
            if let Err(err) = conn.write_all(&wire) {
                if reused && method.is_idempotent() {
                    continue;
                }
                return Err(err);
            }
            let mut reader = BufferedConn::new(conn, leftover);
            let outcome = read_response(&mut reader, method, url, &self.limits);
            match outcome {
                Ok((resp, reusable)) => {
                    let keep = if keep_alive && reusable {
                        let (inner, rest) = reader.take_leftover();
                        Some(PooledConn {
                            host: url.host.clone(),
                            port: url.port,
                            conn: inner,
                            leftover: rest,
                        })
                    } else {
                        None
                    };
                    return Ok((resp, keep));
                }
                Err(HttpError::Truncated) if reused && method.is_idempotent() => continue,
                Err(err) => return Err(err),
            }
        }
    }
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn build_request(
    method: Method,
    url: &Url,
    extra: &[Header],
    body: &[u8],
    keep_alive: bool,
) -> Result<Vec<u8>, HttpError> {
    let mut out = format!("{} {} HTTP/1.1\r\n", method.as_str(), url.path);
    let has = |target: &str| extra.iter().any(|h| h.name.eq_ignore_ascii_case(target));
    if !has("Host") {
        out.push_str(&format!("Host: {}\r\n", url.host_header()));
    }
    if !has("User-Agent") {
        out.push_str("User-Agent: dunit-http/0.1\r\n");
    }
    if !has("Accept-Encoding") {
        out.push_str("Accept-Encoding: gzip, deflate\r\n");
    }
    if !has("Connection") {
        out.push_str(if keep_alive {
            "Connection: keep-alive\r\n"
        } else {
            "Connection: close\r\n"
        });
    }
    for h in extra {
        validate_header(&h.name, &h.value)?;
        if h.name.eq_ignore_ascii_case("Content-Length")
            || h.name.eq_ignore_ascii_case("Transfer-Encoding")
        {
            return Err(HttpError::InvalidHeader);
        }
        out.push_str(&format!("{}: {}\r\n", h.name, h.value));
    }
    if !body.is_empty() || matches!(method, Method::Post | Method::Put) {
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    Ok(bytes)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Framing {
    Empty,
    Length(usize),
    Chunked,
    CloseDelimited,
}

fn read_response<T: Transport>(
    reader: &mut BufferedConn<T>,
    method: Method,
    url: &Url,
    limits: &Limits,
) -> Result<(Response, bool), HttpError> {
    let (raw_head, _) = reader.read_headers_block(limits.max_header_bytes)?;
    let head_str = core::str::from_utf8(&raw_head).map_err(|_| HttpError::InvalidResponse)?;
    let mut lines = head_str.split("\r\n");
    let status_line = lines.next().ok_or(HttpError::InvalidResponse)?;
    let (version, status, reason) = parse_status_line(status_line)?;

    let mut headers = Vec::new();
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    let mut conn_close = version == 0;

    for line in lines {
        if headers.len() >= limits.max_headers {
            return Err(HttpError::TooLarge);
        }
        let (name_raw, val_raw) = line.split_once(':').ok_or(HttpError::InvalidResponse)?;
        let name = name_raw.trim();
        if name != name_raw || name.is_empty() || has_ctl(name) {
            return Err(HttpError::InvalidResponse);
        }
        let value = val_raw.trim();
        if value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
            return Err(HttpError::InvalidResponse);
        }
        if name.eq_ignore_ascii_case("Content-Length") {
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(HttpError::InvalidResponse);
            }
            let parsed: usize = value.parse().map_err(|_| HttpError::TooLarge)?;
            if let Some(prev) = content_length {
                if prev != parsed {
                    return Err(HttpError::InvalidResponse);
                }
            }
            content_length = Some(parsed);
        } else if name.eq_ignore_ascii_case("Transfer-Encoding") {
            let last = value
                .rsplit(',')
                .next()
                .map(str::trim)
                .unwrap_or("");
            if last.eq_ignore_ascii_case("chunked") {
                chunked = true;
            } else {
                return Err(HttpError::InvalidResponse);
            }
        } else if name.eq_ignore_ascii_case("Connection") {
            for token in value.split(',').map(str::trim) {
                if token.eq_ignore_ascii_case("close") {
                    conn_close = true;
                } else if token.eq_ignore_ascii_case("keep-alive") && version == 0 {
                    conn_close = false;
                }
            }
        }
        headers.push(Header {
            name: name.to_string(),
            value: value.to_string(),
        });
    }

    if chunked && content_length.is_some() {
        return Err(HttpError::InvalidResponse);
    }

    let framing = if method == Method::Head || matches!(status, 100..=199 | 204 | 304) {
        Framing::Empty
    } else if chunked {
        Framing::Chunked
    } else if let Some(len) = content_length {
        Framing::Length(len)
    } else {
        Framing::CloseDelimited
    };

    let mut body = Vec::new();
    let mut reusable = !conn_close;
    match framing {
        Framing::Empty => {}
        Framing::Length(len) => {
            if len > limits.max_body_bytes {
                return Err(HttpError::TooLarge);
            }
            reader.read_exact_limited(&mut body, len)?;
        }
        Framing::Chunked => {
            read_chunked_body(reader, &mut body, limits)?;
        }
        Framing::CloseDelimited => {
            reusable = false;
            reader.read_to_eof(&mut body, limits.max_body_bytes)?;
        }
    }

    Ok((
        Response {
            status,
            reason: reason.to_string(),
            url: url.clone(),
            headers,
            body,
        },
        reusable,
    ))
}

fn parse_status_line(line: &str) -> Result<(u8, u16, &str), HttpError> {
    let (proto, rest) = line.split_once(' ').ok_or(HttpError::InvalidResponse)?;
    let minor = match proto {
        "HTTP/1.1" => 1u8,
        "HTTP/1.0" => 0u8,
        _ => return Err(HttpError::InvalidResponse),
    };
    let (code_str, reason) = match rest.split_once(' ') {
        Some((c, r)) => (c, r),
        None => (rest, ""),
    };
    if code_str.len() != 3 || !code_str.bytes().all(|b| b.is_ascii_digit()) {
        return Err(HttpError::InvalidResponse);
    }
    let code: u16 = code_str.parse().map_err(|_| HttpError::InvalidResponse)?;
    if !(100..=599).contains(&code) {
        return Err(HttpError::InvalidResponse);
    }
    Ok((minor, code, reason))
}

fn read_chunked_body<T: Transport>(
    reader: &mut BufferedConn<T>,
    out: &mut Vec<u8>,
    limits: &Limits,
) -> Result<(), HttpError> {
    loop {
        let size_line = reader.read_line_crlf(limits.max_chunk_line)?;
        let hex = size_line.split(';').next().unwrap_or("").trim();
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(HttpError::InvalidResponse);
        }
        let chunk_len = usize::from_str_radix(hex, 16).map_err(|_| HttpError::TooLarge)?;
        if chunk_len == 0 {
            let mut seen = 0usize;
            loop {
                let trailer = reader.read_line_crlf(limits.max_header_bytes)?;
                if trailer.is_empty() {
                    return Ok(());
                }
                seen += 1;
                if seen > limits.max_headers {
                    return Err(HttpError::TooLarge);
                }
            }
        }
        if out.len().checked_add(chunk_len).is_none_or(|n| n > limits.max_body_bytes) {
            return Err(HttpError::TooLarge);
        }
        reader.read_exact_limited(out, chunk_len)?;
        let term = reader.read_line_crlf(4)?;
        if !term.is_empty() {
            return Err(HttpError::InvalidResponse);
        }
    }
}

fn decode_body(
    encoding: Option<&str>,
    raw: Vec<u8>,
    limit: usize,
) -> Result<Vec<u8>, HttpError> {
    let Some(enc) = encoding.map(str::trim).filter(|s| !s.is_empty()) else {
        if raw.len() > limit {
            return Err(HttpError::TooLarge);
        }
        return Ok(raw);
    };
    if enc.eq_ignore_ascii_case("identity") {
        if raw.len() > limit {
            return Err(HttpError::TooLarge);
        }
        Ok(raw)
    } else if enc.eq_ignore_ascii_case("gzip") || enc.eq_ignore_ascii_case("x-gzip") {
        inflate::inflate_gzip(&raw, limit)
    } else if enc.eq_ignore_ascii_case("deflate") {
        inflate::inflate_zlib(&raw, limit)
    } else {
        Err(HttpError::UnsupportedEncoding)
    }
}

#[cfg(test)]
mod tests;
