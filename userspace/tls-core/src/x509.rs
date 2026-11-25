//! Strict ASN.1 DER and X.509 v3 certificate chain verifier.
use crate::{crypto::sha256, ecdsa_p256, rsa, TlsError};
use alloc::{string::String, vec::Vec};

const OID_RSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
const OID_ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const OID_RSA_ENC: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const OID_EC_PUBKEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];

const OID_EXT_SAN: &[u8] = &[0x55, 0x1d, 0x11];
const OID_EXT_BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13];
const OID_EXT_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
const OID_EXT_EKU: &[u8] = &[0x55, 0x1d, 0x25];
const OID_EKU_SERVER_AUTH: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SigAlg {
    RsaSha256,
    EcdsaP256Sha256,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicKey {
    Rsa { n: Vec<u8>, e: Vec<u8> },
    EcdsaP256(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct Certificate {
    pub raw_tbs: Vec<u8>,
    pub sig_alg: SigAlg,
    pub sig_value: Vec<u8>,
    pub issuer_der: Vec<u8>,
    pub subject_der: Vec<u8>,
    pub not_before: u64,
    pub not_after: u64,
    pub public_key: PublicKey,
    pub is_ca: bool,
    pub path_len: Option<u32>,
    pub key_cert_sign: Option<bool>,
    pub server_auth: Option<bool>,
    pub san_dns: Vec<String>,
    pub san_ips: Vec<[u8; 4]>,
    pub san_ips_v6: Vec<[u8; 16]>,
}

struct DerReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> DerReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn read_tlv_raw(&mut self) -> Result<(u8, &'a [u8], &'a [u8]), TlsError> {
        let start = self.pos;
        if self.pos >= self.data.len() {
            return Err(TlsError::InvalidCertificate);
        }
        let tag = self.data[self.pos];
        self.pos += 1;
        if (tag & 0x1f) == 0x1f {
            return Err(TlsError::InvalidCertificate);
        }
        if self.pos >= self.data.len() {
            return Err(TlsError::InvalidCertificate);
        }
        let first_len = self.data[self.pos];
        self.pos += 1;
        let len = if first_len < 0x80 {
            usize::from(first_len)
        } else {
            let nbytes = usize::from(first_len & 0x7f);
            if nbytes == 0 || nbytes > 4 || self.pos + nbytes > self.data.len() {
                return Err(TlsError::InvalidCertificate);
            }
            let mut v = 0usize;
            for i in 0..nbytes {
                v = (v << 8) | usize::from(self.data[self.pos + i]);
            }
            if v < 0x80 || (nbytes > 1 && self.data[self.pos] == 0) {
                return Err(TlsError::InvalidCertificate);
            }
            self.pos += nbytes;
            v
        };
        if self.pos.checked_add(len).is_none_or(|e| e > self.data.len()) {
            return Err(TlsError::InvalidCertificate);
        }
        let val = &self.data[self.pos..self.pos + len];
        self.pos += len;
        let full = &self.data[start..self.pos];
        Ok((tag, val, full))
    }

    fn read_tag(&mut self, expected: u8) -> Result<&'a [u8], TlsError> {
        let (tag, val, _) = self.read_tlv_raw()?;
        if tag != expected {
            return Err(TlsError::InvalidCertificate);
        }
        Ok(val)
    }

    fn read_tag_full(&mut self, expected: u8) -> Result<(&'a [u8], &'a [u8]), TlsError> {
        let (tag, val, full) = self.read_tlv_raw()?;
        if tag != expected {
            return Err(TlsError::InvalidCertificate);
        }
        Ok((val, full))
    }
}

fn parse_sig_alg(der: &[u8]) -> Result<SigAlg, TlsError> {
    let mut r = DerReader::new(der);
    let oid = r.read_tag(0x06)?;
    if oid == OID_RSA_SHA256 {
        Ok(SigAlg::RsaSha256)
    } else if oid == OID_ECDSA_SHA256 {
        Ok(SigAlg::EcdsaP256Sha256)
    } else {
        Err(TlsError::UnsupportedAlgorithm)
    }
}

fn parse_spki(der: &[u8]) -> Result<PublicKey, TlsError> {
    let mut r = DerReader::new(der);
    let alg_der = r.read_tag(0x30)?;
    let bits = r.read_tag(0x03)?;
    if !r.is_empty() || bits.is_empty() || bits[0] != 0 {
        return Err(TlsError::InvalidCertificate);
    }
    let pk_bytes = &bits[1..];

    let mut ar = DerReader::new(alg_der);
    let alg_oid = ar.read_tag(0x06)?;
    if alg_oid == OID_RSA_ENC {
        let mut kr = DerReader::new(pk_bytes);
        let seq = kr.read_tag(0x30)?;
        if !kr.is_empty() {
            return Err(TlsError::InvalidCertificate);
        }
        let mut sr = DerReader::new(seq);
        let n = strip_uint_pad(sr.read_tag(0x02)?)?;
        let e = strip_uint_pad(sr.read_tag(0x02)?)?;
        if !sr.is_empty() {
            return Err(TlsError::InvalidCertificate);
        }
        Ok(PublicKey::Rsa {
            n: n.to_vec(),
            e: e.to_vec(),
        })
    } else if alg_oid == OID_EC_PUBKEY {
        let curve_oid = ar.read_tag(0x06)?;
        if curve_oid != OID_P256 || pk_bytes.len() != 65 || pk_bytes[0] != 0x04 {
            return Err(TlsError::UnsupportedAlgorithm);
        }
        Ok(PublicKey::EcdsaP256(pk_bytes.to_vec()))
    } else {
        Err(TlsError::UnsupportedAlgorithm)
    }
}

fn strip_uint_pad(bytes: &[u8]) -> Result<&[u8], TlsError> {
    if bytes.is_empty() {
        return Err(TlsError::InvalidCertificate);
    }
    if bytes[0] == 0 {
        Ok(&bytes[1..])
    } else {
        Ok(bytes)
    }
}

fn parse_time(reader: &mut DerReader<'_>) -> Result<u64, TlsError> {
    let (tag, raw, _) = reader.read_tlv_raw()?;
    let s = core::str::from_utf8(raw).map_err(|_| TlsError::InvalidCertificate)?;
    let (year, rest) = match tag {
        0x17 => {
            if s.len() != 13 || !s.ends_with('Z') {
                return Err(TlsError::InvalidCertificate);
            }
            let yy: u64 = s[..2].parse().map_err(|_| TlsError::InvalidCertificate)?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, &s[2..12])
        }
        0x18 => {
            if s.len() != 15 || !s.ends_with('Z') {
                return Err(TlsError::InvalidCertificate);
            }
            let yyyy: u64 = s[..4].parse().map_err(|_| TlsError::InvalidCertificate)?;
            (yyyy, &s[4..14])
        }
        _ => return Err(TlsError::InvalidCertificate),
    };
    let month: u64 = rest[0..2].parse().map_err(|_| TlsError::InvalidCertificate)?;
    let day: u64 = rest[2..4].parse().map_err(|_| TlsError::InvalidCertificate)?;
    let hour: u64 = rest[4..6].parse().map_err(|_| TlsError::InvalidCertificate)?;
    let min: u64 = rest[6..8].parse().map_err(|_| TlsError::InvalidCertificate)?;
    let sec: u64 = rest[8..10].parse().map_err(|_| TlsError::InvalidCertificate)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 59 {
        return Err(TlsError::InvalidCertificate);
    }
    civil_to_unix(year, month, day, hour, min, sec)
}

