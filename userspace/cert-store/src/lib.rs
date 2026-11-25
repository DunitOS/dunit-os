//! Versioned read-only X.509 trust store with integrity verification, rotation, and rollback.
#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use core::fmt;
use dunit_tls_core::crypto::{ct_eq, sha256};
use dunit_tls_core::Certificate;

const MAGIC: [u8; 4] = *b"DCRT";
const FORMAT_VERSION: u32 = 1;
const HEADER_LEN: usize = 24;
const MAX_CERTS: usize = 256;
const MAX_CERT_DER: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    Malformed,
    ChecksumMismatch,
    EmptyBundle,
    InvalidCert,
    NotCa,
    RollbackRejected,
    NoRollback,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => f.write_str("malformed certificate store"),
            Self::ChecksumMismatch => f.write_str("certificate store checksum mismatch"),
            Self::EmptyBundle => f.write_str("empty certificate store"),
            Self::InvalidCert => f.write_str("invalid X.509 certificate in store"),
            Self::NotCa => f.write_str("root anchor lacks CA BasicConstraints"),
            Self::RollbackRejected => f.write_str("store version downgrade rejected"),
            Self::NoRollback => f.write_str("no previous certificate store for rollback"),
        }
    }
}

#[derive(Clone)]
pub struct TrustBundle {
    pub version: u32,
    pub generation: u64,
    ders: Vec<Vec<u8>>,
    anchors: Vec<Certificate>,
}

impl TrustBundle {
    pub fn from_ders(version: u32, generation: u64, ders: Vec<Vec<u8>>) -> Result<Self, StoreError> {
        if ders.is_empty() || ders.len() > MAX_CERTS {
            return Err(if ders.is_empty() { StoreError::EmptyBundle } else { StoreError::Malformed });
        }
        let mut anchors = Vec::with_capacity(ders.len());
        for raw in &ders {
            if raw.len() > MAX_CERT_DER {
                return Err(StoreError::Malformed);
            }
            let cert = Certificate::parse(raw).map_err(|_| StoreError::InvalidCert)?;
            if !cert.is_ca {
                return Err(StoreError::NotCa);
            }
            anchors.push(cert);
        }
        Ok(Self {
            version,
            generation,
            ders,
            anchors,
        })
    }

    pub fn from_pem(version: u32, generation: u64, pem: &str) -> Result<Self, StoreError> {
        let ders = parse_pem_certs(pem)?;
        Self::from_ders(version, generation, ders)
    }

    pub fn anchors(&self) -> &[Certificate] {
        &self.anchors
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&self.generation.to_le_bytes());
        out.extend_from_slice(&(self.ders.len() as u32).to_le_bytes());
        for der in &self.ders {
            out.extend_from_slice(&(der.len() as u32).to_le_bytes());
            out.extend_from_slice(der);
        }
        let digest = sha256(&out);
        out.extend_from_slice(&digest);
        out
    }

    pub fn decode_verified(blob: &[u8]) -> Result<Self, StoreError> {
        if blob.len() < HEADER_LEN + 32 || blob[..4] != MAGIC {
            return Err(StoreError::Malformed);
        }
        let (payload, checksum) = blob.split_at(blob.len() - 32);
        if !ct_eq(&sha256(payload), checksum) {
            return Err(StoreError::ChecksumMismatch);
        }
        let fmt_ver = u32::from_le_bytes(payload[4..8].try_into().unwrap());
        if fmt_ver != FORMAT_VERSION {
            return Err(StoreError::Malformed);
        }
        let version = u32::from_le_bytes(payload[8..12].try_into().unwrap());
        let generation = u64::from_le_bytes(payload[12..20].try_into().unwrap());
        let count = usize::try_from(u32::from_le_bytes(payload[20..24].try_into().unwrap()))
            .map_err(|_| StoreError::Malformed)?;
        if count == 0 || count > MAX_CERTS {
            return Err(if count == 0 { StoreError::EmptyBundle } else { StoreError::Malformed });
        }

        let mut pos = HEADER_LEN;
        let mut ders = Vec::with_capacity(count);
        for _ in 0..count {
            if pos + 4 > payload.len() {
                return Err(StoreError::Malformed);
            }
            let clen = usize::try_from(u32::from_le_bytes(payload[pos..pos + 4].try_into().unwrap()))
                .map_err(|_| StoreError::Malformed)?;
            pos += 4;
            if clen == 0 || clen > MAX_CERT_DER || pos + clen > payload.len() {
                return Err(StoreError::Malformed);
            }
            ders.push(payload[pos..pos + clen].to_vec());
            pos += clen;
        }
        if pos != payload.len() {
            return Err(StoreError::Malformed);
        }
        Self::from_ders(version, generation, ders)
    }
}

