//! Kernel IPC + SharedFrames wire protocol (`DBNT` v1) between sandboxed clients and `browser_netd`.
use alloc::{string::String, vec::Vec};
use dunit_http_core::{Header, Method};

use crate::{
    cache::{decode_headers, encode_headers},
    policy::ResourceIntent,
};

pub const MAGIC: [u8; 4] = *b"DBNT";
pub const VERSION: u16 = 1;
pub const CONTROL_SIZE: usize = 128;
pub const SHM_HANDLE_OFFSET: usize = 20;

pub const OP_PING: u16 = 1;
pub const OP_FETCH_REQ: u16 = 2;
pub const OP_FETCH_RESP: u16 = 3;
pub const OP_STATS_REQ: u16 = 4;
pub const OP_STATS_RESP: u16 = 5;
pub const OP_TRIGGER_FAULT: u16 = 6;

pub const FLAG_BYPASS_CACHE: u32 = 1 << 0;
pub const FLAG_CACHE_HIT: u32 = 1 << 1;
pub const FLAG_CACHE_REPAIRED: u32 = 1 << 2;
pub const FLAG_IS_DOWNLOAD: u32 = 1 << 3;
pub const FLAG_TLS_VERIFIED: u32 = 1 << 4;
pub const FLAG_VIA_PROXY: u32 = 1 << 5;

pub const ERR_OK: i32 = 0;
pub const ERR_INVALID: i32 = -1;
pub const ERR_POLICY: i32 = -2;
pub const ERR_TLS_EXPIRED: i32 = -3;
pub const ERR_TLS_INVALID_CERT: i32 = -4;
pub const ERR_TLS_UNTRUSTED: i32 = -5;
pub const ERR_TLS_HANDSHAKE: i32 = -6;
pub const ERR_REDIRECT_LOOP: i32 = -7;
pub const ERR_TRUNCATED: i32 = -8;
pub const ERR_DECOMPRESS_BOMB: i32 = -9;
pub const ERR_QUOTA: i32 = -10;
pub const ERR_TRANSPORT: i32 = -11;
pub const ERR_FORBIDDEN: i32 = -12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlMessage {
    pub op: u16,
    pub request_id: u64,
    pub status_or_error: i32,
    pub shm_handle: u32,
    pub payload_len: u32,
    pub flags: u32,
    pub service_pid: u32,
    pub generation: u32,
    pub cache_hits: u16,
    pub cache_repaired: u16,
    pub cookie_count: u16,
    pub restarts: u16,
    pub summary: String,
}

impl ControlMessage {
    pub fn empty(op: u16, request_id: u64) -> Self {
        Self {
            op,
            request_id,
            status_or_error: 0,
            shm_handle: 0,
            payload_len: 0,
            flags: 0,
            service_pid: 0,
            generation: 0,
            cache_hits: 0,
            cache_repaired: 0,
            cookie_count: 0,
            restarts: 0,
            summary: String::new(),
        }
    }

