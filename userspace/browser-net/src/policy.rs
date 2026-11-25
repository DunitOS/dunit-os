//! Origin, proxy, download, quota, and trust policies for the isolated browser network service.
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;
use dunit_http_core::{Header, HttpError, Method, Scheme, Url};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
}

impl Origin {
    pub fn from_url(url: &Url) -> Self {
        Self {
            scheme: url.scheme,
            host: url.host.to_ascii_lowercase(),
            port: url.port,
        }
    }

    pub fn is_secure(&self) -> bool {
        self.scheme == Scheme::Https
    }

    pub fn same_origin(&self, other: &Self) -> bool {
        self.scheme == other.scheme && self.host == other.host && self.port == other.port
    }

    pub fn canonical(&self) -> String {
        let host_fmt = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == self.scheme.default_port() {
            format!("{}://{host_fmt}", self.scheme.as_str())
        } else {
            format!("{}://{host_fmt}:{}", self.scheme.as_str(), self.port)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceIntent {
    Document,
    Subresource,
    Download,
}

impl ResourceIntent {
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Document => 0,
            Self::Subresource => 1,
            Self::Download => 2,
        }
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Document),
            1 => Some(Self::Subresource),
            2 => Some(Self::Download),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyViolation {
    BlockedScheme,
    BlockedHost,
    PrivateAddressDenied,
    CrossOriginDenied,
    DowngradeDenied,
    MethodDenied,
    ExecutableDownloadOnly,
    QuotaExceeded,
    UntrustedProxy,
}

impl fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::BlockedScheme => "blocked URL scheme",
            Self::BlockedHost => "blocked destination host",
            Self::PrivateAddressDenied => "private network access denied",
            Self::CrossOriginDenied => "cross-origin access denied",
            Self::DowngradeDenied => "HTTPS to HTTP downgrade denied",
            Self::MethodDenied => "HTTP method not permitted by policy",
            Self::ExecutableDownloadOnly => "executable MIME must use download flow",
            Self::QuotaExceeded => "resource quota exceeded",
            Self::UntrustedProxy => "invalid proxy target",
        };
        f.write_str(label)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginPolicy {
    pub allow_http: bool,
    pub allow_https: bool,
    pub require_https_subresources: bool,
    pub allow_private_ips: bool,
    pub allow_cross_origin_redirect: bool,
    pub allow_hosts: Vec<String>,
    pub deny_hosts: Vec<String>,
}

impl Default for OriginPolicy {
    fn default() -> Self {
        Self {
            allow_http: false,
            allow_https: true,
            require_https_subresources: true,
            allow_private_ips: false,
            allow_cross_origin_redirect: true,
            allow_hosts: Vec::new(),
            deny_hosts: alloc::vec![
                String::from("169.254.169.254"),
                String::from("metadata.google.internal"),
            ],
        }
    }
}

impl OriginPolicy {
    pub fn check_url(&self, url: &Url, intent: ResourceIntent) -> Result<(), PolicyViolation> {
        match url.scheme {
            Scheme::Https if !self.allow_https => return Err(PolicyViolation::BlockedScheme),
            Scheme::Http => {
                if !self.allow_http {
                    return Err(PolicyViolation::BlockedScheme);
                }
                if intent == ResourceIntent::Subresource && self.require_https_subresources {
                    return Err(PolicyViolation::BlockedScheme);
                }
            }
            _ => {}
        }

        let host = url.host.to_ascii_lowercase();
        if self.deny_hosts.iter().any(|blocked| host_matches(&host, blocked)) {
            return Err(PolicyViolation::BlockedHost);
        }
        if !self.allow_private_ips && is_private_or_loopback_host(&host) {
            return Err(PolicyViolation::PrivateAddressDenied);
        }
        if !self.allow_hosts.is_empty()
            && !self.allow_hosts.iter().any(|allowed| host_matches(&host, allowed))
        {
            return Err(PolicyViolation::BlockedHost);
        }
        Ok(())
    }