pub struct ManagedStore {
    active: TrustBundle,
    previous: Option<TrustBundle>,
}

impl ManagedStore {
    pub fn from_blob(blob: &[u8]) -> Result<Self, StoreError> {
        Ok(Self {
            active: TrustBundle::decode_verified(blob)?,
            previous: None,
        })
    }

    pub fn from_bundle(bundle: TrustBundle) -> Self {
        Self {
            active: bundle,
            previous: None,
        }
    }

    pub fn version(&self) -> u32 {
        self.active.version
    }

    pub fn generation(&self) -> u64 {
        self.active.generation
    }

    pub fn anchors(&self) -> &[Certificate] {
        self.active.anchors()
    }

    pub fn active_blob(&self) -> Vec<u8> {
        self.active.encode()
    }

    pub fn rotate(&mut self, new_blob: &[u8]) -> Result<u32, StoreError> {
        let candidate = TrustBundle::decode_verified(new_blob)?;
        if candidate.version <= self.active.version && candidate.generation <= self.active.generation {
            return Err(StoreError::RollbackRejected);
        }
        let next_ver = candidate.version;
        let prev = core::mem::replace(&mut self.active, candidate);
        self.previous = Some(prev);
        Ok(next_ver)
    }

    pub fn rollback(&mut self) -> Result<u32, StoreError> {
        let Some(prev) = self.previous.take() else {
            return Err(StoreError::NoRollback);
        };
        let restored = prev.version;
        self.active = prev;
        Ok(restored)
    }
}

pub fn parse_pem_certs(pem: &str) -> Result<Vec<Vec<u8>>, StoreError> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let mut certs = Vec::new();
    let mut rest = pem;
    while let Some(start_idx) = rest.find(BEGIN) {
        let after_begin = &rest[start_idx + BEGIN.len()..];
        let end_idx = after_begin.find(END).ok_or(StoreError::Malformed)?;
        let body = &after_begin[..end_idx];
        certs.push(decode_base64(body)?);
        rest = &after_begin[end_idx + END.len()..];
    }
    if certs.is_empty() {
        return Err(StoreError::EmptyBundle);
    }
    Ok(certs)
}

fn decode_base64(text: &str) -> Result<Vec<u8>, StoreError> {
    let mut clean = Vec::new();
    for b in text.bytes() {
        if !b.is_ascii_whitespace() {
            clean.push(b);
        }
    }
    if clean.is_empty() || clean.len() % 4 != 0 {
        return Err(StoreError::Malformed);
    }
    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    for (idx, chunk) in clean.chunks_exact(4).enumerate() {
        let is_last = (idx + 1) * 4 == clean.len();
        let v0 = b64_val(chunk[0])?;
        let v1 = b64_val(chunk[1])?;
        let pad2 = chunk[2] == b'=';
        let pad3 = chunk[3] == b'=';
        if (pad2 || pad3) && !is_last {
            return Err(StoreError::Malformed);
        }
        if pad2 && !pad3 {
            return Err(StoreError::Malformed);
        }
        let v2 = if pad2 { 0 } else { b64_val(chunk[2])? };
        let v3 = if pad3 { 0 } else { b64_val(chunk[3])? };
        let word = (u32::from(v0) << 18) | (u32::from(v1) << 12) | (u32::from(v2) << 6) | u32::from(v3);
        out.push((word >> 16) as u8);
        if !pad2 {
            out.push((word >> 8) as u8);
        }
        if !pad3 {
            out.push(word as u8);
        }
    }
    Ok(out)
}

fn b64_val(b: u8) -> Result<u8, StoreError> {
    match b {
        b'A'..=b'Z' => Ok(b - b'A'),
        b'a'..=b'z' => Ok(b - b'a' + 26),
        b'0'..=b'9' => Ok(b - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(StoreError::Malformed),
    }
}

#[cfg(test)]
mod tests;
