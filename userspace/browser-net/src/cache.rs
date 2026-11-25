//! Bounded, checksum-verified, crash-safe HTTP response cache (`DNWC` v1).
use alloc::{
    format,
    string::String,
    vec::Vec,
};
use dunit_http_core::{Header, Method, Response, Url};
use dunit_tls_core::crypto::Sha256;

const MAGIC: &[u8; 4] = b"DNWC";
const VERSION: u16 = 1;
const HEADER_LEN: usize = 60;

pub trait CacheStorage {
    fn list_files(&self) -> Vec<String>;
    fn read_file(&self, name: &str) -> Option<Vec<u8>>;
    fn write_atomic(&mut self, name: &str, data: &[u8]) -> bool;
    fn remove_file(&mut self, name: &str);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedEntry {
    pub key: String,
    pub stored_at: u64,
    pub max_age: u64,
    pub status: u16,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u32,
    pub misses: u32,
    pub stores: u32,
    pub evictions: u32,
    pub corrupt_repaired: u32,
    pub active_entries: usize,
    pub total_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheLookup {
    Hit(CachedEntry),
    Miss,
    MissCorrupt,
}

pub struct BoundedCache<S: CacheStorage> {
    storage: S,
    max_entries: usize,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    stats: CacheStats,
}

impl<S: CacheStorage> BoundedCache<S> {
    pub fn open(storage: S, max_entries: usize, max_entry_bytes: usize, max_total_bytes: usize) -> Self {
        let mut cache = Self {
            storage,
            max_entries: max_entries.max(1),
            max_entry_bytes: max_entry_bytes.max(64),
            max_total_bytes: max_total_bytes.max(128),
            stats: CacheStats::default(),
        };
        cache.recompute_index();
        cache
    }

    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    pub fn cache_key(method: Method, url: &Url) -> String {
        format!("{} {}://{}:{}{}", method.as_str(), url.scheme.as_str(), url.host.to_ascii_lowercase(), url.port, url.path)
    }

    pub fn slot_name(key: &str) -> String {
        let digest = sha256_bytes(key.as_bytes());
        let mut hex = String::with_capacity(21);
        for b in &digest[..8] {
            let hi = (b >> 4) & 0x0f;
            let lo = b & 0x0f;
            hex.push(hex_digit(hi));
            hex.push(hex_digit(lo));
        }
        hex.push_str(".dnwc");
        hex
    }

    pub fn lookup(&mut self, method: Method, url: &Url, now_unix: u64) -> CacheLookup {
        if method != Method::Get {
            self.stats.misses += 1;
            return CacheLookup::Miss;
        }
        let key = Self::cache_key(method, url);
        let slot = Self::slot_name(&key);
        let Some(blob) = self.storage.read_file(&slot) else {
            self.stats.misses += 1;
            return CacheLookup::Miss;
        };
        match decode_entry(&blob) {
            Ok(entry) if entry.key == key => {
                let expires = entry.stored_at.saturating_add(entry.max_age);
                if entry.max_age > 0 && now_unix <= expires {
                    self.stats.hits += 1;
                    CacheLookup::Hit(entry)
                } else {
                    self.storage.remove_file(&slot);
                    self.recompute_index();
                    self.stats.misses += 1;
                    CacheLookup::Miss
                }
            }
            _ => {
                self.storage.remove_file(&slot);
                self.stats.corrupt_repaired += 1;
                self.stats.misses += 1;
                self.recompute_index();
                CacheLookup::MissCorrupt
            }
        }
    }

    pub fn store(&mut self, method: Method, url: &Url, resp: &Response, now_unix: u64) -> bool {
        if method != Method::Get || resp.status != 200 {
            return false;
        }
        let max_age = match parse_cache_control(&resp.headers) {
            Some(secs) if secs > 0 => secs,
            _ => return false,
        };
        let key = Self::cache_key(method, url);
        let entry = CachedEntry {
            key: key.clone(),
            stored_at: now_unix,
            max_age,
            status: resp.status,
            headers: resp.headers.clone(),
            body: resp.body.clone(),
        };
        let encoded = encode_entry(&entry);
        if encoded.len() > self.max_entry_bytes || encoded.len() > self.max_total_bytes {
            return false;
        }
        let target_slot = Self::slot_name(&key);
        self.evict_for(encoded.len(), &target_slot);
        if self.storage.write_atomic(&target_slot, &encoded) {
            self.stats.stores += 1;
            self.recompute_index();
            true
        } else {
            false
        }
    }

    pub fn purge_corrupt(&mut self) -> u32 {
        let mut repaired = 0u32;
        for name in self.storage.list_files() {
            if !name.ends_with(".dnwc") {
                continue;
            }
            let valid = self
                .storage
                .read_file(&name)
                .is_some_and(|blob| decode_entry(&blob).is_ok());
            if !valid {
                self.storage.remove_file(&name);
                repaired += 1;
            }
        }
        self.stats.corrupt_repaired += repaired;
        self.recompute_index();
        repaired
    }

    fn evict_for(&mut self, incoming_len: usize, replace_slot: &str) {
        let mut items: Vec<(String, u64, usize)> = Vec::new();
        for name in self.storage.list_files() {
            if !name.ends_with(".dnwc") || name == replace_slot {
                continue;
            }
            match self.storage.read_file(&name) {
                Some(blob) => match decode_entry(&blob) {
                    Ok(entry) => items.push((name, entry.stored_at, blob.len())),
                    Err(()) => {
                        self.storage.remove_file(&name);
                        self.stats.corrupt_repaired += 1;
                    }
                },
                None => {}
            }
        }
        items.sort_by_key(|item| item.1);
        let mut total: usize = items.iter().map(|i| i.2).sum();
        let mut count = items.len();
        let mut idx = 0usize;
        while idx < items.len() && (count >= self.max_entries || total.saturating_add(incoming_len) > self.max_total_bytes) {
            self.storage.remove_file(&items[idx].0);
            total = total.saturating_sub(items[idx].2);
            count -= 1;
            idx += 1;
            self.stats.evictions += 1;
        }
    }

    fn recompute_index(&mut self) {
        let mut count = 0usize;
        let mut total = 0usize;
        for name in self.storage.list_files() {
            if !name.ends_with(".dnwc") {
                continue;
            }
            if let Some(blob) = self.storage.read_file(&name) {
                if decode_entry(&blob).is_ok() {
                    count += 1;
                    total = total.saturating_add(blob.len());
                }
            }
        }
        self.stats.active_entries = count;
        self.stats.total_bytes = total;
    }
}

pub fn parse_cache_control(headers: &[Header]) -> Option<u64> {
    let raw = headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("cache-control"))?
        .value
        .as_str();
    let mut max_age = None;
    for directive in raw.split(',') {
        let d = directive.trim().to_ascii_lowercase();
        if d == "no-store" || d == "no-cache" {
            return None;
        }
        if let Some(secs) = d.strip_prefix("max-age=") {
            max_age = secs.trim().parse::<u64>().ok();
        }
    }
    max_age
}

pub fn encode_entry(entry: &CachedEntry) -> Vec<u8> {
    let hdr_blob = encode_headers(&entry.headers);
    let key_bytes = entry.key.as_bytes();
    let mut out = Vec::with_capacity(HEADER_LEN + key_bytes.len() + hdr_blob.len() + entry.body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&entry.status.to_le_bytes());
    out.extend_from_slice(&entry.stored_at.to_le_bytes());
    out.extend_from_slice(&entry.max_age.to_le_bytes());
    out.extend_from_slice(&(key_bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(hdr_blob.len() as u32).to_le_bytes());
    out.extend_from_slice(&(entry.body.len() as u32).to_le_bytes());
    let mut hasher = Sha256::new();
    hasher.update(&out[..36]);
    hasher.update(key_bytes);
    hasher.update(&hdr_blob);
    hasher.update(&entry.body);
    let digest = hasher.finalize();
    out.extend_from_slice(&digest[..24]);
    out.extend_from_slice(key_bytes);
    out.extend_from_slice(&hdr_blob);
    out.extend_from_slice(&entry.body);
    out
}

pub fn decode_entry(blob: &[u8]) -> Result<CachedEntry, ()> {
    if blob.len() < HEADER_LEN || &blob[..4] != MAGIC {
        return Err(());
    }
    let version = u16::from_le_bytes([blob[4], blob[5]]);
    if version != VERSION {
        return Err(());
    }
    let status = u16::from_le_bytes([blob[6], blob[7]]);
    let stored_at = u64::from_le_bytes(blob[8..16].try_into().map_err(|_| ())?);
    let max_age = u64::from_le_bytes(blob[16..24].try_into().map_err(|_| ())?);
    let key_len = u16::from_le_bytes([blob[24], blob[25]]) as usize;
    let hdr_len = u32::from_le_bytes(blob[28..32].try_into().map_err(|_| ())?) as usize;
    let body_len = u32::from_le_bytes(blob[32..36].try_into().map_err(|_| ())?) as usize;
    let expected_len = HEADER_LEN
        .checked_add(key_len)
        .and_then(|v| v.checked_add(hdr_len))
        .and_then(|v| v.checked_add(body_len))
        .ok_or(())?;
    if blob.len() != expected_len {
        return Err(());
    }
    let mut hasher = Sha256::new();
    hasher.update(&blob[..36]);
    hasher.update(&blob[HEADER_LEN..]);
    let digest = hasher.finalize();
    if digest[..24] != blob[36..60] {
        return Err(());
    }
    let key_end = HEADER_LEN + key_len;
    let hdr_end = key_end + hdr_len;
    let key = String::from_utf8(blob[HEADER_LEN..key_end].to_vec()).map_err(|_| ())?;
    let headers = decode_headers(&blob[key_end..hdr_end])?;
    let body = blob[hdr_end..].to_vec();
    Ok(CachedEntry {
        key,
        stored_at,
        max_age,
        status,
        headers,
        body,
    })
}

pub(crate) fn encode_headers(headers: &[Header]) -> Vec<u8> {
    let mut out = Vec::new();
    let count = headers.len().min(u16::MAX as usize) as u16;
    out.extend_from_slice(&count.to_le_bytes());
    for h in headers.iter().take(count as usize) {
        let nb = h.name.as_bytes();
        let vb = h.value.as_bytes();
        out.extend_from_slice(&(nb.len().min(u16::MAX as usize) as u16).to_le_bytes());
        out.extend_from_slice(&(vb.len().min(u16::MAX as usize) as u16).to_le_bytes());
        out.extend_from_slice(&nb[..nb.len().min(u16::MAX as usize)]);
        out.extend_from_slice(&vb[..vb.len().min(u16::MAX as usize)]);
    }
    out
}

pub(crate) fn decode_headers(bytes: &[u8]) -> Result<Vec<Header>, ()> {
    if bytes.len() < 2 {
        return Err(());
    }
    let count = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
    let mut pos = 2usize;
    let mut out = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        if pos + 4 > bytes.len() {
            return Err(());
        }
        let nlen = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        let vlen = u16::from_le_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        pos += 4;
        if pos + nlen + vlen > bytes.len() {
            return Err(());
        }
        let name = String::from_utf8(bytes[pos..pos + nlen].to_vec()).map_err(|_| ())?;
        pos += nlen;
        let value = String::from_utf8(bytes[pos..pos + vlen].to_vec()).map_err(|_| ())?;
        pos += vlen;
        out.push(Header { name, value });
    }
    if pos != bytes.len() {
        return Err(());
    }
    Ok(out)
}

fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize()
}

const fn hex_digit(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        10..=15 => (b'a' + (v - 10)) as char,
        _ => '0',
    }
}