    pub fn check_redirect(&self, from: &Url, to: &Url) -> Result<(), PolicyViolation> {
        if from.scheme == Scheme::Https && to.scheme == Scheme::Http {
            return Err(PolicyViolation::DowngradeDenied);
        }
        if !self.allow_cross_origin_redirect
            && !Origin::from_url(from).same_origin(&Origin::from_url(to))
        {
            return Err(PolicyViolation::CrossOriginDenied);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyRoute {
    Direct,
    HttpForward { host: String, port: u16 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyPolicy {
    pub route: ProxyRoute,
    pub no_proxy: Vec<String>,
}

impl Default for ProxyPolicy {
    fn default() -> Self {
        Self {
            route: ProxyRoute::Direct,
            no_proxy: Vec::new(),
        }
    }
}

impl ProxyPolicy {
    pub fn resolve(&self, host: &str) -> Result<ProxyRoute, PolicyViolation> {
        let lower = host.to_ascii_lowercase();
        if self.no_proxy.iter().any(|pattern| host_matches(&lower, pattern)) {
            return Ok(ProxyRoute::Direct);
        }
        if let ProxyRoute::HttpForward { host: proxy_host, port } = &self.route {
            if proxy_host.is_empty() || *port == 0 {
                return Err(PolicyViolation::UntrustedProxy);
            }
        }
        Ok(self.route.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadPolicy {
    pub max_download_bytes: usize,
    pub max_inline_bytes: usize,
    pub block_executable_inline: bool,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        Self {
            max_download_bytes: 512 * 1024,
            max_inline_bytes: 256 * 1024,
            block_executable_inline: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseDisposition {
    pub is_download: bool,
    pub filename: Option<String>,
    pub content_type: String,
}

impl DownloadPolicy {
    pub fn classify(
        &self,
        url: &Url,
        headers: &[Header],
        body_len: usize,
        intent: ResourceIntent,
    ) -> Result<ResponseDisposition, PolicyViolation> {
        let content_type = header_value(headers, "content-type")
            .unwrap_or("application/octet-stream")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let disp = header_value(headers, "content-disposition");
        let attachment_name = disp.and_then(parse_disposition_filename);
        let forced_attachment = disp.is_some_and(|v| v.to_ascii_lowercase().starts_with("attachment"));
        let is_executable_mime = matches!(
            content_type.as_str(),
            "application/x-executable"
                | "application/x-elf"
                | "application/x-msdownload"
                | "application/vnd.microsoft.portable-executable"
        );
        if is_executable_mime && self.block_executable_inline && intent == ResourceIntent::Document && !forced_attachment {
            return Err(PolicyViolation::ExecutableDownloadOnly);
        }

        let is_download = intent == ResourceIntent::Download
            || forced_attachment
            || content_type == "application/octet-stream"
            || is_executable_mime;
        let limit = if is_download {
            self.max_download_bytes
        } else {
            self.max_inline_bytes
        };
        if body_len > limit {
            return Err(PolicyViolation::QuotaExceeded);
        }
        let filename = if is_download {
            Some(sanitize_filename(
                attachment_name
                    .as_deref()
                    .or_else(|| url_leaf(&url.path))
                    .unwrap_or("download.bin"),
            ))
        } else {
            None
        };
        Ok(ResponseDisposition {
            is_download,
            filename,
            content_type,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceQuotas {
    pub max_header_bytes: usize,
    pub max_headers: usize,
    pub max_body_bytes: usize,
    pub max_decompressed_bytes: usize,
    pub max_redirects: usize,
    pub max_cache_entries: usize,
    pub max_cache_entry_bytes: usize,
    pub max_cache_total_bytes: usize,
    pub max_cookies_per_origin: usize,
    pub max_total_cookies: usize,
    pub max_inflight_per_origin: usize,
    pub max_shm_bytes: usize,
}

impl Default for ResourceQuotas {
    fn default() -> Self {
        Self {
            max_header_bytes: 16 * 1024,
            max_headers: 64,
            max_body_bytes: 256 * 1024,
            max_decompressed_bytes: 512 * 1024,
            max_redirects: 4,
            max_cache_entries: 32,
            max_cache_entry_bytes: 128 * 1024,
            max_cache_total_bytes: 512 * 1024,
            max_cookies_per_origin: 16,
            max_total_cookies: 128,
            max_inflight_per_origin: 4,
            max_shm_bytes: 1024 * 1024,
        }
    }
}

impl ResourceQuotas {
    pub fn to_http_limits(&self) -> dunit_http_core::Limits {
        dunit_http_core::Limits {
            max_header_bytes: self.max_header_bytes,
            max_headers: self.max_headers,
            max_body_bytes: self.max_body_bytes,
            max_decompressed_bytes: self.max_decompressed_bytes,
            max_chunk_line: 1024,
            max_redirects: self.max_redirects,
            max_idle_connections: 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserPolicy {
    pub origins: OriginPolicy,
    pub proxy: ProxyPolicy,
    pub downloads: DownloadPolicy,
    pub quotas: ResourceQuotas,
    pub allow_methods: [bool; 5],
}

impl Default for BrowserPolicy {
    fn default() -> Self {
        Self {
            origins: OriginPolicy::default(),
            proxy: ProxyPolicy::default(),
            downloads: DownloadPolicy::default(),
            quotas: ResourceQuotas::default(),
            allow_methods: [true, true, true, false, false],
        }
    }
}

impl BrowserPolicy {
    pub fn check_method(&self, method: Method) -> Result<(), PolicyViolation> {
        let allowed = match method {
            Method::Get => self.allow_methods[0],
            Method::Head => self.allow_methods[1],
            Method::Post => self.allow_methods[2],
            Method::Put => self.allow_methods[3],
            Method::Delete => self.allow_methods[4],
        };
        if allowed {
            Ok(())
        } else {
            Err(PolicyViolation::MethodDenied)
        }
    }

    pub fn check_request(&self, method: Method, url: &Url, intent: ResourceIntent) -> Result<ProxyRoute, PolicyViolation> {
        self.check_method(method)?;
        self.origins.check_url(url, intent)?;
        self.proxy.resolve(&url.host)
    }

    pub fn map_http_error(err: &HttpError) -> &'static str {
        match err {
            HttpError::TooManyRedirects => "redirect-loop",
            HttpError::Truncated => "truncated-framing",
            HttpError::Decompress => "decompression-error",
            HttpError::TooLarge => "quota-exceeded",
            HttpError::Tls(tls) => match tls {
                dunit_http_core::TlsFailure::CertificateExpired => "tls-expired-cert",
                dunit_http_core::TlsFailure::InvalidCertificate => "tls-invalid-cert",
                dunit_http_core::TlsFailure::UntrustedRoot => "tls-untrusted-root",
                dunit_http_core::TlsFailure::BadSignature => "tls-bad-signature",
                _ => "tls-handshake",
            },
            HttpError::UnsupportedScheme => "unsupported-scheme",
            HttpError::InvalidUrl => "invalid-url",
            HttpError::InvalidHeader | HttpError::InvalidResponse | HttpError::UnsupportedEncoding => "malformed-http",
            HttpError::Transport(_) => "transport-error",
        }
    }
}

pub fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let mut out = String::new();
    for ch in base.chars() {
        if out.len() >= 96 {
            break;
        }
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            out.push(ch);
        } else if ch == ' ' {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('.');
    if trimmed.is_empty() {
        String::from("download.bin")
    } else {
        String::from(trimmed)
    }
}

fn url_leaf(path: &str) -> Option<&str> {
    let clean = path.split('?').next().unwrap_or(path);
    let leaf = clean.rsplit('/').next()?;
    if leaf.is_empty() { None } else { Some(leaf) }
}

fn parse_disposition_filename(header: &str) -> Option<String> {
    for part in header.split(';') {
        let part = part.trim();
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case("filename") {
            let v = v.trim();
            let unquoted = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v);
            if !unquoted.is_empty() {
                return Some(unquoted.to_string());
            }
        }
    }
    None
}

fn header_value<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn host_matches(host: &str, pattern: &str) -> bool {
    let p = pattern.trim().to_ascii_lowercase();
    if let Some(suffix) = p.strip_prefix("*.") {
        host == suffix || (host.len() > suffix.len() && host.ends_with(suffix) && host.as_bytes()[host.len() - suffix.len() - 1] == b'.')
    } else if let Some(suffix) = p.strip_prefix('.') {
        host == suffix || (host.len() > suffix.len() && host.ends_with(suffix) && host.as_bytes()[host.len() - suffix.len() - 1] == b'.')
    } else {
        host == p
    }
}

fn is_private_or_loopback_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    if let Some(ip) = parse_ipv4(host) {
        return is_private_v4(ip);
    }
    if let Some(ip6) = parse_ipv6(host) {
        return ip6 == [0; 16]
            || ip6 == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
            || (ip6[0] == 0xfe && (ip6[1] & 0xc0) == 0x80)
            || (ip6[0] & 0xfe == 0xfc)
            || (ip6[..12] == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff]
                && is_private_v4([ip6[12], ip6[13], ip6[14], ip6[15]]));
    }
    false
}

fn is_private_v4(ip: [u8; 4]) -> bool {
    ip[0] == 127
        || ip[0] == 10
        || (ip[0] == 172 && (16..=31).contains(&ip[1]))
        || (ip[0] == 192 && ip[1] == 168)
        || (ip[0] == 169 && ip[1] == 254)
        || ip[0] == 0
}

fn parse_ipv6(s: &str) -> Option<[u8; 16]> {
    if s.is_empty() || s.len() > 45 || !s.contains(':') {
        return None;
    }
    let (h, t, comp) = match s.split_once("::") {
        Some((h, t)) if !t.contains("::") => (h, t, true),
        Some(_) => return None,
        None => (s, "", false),
    };
    let mut head = Vec::new();
    if !h.is_empty() {
        for g in h.split(':') {
            if g.is_empty() || g.len() > 4 {
                return None;
            }
            head.push(u16::from_str_radix(g, 16).ok()?);
        }
    }
    let mut tail = Vec::new();
    if !t.is_empty() {
        for g in t.split(':') {
            if g.is_empty() || g.len() > 4 {
                return None;
            }
            tail.push(u16::from_str_radix(g, 16).ok()?);
        }
    }
    if (comp && head.len() + tail.len() >= 8) || (!comp && (head.len() != 8 || !tail.is_empty())) {
        return None;
    }
    let mut words = [0u16; 8];
    words[..head.len()].copy_from_slice(&head);
    words[8 - tail.len()..].copy_from_slice(&tail);
    let mut out = [0u8; 16];
    for (i, w) in words.iter().enumerate() {
        let b = w.to_be_bytes();
        out[i * 2] = b[0];
        out[i * 2 + 1] = b[1];
    }
    Some(out)
}

fn parse_ipv4(host: &str) -> Option<[u8; 4]> {
    let mut parts = host.split('.');
    let mut out = [0u8; 4];
    for slot in &mut out {
        *slot = parts.next()?.parse::<u8>().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