    pub fn encode(&self) -> [u8; CONTROL_SIZE] {
        let mut buf = [0u8; CONTROL_SIZE];
        buf[0..4].copy_from_slice(&MAGIC);
        buf[4..6].copy_from_slice(&VERSION.to_le_bytes());
        buf[6..8].copy_from_slice(&self.op.to_le_bytes());
        buf[8..16].copy_from_slice(&self.request_id.to_le_bytes());
        buf[16..20].copy_from_slice(&self.status_or_error.to_le_bytes());
        buf[20..24].copy_from_slice(&self.shm_handle.to_le_bytes());
        buf[24..28].copy_from_slice(&self.payload_len.to_le_bytes());
        buf[28..32].copy_from_slice(&self.flags.to_le_bytes());
        buf[32..36].copy_from_slice(&self.service_pid.to_le_bytes());
        buf[36..40].copy_from_slice(&self.generation.to_le_bytes());
        buf[40..42].copy_from_slice(&self.cache_hits.to_le_bytes());
        buf[42..44].copy_from_slice(&self.cache_repaired.to_le_bytes());
        buf[44..46].copy_from_slice(&self.cookie_count.to_le_bytes());
        buf[46..48].copy_from_slice(&self.restarts.to_le_bytes());
        let raw = self.summary.as_bytes();
        let n = raw.len().min(CONTROL_SIZE - 50);
        buf[48..50].copy_from_slice(&(n as u16).to_le_bytes());
        buf[50..50 + n].copy_from_slice(&raw[..n]);
        buf
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ()> {
        if bytes.len() != CONTROL_SIZE || bytes[0..4] != MAGIC {
            return Err(());
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != VERSION {
            return Err(());
        }
        let op = u16::from_le_bytes([bytes[6], bytes[7]]);
        let request_id = u64::from_le_bytes(bytes[8..16].try_into().map_err(|_| ())?);
        let status_or_error = i32::from_le_bytes(bytes[16..20].try_into().map_err(|_| ())?);
        let shm_handle = u32::from_le_bytes(bytes[20..24].try_into().map_err(|_| ())?);
        let payload_len = u32::from_le_bytes(bytes[24..28].try_into().map_err(|_| ())?);
        let flags = u32::from_le_bytes(bytes[28..32].try_into().map_err(|_| ())?);
        let service_pid = u32::from_le_bytes(bytes[32..36].try_into().map_err(|_| ())?);
        let generation = u32::from_le_bytes(bytes[36..40].try_into().map_err(|_| ())?);
        let cache_hits = u16::from_le_bytes([bytes[40], bytes[41]]);
        let cache_repaired = u16::from_le_bytes([bytes[42], bytes[43]]);
        let cookie_count = u16::from_le_bytes([bytes[44], bytes[45]]);
        let restarts = u16::from_le_bytes([bytes[46], bytes[47]]);
        let slen = u16::from_le_bytes([bytes[48], bytes[49]]) as usize;
        if 50 + slen > CONTROL_SIZE {
            return Err(());
        }
        let summary = String::from_utf8(bytes[50..50 + slen].to_vec()).map_err(|_| ())?;
        Ok(Self {
            op,
            request_id,
            status_or_error,
            shm_handle,
            payload_len,
            flags,
            service_pid,
            generation,
            cache_hits,
            cache_repaired,
            cookie_count,
            restarts,
            summary,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchRequest {
    pub method: Method,
    pub intent: ResourceIntent,
    pub bypass_cache: bool,
    pub url: String,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
}

impl FetchRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"BREQ");
        out.push(method_to_u8(self.method));
        out.push(self.intent.as_u8());
        out.push(self.bypass_cache as u8);
        out.push(0);
        push_vec(&mut out, self.url.as_bytes());
        let hdrs = encode_headers(&self.headers);
        push_vec(&mut out, &hdrs);
        push_vec(&mut out, &self.body);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ()> {
        if bytes.len() < 8 || &bytes[0..4] != b"BREQ" {
            return Err(());
        }
        let method = u8_to_method(bytes[4]).ok_or(())?;
        let intent = ResourceIntent::from_u8(bytes[5]).ok_or(())?;
        let bypass_cache = bytes[6] != 0;
        let mut pos = 8usize;
        let url_bytes = read_vec(bytes, &mut pos)?;
        let url = String::from_utf8(url_bytes.to_vec()).map_err(|_| ())?;
        let hdrs_bytes = read_vec(bytes, &mut pos)?;
        let headers = decode_headers(hdrs_bytes)?;
        let body = read_vec(bytes, &mut pos)?.to_vec();
        if pos != bytes.len() {
            return Err(());
        }
        Ok(Self {
            method,
            intent,
            bypass_cache,
            url,
            headers,
            body,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchResponsePayload {
    pub status: u16,
    pub final_url: String,
    pub content_type: String,
    pub download_filename: Option<String>,
    pub tls_info: String,
    pub detail: String,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
}

impl FetchResponsePayload {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"BRSP");
        out.extend_from_slice(&self.status.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        push_vec(&mut out, self.final_url.as_bytes());
        push_vec(&mut out, self.content_type.as_bytes());
        push_vec(&mut out, self.download_filename.as_deref().unwrap_or("").as_bytes());
        push_vec(&mut out, self.tls_info.as_bytes());
        push_vec(&mut out, self.detail.as_bytes());
        let hdrs = encode_headers(&self.headers);
        push_vec(&mut out, &hdrs);
        push_vec(&mut out, &self.body);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ()> {
        if bytes.len() < 8 || &bytes[0..4] != b"BRSP" {
            return Err(());
        }
        let status = u16::from_le_bytes([bytes[4], bytes[5]]);
        let mut pos = 8usize;
        let final_url = String::from_utf8(read_vec(bytes, &mut pos)?.to_vec()).map_err(|_| ())?;
        let content_type = String::from_utf8(read_vec(bytes, &mut pos)?.to_vec()).map_err(|_| ())?;
        let dl_raw = read_vec(bytes, &mut pos)?;
        let download_filename = if dl_raw.is_empty() {
            None
        } else {
            Some(String::from_utf8(dl_raw.to_vec()).map_err(|_| ())?)
        };
        let tls_info = String::from_utf8(read_vec(bytes, &mut pos)?.to_vec()).map_err(|_| ())?;
        let detail = String::from_utf8(read_vec(bytes, &mut pos)?.to_vec()).map_err(|_| ())?;
        let headers = decode_headers(read_vec(bytes, &mut pos)?)?;
        let body = read_vec(bytes, &mut pos)?.to_vec();
        if pos != bytes.len() {
            return Err(());
        }
        Ok(Self {
            status,
            final_url,
            content_type,
            download_filename,
            tls_info,
            detail,
            headers,
            body,
        })
    }
}

fn push_vec(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
}

fn read_vec<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8], ()> {
    if *pos + 4 > bytes.len() {
        return Err(());
    }
    let len = u32::from_le_bytes(bytes[*pos..*pos + 4].try_into().map_err(|_| ())?) as usize;
    *pos += 4;
    if *pos + len > bytes.len() {
        return Err(());
    }
    let slice = &bytes[*pos..*pos + len];
    *pos += len;
    Ok(slice)
}

const fn method_to_u8(m: Method) -> u8 {
    match m {
        Method::Get => 0,
        Method::Head => 1,
        Method::Post => 2,
        Method::Put => 3,
        Method::Delete => 4,
    }
}

const fn u8_to_method(v: u8) -> Option<Method> {
    match v {
        0 => Some(Method::Get),
        1 => Some(Method::Head),
        2 => Some(Method::Post),
        3 => Some(Method::Put),
        4 => Some(Method::Delete),
        _ => None,
    }
}

