//! Bounded, SHA-256 verified cookie jar (`DNCK` v1) with origin, Secure, and expiry enforcement.
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use dunit_http_core::{Header, Scheme, Url};
use dunit_tls_core::crypto::Sha256;

const MAGIC: &[u8; 4] = b"DNCK";
const VERSION: u16 = 1;
const HEADER_LEN: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub expires_at: Option<u64>,
}

pub struct CookieJar {
    cookies: Vec<Cookie>,
    max_per_origin: usize,
    max_total: usize,
    pub corrupt_reset_count: u32,
}

impl CookieJar {
    pub fn new(max_per_origin: usize, max_total: usize) -> Self {
        Self {
            cookies: Vec::new(),
            max_per_origin: max_per_origin.max(1),
            max_total: max_total.max(1),
            corrupt_reset_count: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    pub fn apply_response_headers(&mut self, url: &Url, headers: &[Header], now_unix: u64) -> bool {
        let mut modified = false;
        for h in headers {
            if !h.name.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            if let Some(parsed) = parse_set_cookie(url, &h.value, now_unix) {
                self.upsert(parsed, now_unix);
                modified = true;
            }
        }
        modified
    }

    pub fn cookie_header_for(&mut self, url: &Url, now_unix: u64) -> Option<String> {
        self.purge_expired(now_unix);
        let host = url.host.to_ascii_lowercase();
        let is_https = url.scheme == Scheme::Https;
        let path = url.path.split('?').next().unwrap_or("/");
        let mut pairs = Vec::new();
        for c in &self.cookies {
            if c.secure && !is_https {
                continue;
            }
            if !domain_matches(&host, &c.domain, c.host_only) {
                continue;
            }
            if !path_matches(path, &c.path) {
                continue;
            }
            pairs.push(format!("{}={}", c.name, c.value));
        }
        if pairs.is_empty() {
            None
        } else {
            Some(pairs.join("; "))
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&(self.cookies.len() as u32).to_le_bytes());
        for c in &self.cookies {
            push_str(&mut payload, &c.name);
            push_str(&mut payload, &c.value);
            push_str(&mut payload, &c.domain);
            push_str(&mut payload, &c.path);
            let flags = (c.host_only as u8)
                | ((c.secure as u8) << 1)
                | ((c.http_only as u8) << 2)
                | ((c.expires_at.is_some() as u8) << 3);
            payload.push(flags);
            payload.extend_from_slice(&c.expires_at.unwrap_or(0).to_le_bytes());
        }
        let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        let mut hasher = Sha256::new();
        hasher.update(&out[..10]);
        hasher.update(&payload);
        let digest = hasher.finalize();
        out.extend_from_slice(&digest[..30]);
        out.extend_from_slice(&payload);
        out
    }

    pub fn load_blob(&mut self, blob: &[u8], now_unix: u64) -> bool {
        match self.decode_blob(blob, now_unix) {
            Ok(loaded) => {
                self.cookies = loaded;
                true
            }
            Err(()) => {
                self.cookies.clear();
                self.corrupt_reset_count += 1;
                false
            }
        }
    }

    fn decode_blob(&self, blob: &[u8], now_unix: u64) -> Result<Vec<Cookie>, ()> {
        if blob.len() < HEADER_LEN || &blob[..4] != MAGIC {
            return Err(());
        }
        let version = u16::from_le_bytes([blob[4], blob[5]]);
        if version != VERSION {
            return Err(());
        }
        let payload_len = u32::from_le_bytes(blob[6..10].try_into().map_err(|_| ())?) as usize;
        if HEADER_LEN.checked_add(payload_len) != Some(blob.len()) {
            return Err(());
        }
        let mut hasher = Sha256::new();
        hasher.update(&blob[..10]);
        hasher.update(&blob[HEADER_LEN..]);
        let digest = hasher.finalize();
        if digest[..30] != blob[10..40] {
            return Err(());
        }
        let payload = &blob[HEADER_LEN..];
        if payload.len() < 4 {
            return Err(());
        }
        let count = u32::from_le_bytes(payload[..4].try_into().map_err(|_| ())?) as usize;
        if count > self.max_total * 2 {
            return Err(());
        }
        let mut pos = 4usize;
        let mut out = Vec::with_capacity(count.min(self.max_total));
        for _ in 0..count {
            let name = read_str(payload, &mut pos)?;
            let value = read_str(payload, &mut pos)?;
            let domain = read_str(payload, &mut pos)?;
            let path = read_str(payload, &mut pos)?;
            if pos + 9 > payload.len() {
                return Err(());
            }
            let flags = payload[pos];
            pos += 1;
            let raw_exp = u64::from_le_bytes(payload[pos..pos + 8].try_into().map_err(|_| ())?);
            pos += 8;
            let expires_at = (flags & 8 != 0).then_some(raw_exp);
            if expires_at.is_some_and(|t| t <= now_unix) {
                continue;
            }
            if out.len() < self.max_total {
                out.push(Cookie {
                    name,
                    value,
                    domain,
                    host_only: flags & 1 != 0,
                    path,
                    secure: flags & 2 != 0,
                    http_only: flags & 4 != 0,
                    expires_at,
                });
            }
        }
        if pos != payload.len() {
            return Err(());
        }
        Ok(out)
    }

    fn upsert(&mut self, incoming: Cookie, now_unix: u64) {
        self.purge_expired(now_unix);
        self.cookies.retain(|c| {
            !(c.name == incoming.name && c.domain == incoming.domain && c.path == incoming.path)
        });
        if incoming.expires_at.is_some_and(|exp| exp <= now_unix) {
            return;
        }
        let same_domain_count = self
            .cookies
            .iter()
            .filter(|c| c.domain == incoming.domain)
            .count();
        if same_domain_count >= self.max_per_origin {
            if let Some(pos) = self.cookies.iter().position(|c| c.domain == incoming.domain) {
                self.cookies.remove(pos);
            }
        }
        while self.cookies.len() >= self.max_total {
            self.cookies.remove(0);
        }
        self.cookies.push(incoming);
    }

    fn purge_expired(&mut self, now_unix: u64) {
        self.cookies.retain(|c| !c.expires_at.is_some_and(|t| t <= now_unix));
    }
}

fn parse_set_cookie(url: &Url, header: &str, now_unix: u64) -> Option<Cookie> {
    if header.len() > 4096 {
        return None;
    }
    let mut parts = header.split(';');
    let first = parts.next()?.trim();
    let (name, value) = first.split_once('=')?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || name.len() > 128 || value.len() > 2048 {
        return None;
    }
    let req_host = url.host.to_ascii_lowercase();
    let mut cookie = Cookie {
        name: String::from(name),
        value: String::from(value),
        domain: req_host.clone(),
        host_only: true,
        path: default_cookie_path(&url.path),
        secure: false,
        http_only: false,
        expires_at: None,
    };

    for attr in parts {
        let attr = attr.trim();
        if attr.is_empty() {
            continue;
        }
        if let Some((k, v)) = attr.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if k.eq_ignore_ascii_case("domain") {
                let dom = v.trim_start_matches('.').to_ascii_lowercase();
                if dom.is_empty() || !domain_matches(&req_host, &dom, false) {
                    return None;
                }
                cookie.domain = dom;
                cookie.host_only = false;
            } else if k.eq_ignore_ascii_case("path") {
                if v.starts_with('/') {
                    cookie.path = String::from(v);
                }
            } else if k.eq_ignore_ascii_case("max-age") {
                if let Ok(secs) = v.parse::<i64>() {
                    cookie.expires_at = Some(if secs <= 0 {
                        now_unix
                    } else {
                        now_unix.saturating_add(secs as u64)
                    });
                }
            }
        } else if attr.eq_ignore_ascii_case("secure") {
            if url.scheme != Scheme::Https {
                return None;
            }
            cookie.secure = true;
        } else if attr.eq_ignore_ascii_case("httponly") {
            cookie.http_only = true;
        }
    }
    Some(cookie)
}

fn default_cookie_path(url_path: &str) -> String {
    let clean = url_path.split('?').next().unwrap_or("/");
    if let Some(pos) = clean.rfind('/') {
        if pos == 0 {
            String::from("/")
        } else {
            clean[..pos].to_string()
        }
    } else {
        String::from("/")
    }
}

fn domain_matches(host: &str, domain: &str, host_only: bool) -> bool {
    if host_only {
        return host == domain;
    }
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

fn path_matches(req_path: &str, cookie_path: &str) -> bool {
    if !req_path.starts_with(cookie_path) {
        return false;
    }
    cookie_path.ends_with('/')
        || req_path.len() == cookie_path.len()
        || req_path.as_bytes().get(cookie_path.len()) == Some(&b'/')
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len().min(u16::MAX as usize) as u16).to_le_bytes());
    out.extend_from_slice(&bytes[..bytes.len().min(u16::MAX as usize)]);
}

fn read_str(bytes: &[u8], pos: &mut usize) -> Result<String, ()> {
    if *pos + 2 > bytes.len() {
        return Err(());
    }
    let len = u16::from_le_bytes([bytes[*pos], bytes[*pos + 1]]) as usize;
    *pos += 2;
    if *pos + len > bytes.len() {
        return Err(());
    }
    let s = String::from_utf8(bytes[*pos..*pos + len].to_vec()).map_err(|_| ())?;
    *pos += len;
    Ok(s)
}