fn civil_to_unix(y: u64, m: u64, d: u64, hh: u64, mm: u64, ss: u64) -> Result<u64, TlsError> {
    if y < 1970 {
        return Ok(0);
    }
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m_adj = m as i64 + if m > 2 { -3 } else { 9 };
    let doy = (153 * m_adj + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    if days < 0 {
        return Ok(0);
    }
    Ok((days as u64) * 86_400 + hh * 3600 + mm * 60 + ss)
}

pub fn parse_ecdsa_sig_der(sig_der: &[u8]) -> Result<(&[u8], &[u8]), TlsError> {
    let mut r = DerReader::new(sig_der);
    let seq = r.read_tag(0x30)?;
    if !r.is_empty() {
        return Err(TlsError::BadSignature);
    }
    let mut sr = DerReader::new(seq);
    let rv = strip_uint_pad(sr.read_tag(0x02).map_err(|_| TlsError::BadSignature)?)
        .map_err(|_| TlsError::BadSignature)?;
    let sv = strip_uint_pad(sr.read_tag(0x02).map_err(|_| TlsError::BadSignature)?)
        .map_err(|_| TlsError::BadSignature)?;
    if !sr.is_empty() || rv.len() > 32 || sv.len() > 32 {
        return Err(TlsError::BadSignature);
    }
    Ok((rv, sv))
}

impl Certificate {
    pub fn parse(der: &[u8]) -> Result<Self, TlsError> {
        let mut root = DerReader::new(der);
        let top = root.read_tag(0x30)?;
        if !root.is_empty() {
            return Err(TlsError::InvalidCertificate);
        }
        let mut cert_r = DerReader::new(top);
        let (tbs_val, tbs_full) = cert_r.read_tag_full(0x30)?;
        let outer_alg = parse_sig_alg(cert_r.read_tag(0x30)?)?;
        let sig_bits = cert_r.read_tag(0x03)?;
        if !cert_r.is_empty() || sig_bits.is_empty() || sig_bits[0] != 0 {
            return Err(TlsError::InvalidCertificate);
        }

        let mut tbs = DerReader::new(tbs_val);
        if tbs.peek() == Some(0xa0) {
            let v_inner = tbs.read_tag(0xa0)?;
            let mut vr = DerReader::new(v_inner);
            let _ver = vr.read_tag(0x02)?;
        }
        let _serial = tbs.read_tag(0x02)?;
        let inner_alg = parse_sig_alg(tbs.read_tag(0x30)?)?;
        if inner_alg != outer_alg {
            return Err(TlsError::InvalidCertificate);
        }
        let (_, issuer_full) = tbs.read_tag_full(0x30)?;
        let validity = tbs.read_tag(0x30)?;
        let mut val_r = DerReader::new(validity);
        let not_before = parse_time(&mut val_r)?;
        let not_after = parse_time(&mut val_r)?;
        if !val_r.is_empty() || not_before > not_after {
            return Err(TlsError::InvalidCertificate);
        }
        let (_, subject_full) = tbs.read_tag_full(0x30)?;
        let spki_der = tbs.read_tag(0x30)?;
        let public_key = parse_spki(spki_der)?;

        let mut is_ca = false;
        let mut path_len = None;
        let mut key_cert_sign = None;
        let mut server_auth = None;
        let mut san_dns = Vec::new();
        let mut san_ips = Vec::new();
        let mut san_ips_v6 = Vec::new();

        while !tbs.is_empty() {
            let (tag, inner, _) = tbs.read_tlv_raw()?;
            if tag == 0xa3 {
                let mut er = DerReader::new(inner);
                let exts_seq = er.read_tag(0x30)?;
                let mut list = DerReader::new(exts_seq);
                while !list.is_empty() {
                    let ext_seq = list.read_tag(0x30)?;
                    let mut x = DerReader::new(ext_seq);
                    let oid = x.read_tag(0x06)?;
                    if x.peek() == Some(0x01) {
                        let _crit = x.read_tag(0x01)?;
                    }
                    let oct = x.read_tag(0x04)?;
                    if oid == OID_EXT_BASIC_CONSTRAINTS {
                        let mut br = DerReader::new(oct);
                        let bseq = br.read_tag(0x30)?;
                        let mut b = DerReader::new(bseq);
                        if b.peek() == Some(0x01) {
                            let val = b.read_tag(0x01)?;
                            is_ca = val.len() == 1 && val[0] != 0;
                        }
                        if !b.is_empty() && b.peek() == Some(0x02) {
                            let pl = strip_uint_pad(b.read_tag(0x02)?)?;
                            if pl.len() <= 4 {
                                let mut v = 0u32;
                                for &byte in pl {
                                    v = (v << 8) | u32::from(byte);
                                }
                                path_len = Some(v);
                            }
                        }
                    } else if oid == OID_EXT_KEY_USAGE {
                        let mut kr = DerReader::new(oct);
                        let bs = kr.read_tag(0x03)?;
                        if bs.len() >= 2 {
                            // Bit 5 (MSB-first within byte 1): 0x04 in big-endian bitmask.
                            key_cert_sign = Some((bs[1] & 0x04) != 0);
                        }
                    } else if oid == OID_EXT_EKU {
                        let mut ekr = DerReader::new(oct);
                        let seq = ekr.read_tag(0x30)?;
                        let mut oids = DerReader::new(seq);
                        let mut has_server = false;
                        while !oids.is_empty() {
                            if oids.read_tag(0x06)? == OID_EKU_SERVER_AUTH {
                                has_server = true;
                            }
                        }
                        server_auth = Some(has_server);
                    } else if oid == OID_EXT_SAN {
                        let mut sr = DerReader::new(oct);
                        let seq = sr.read_tag(0x30)?;
                        let mut names = DerReader::new(seq);
                        while !names.is_empty() {
                            let (gtag, gval, _) = names.read_tlv_raw()?;
                            if gtag == 0x82 {
                                if let Ok(s) = core::str::from_utf8(gval) {
                                    san_dns.push(s.to_ascii_lowercase());
                                }
                            } else if gtag == 0x87 && gval.len() == 4 {
                                san_ips.push(gval.try_into().unwrap());
                            } else if gtag == 0x87 && gval.len() == 16 {
                                san_ips_v6.push(gval.try_into().unwrap());
                            }
                        }
                    }
                }
            }
        }

        Ok(Self {
            raw_tbs: tbs_full.to_vec(),
            sig_alg: outer_alg,
            sig_value: sig_bits[1..].to_vec(),
            issuer_der: issuer_full.to_vec(),
            subject_der: subject_full.to_vec(),
            not_before,
            not_after,
            public_key,
            is_ca,
            path_len,
            key_cert_sign,
            server_auth,
            san_dns,
            san_ips,
            san_ips_v6,
        })
    }

    pub fn verify_signed_by(&self, issuer: &Certificate) -> Result<(), TlsError> {
        if self.issuer_der != issuer.subject_der {
            return Err(TlsError::InvalidCertificate);
        }
        if !issuer.is_ca || issuer.key_cert_sign == Some(false) {
            return Err(TlsError::InvalidCertificate);
        }
        verify_signature(&issuer.public_key, &self.sig_alg, &self.raw_tbs, &self.sig_value)
    }

    pub fn matches_host(&self, host: &str) -> bool {
        if let Some(ip) = parse_ipv4_literal(host) {
            return self.san_ips.contains(&ip);
        }
        let unbracketed = host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(host);
        if let Some(ip6) = parse_ipv6_literal(unbracketed) {
            return self.san_ips_v6.contains(&ip6);
        }
        let target = host.to_ascii_lowercase();
        self.san_dns.iter().any(|pat| dns_name_matches(pat, &target))
    }
}

pub fn verify_signature(
    key: &PublicKey,
    alg: &SigAlg,
    msg: &[u8],
    sig: &[u8],
) -> Result<(), TlsError> {
    let hash = sha256(msg);
    let ok = match (key, alg) {
        (PublicKey::Rsa { n, e }, SigAlg::RsaSha256) => rsa::verify_pkcs1_sha256(n, e, &hash, sig),
        (PublicKey::EcdsaP256(pub_sec1), SigAlg::EcdsaP256Sha256) => {
            let (r, s) = parse_ecdsa_sig_der(sig)?;
            ecdsa_p256::verify_p256_sha256(pub_sec1, &hash, r, s)
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(TlsError::BadSignature)
    }
}

fn dns_name_matches(pattern: &str, host: &str) -> bool {
    if pattern == host {
        return true;
    }
    if let Some(rest) = pattern.strip_prefix("*.") {
        if !rest.contains('*') && rest.contains('.') {
            if let Some((_, suffix)) = host.split_once('.') {
                return suffix == rest;
            }
        }
    }
    false
}

fn parse_ipv4_literal(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for slot in &mut out {
        let p = parts.next()?;
        *slot = p.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

fn parse_ipv6_literal(s: &str) -> Option<[u8; 16]> {
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

pub fn verify_chain(
    chain_der: &[Vec<u8>],
    anchors: &[Certificate],
    hostname: &str,
    now_unix: u64,
) -> Result<Certificate, TlsError> {
    if chain_der.is_empty() {
        return Err(TlsError::InvalidCertificate);
    }
    let mut chain = Vec::with_capacity(chain_der.len());
    for raw in chain_der {
        let cert = Certificate::parse(raw)?;
        if now_unix < cert.not_before || now_unix > cert.not_after {
            return Err(TlsError::CertificateExpired);
        }
        chain.push(cert);
    }
    let leaf = &chain[0];
    if leaf.server_auth == Some(false) || !leaf.matches_host(hostname) {
        return Err(TlsError::InvalidCertificate);
    }

    for i in 0..chain.len() {
        let cur = &chain[i];
        // Check if current certificate is itself directly anchored or signed by a trusted root.
        if let Some(root) = anchors.iter().find(|a| {
            now_unix >= a.not_before
                && now_unix <= a.not_after
                && cur.issuer_der == a.subject_der
                && a.is_ca
        }) {
            cur.verify_signed_by(root)?;
            if let Some(max_depth) = root.path_len {
                if (i as u32) > max_depth {
                    return Err(TlsError::InvalidCertificate);
                }
            }
            return Ok(chain.swap_remove(0));
        }
        if let Some(next) = chain.get(i + 1) {
            if let Some(max_depth) = next.path_len {
                if (i as u32) > max_depth {
                    return Err(TlsError::InvalidCertificate);
                }
            }
            cur.verify_signed_by(next)?;
        } else {
            // Check exact DER / SPKI match for directly pinned root certificate if provided.
            if anchors.iter().any(|a| {
                a.subject_der == cur.subject_der
                    && a.public_key == cur.public_key
                    && cur.verify_signed_by(a).is_ok()
            }) {
                return Ok(chain.swap_remove(0));
            }
            return Err(TlsError::UntrustedRoot);
        }
    }
    Err(TlsError::UntrustedRoot)
}
