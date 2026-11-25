//! Minimal bounded RFC 8446 TLS 1.3 client (`no_std + alloc`).
#![no_std]
extern crate alloc;

use alloc::{collections::VecDeque, vec, vec::Vec};
use core::fmt;

pub mod crypto;
pub mod ecdsa_p256;
pub mod rsa;
pub mod x25519;
pub mod x509;

use crypto::{
    aes128_gcm_open, aes128_gcm_seal, chacha20_poly1305_open, chacha20_poly1305_seal, ct_eq,
    derive_secret, hkdf_expand_label, hkdf_extract, hmac_sha256, sha256, Sha256,
};
pub use x509::Certificate;

const CIPHER_AES_128_GCM_SHA256: u16 = 0x1301;
const CIPHER_CHACHA20_POLY1305_SHA256: u16 = 0x1303;
const GROUP_X25519: u16 = 0x001d;
const SIG_ECDSA_P256_SHA256: u16 = 0x0403;
const SIG_RSA_PSS_RSAE_SHA256: u16 = 0x0804;
const MAX_PLAINTEXT: usize = 16384;
const MAX_RECORD: usize = MAX_PLAINTEXT + 256;
const HRR_RANDOM: [u8; 32] = [
    0xcf, 0x21, 0xad, 0x74, 0xe5, 0x9a, 0x61, 0x11, 0xbe, 0x1d, 0x8c, 0x02, 0x1e, 0x65, 0xb8, 0x91,
    0xc2, 0xa2, 0x11, 0x16, 0x7a, 0xbb, 0x8c, 0x5e, 0x07, 0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TlsError {
    InvalidCertificate,
    CertificateExpired,
    UntrustedRoot,
    BadSignature,
    UnsupportedAlgorithm,
    HandshakeFailed,
    DecryptFailed,
    Alert(u8),
    UnexpectedEof,
    Transport(isize),
}

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCertificate => f.write_str("invalid X.509 certificate"),
            Self::CertificateExpired => f.write_str("certificate expired or not yet valid"),
            Self::UntrustedRoot => f.write_str("untrusted certificate authority"),
            Self::BadSignature => f.write_str("cryptographic signature failed"),
            Self::UnsupportedAlgorithm => f.write_str("unsupported cryptographic algorithm"),
            Self::HandshakeFailed => f.write_str("TLS 1.3 handshake failed"),
            Self::DecryptFailed => f.write_str("TLS record decryption failed"),
            Self::Alert(code) => write!(f, "TLS alert ({code})"),
            Self::UnexpectedEof => f.write_str("unexpected TLS EOF"),
            Self::Transport(code) => write!(f, "transport error ({code})"),
        }
    }
}

pub trait ByteStream {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TlsError>;
    fn write_all_bytes(&mut self, buf: &[u8]) -> Result<(), TlsError>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Suite {
    ChaCha20Poly1305,
    Aes128Gcm,
}

impl Suite {
    fn key_len(self) -> usize {
        match self {
            Self::ChaCha20Poly1305 => 32,
            Self::Aes128Gcm => 16,
        }
    }
}

struct TrafficKey {
    suite: Suite,
    traffic_secret: [u8; 32],
    key: [u8; 32],
    iv: [u8; 12],
    seq: u64,
}

impl TrafficKey {
    fn from_secret(suite: Suite, secret: [u8; 32]) -> Self {
        let mut key = [0u8; 32];
        hkdf_expand_label(&secret, b"key", &[], &mut key[..suite.key_len()]);
        let mut iv = [0u8; 12];
        hkdf_expand_label(&secret, b"iv", &[], &mut iv);
        Self {
            suite,
            traffic_secret: secret,
            key,
            iv,
            seq: 0,
        }
    }

    fn update_secret(&mut self) {
        let mut next = [0u8; 32];
        hkdf_expand_label(&self.traffic_secret, b"traffic upd", &[], &mut next);
        *self = Self::from_secret(self.suite, next);
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = self.iv;
        let seq_be = self.seq.to_be_bytes();
        for i in 0..8 {
            n[4 + i] ^= seq_be[i];
        }
        n
    }

    fn seal_record(&mut self, ctype: u8, data: &[u8]) -> Result<Vec<u8>, TlsError> {
        let mut inner = Vec::with_capacity(data.len() + 1);
        inner.extend_from_slice(data);
        inner.push(ctype);
        let ct_len = inner.len() + 16;
        let aad = [0x17u8, 0x03, 0x03, (ct_len >> 8) as u8, ct_len as u8];
        let nonce = self.nonce();
        self.seq = self.seq.checked_add(1).ok_or(TlsError::HandshakeFailed)?;
        let ct = match self.suite {
            Suite::ChaCha20Poly1305 => chacha20_poly1305_seal(&self.key, &nonce, &aad, &inner),
            Suite::Aes128Gcm => {
                let k: &[u8; 16] = self.key[..16].try_into().unwrap();
                aes128_gcm_seal(k, &nonce, &aad, &inner)
            }
        };
        let mut rec = Vec::with_capacity(5 + ct.len());
        rec.extend_from_slice(&aad);
        rec.extend_from_slice(&ct);
        Ok(rec)
    }

    fn open_record(&mut self, aad: &[u8; 5], ct: &[u8]) -> Result<(u8, Vec<u8>), TlsError> {
        let nonce = self.nonce();
        self.seq = self.seq.checked_add(1).ok_or(TlsError::DecryptFailed)?;
        let mut inner = match self.suite {
            Suite::ChaCha20Poly1305 => chacha20_poly1305_open(&self.key, &nonce, aad, ct),
            Suite::Aes128Gcm => {
                let k: &[u8; 16] = self.key[..16].try_into().unwrap();
                aes128_gcm_open(k, &nonce, aad, ct)
            }
        }
        .ok_or(TlsError::DecryptFailed)?;
        while let Some(&0) = inner.last() {
            inner.pop();
        }
        let ctype = inner.pop().ok_or(TlsError::DecryptFailed)?;
        Ok((ctype, inner))
    }
}

pub struct TlsClient<S: ByteStream> {
    stream: S,
    rx_key: TrafficKey,
    tx_key: TrafficKey,
    rx_buf: VecDeque<u8>,
    hs_buf: Vec<u8>,
    closed: bool,
}

impl<S: ByteStream> TlsClient<S> {
    pub fn connect(
        mut stream: S,
        hostname: &str,
        roots: &[Certificate],
        now_unix: u64,
        entropy32: [u8; 32],
        eph_priv32: [u8; 32],
    ) -> Result<Self, TlsError> {
        let eph_pub = x25519::x25519_base(&eph_priv32);
        let ch = build_client_hello(hostname, &entropy32, &eph_pub);
        let mut transcript = Sha256::new();
        transcript.update(&ch);

        let mut ch_record = Vec::with_capacity(5 + ch.len());
        ch_record.extend_from_slice(&[0x16, 0x03, 0x01, (ch.len() >> 8) as u8, ch.len() as u8]);
        ch_record.extend_from_slice(&ch);
        stream.write_all_bytes(&ch_record)?;

        let mut hs_buf = Vec::new();
        let sh = read_handshake_plain(&mut stream, &mut hs_buf)?;
        let (suite, server_pub) = parse_server_hello(&sh)?;
        transcript.update(&sh);

        let shared = x25519::x25519(&eph_priv32, &server_pub);
        if shared == [0u8; 32] {
            return Err(TlsError::HandshakeFailed);
        }

        let empty_hash = sha256(&[]);
        let early = hkdf_extract(&[], &[0u8; 32]);
        let derived = derive_secret(&early, b"derived", &empty_hash);
        let hs_secret = hkdf_extract(&derived, &shared);

        let ch_sh_hash = transcript.digest_clone();
        let c_hs_secret = derive_secret(&hs_secret, b"c hs traffic", &ch_sh_hash);
        let s_hs_secret = derive_secret(&hs_secret, b"s hs traffic", &ch_sh_hash);
        let mut rx_hs = TrafficKey::from_secret(suite, s_hs_secret);
        let mut tx_hs = TrafficKey::from_secret(suite, c_hs_secret);

        let ee = read_handshake_encrypted(&mut stream, &mut rx_hs, &mut hs_buf)?;
        if ee.first() != Some(&8) {
            return Err(TlsError::HandshakeFailed);
        }
        transcript.update(&ee);

        let cert_msg = read_handshake_encrypted(&mut stream, &mut rx_hs, &mut hs_buf)?;
        if cert_msg.first() != Some(&11) {
            return Err(TlsError::HandshakeFailed);
        }
        let chain_der = parse_certificate_msg(&cert_msg[4..])?;
        let leaf = x509::verify_chain(&chain_der, roots, hostname, now_unix)?;
        transcript.update(&cert_msg);

        let cv_msg = read_handshake_encrypted(&mut stream, &mut rx_hs, &mut hs_buf)?;
        if cv_msg.first() != Some(&15) {
            return Err(TlsError::HandshakeFailed);
        }
        verify_cert_verify(&leaf.public_key, &cv_msg[4..], &transcript.digest_clone())?;
        transcript.update(&cv_msg);

        let fin_msg = read_handshake_encrypted(&mut stream, &mut rx_hs, &mut hs_buf)?;
        if fin_msg.first() != Some(&20) {
            return Err(TlsError::HandshakeFailed);
        }
        let mut s_fin_key = [0u8; 32];
        hkdf_expand_label(&s_hs_secret, b"finished", &[], &mut s_fin_key);
        let expected_fin = hmac_sha256(&s_fin_key, &transcript.digest_clone());
        if fin_msg[4..].len() != 32 || !ct_eq(&fin_msg[4..], &expected_fin) {
            return Err(TlsError::BadSignature);
        }
        transcript.update(&fin_msg);

        let sf_hash = transcript.digest_clone();
        let derived2 = derive_secret(&hs_secret, b"derived", &empty_hash);
        let master = hkdf_extract(&derived2, &[0u8; 32]);
        let c_ap_secret = derive_secret(&master, b"c ap traffic", &sf_hash);
        let s_ap_secret = derive_secret(&master, b"s ap traffic", &sf_hash);

        let mut c_fin_key = [0u8; 32];
        hkdf_expand_label(&c_hs_secret, b"finished", &[], &mut c_fin_key);
        let c_verify = hmac_sha256(&c_fin_key, &sf_hash);
        let mut c_fin = Vec::with_capacity(36);
        c_fin.extend_from_slice(&[20, 0, 0, 32]);
        c_fin.extend_from_slice(&c_verify);
        let rec = tx_hs.seal_record(22, &c_fin)?;
        stream.write_all_bytes(&rec)?;

        Ok(Self {
            stream,
            rx_key: TrafficKey::from_secret(suite, s_ap_secret),
            tx_key: TrafficKey::from_secret(suite, c_ap_secret),
            rx_buf: VecDeque::new(),
            hs_buf,
            closed: false,
        })
    }

    pub fn read(&mut self, out: &mut [u8]) -> Result<usize, TlsError> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.rx_buf.is_empty() {
            if self.closed {
                return Ok(0);
            }
            let Some((ctype, payload)) = read_record_encrypted(&mut self.stream, &mut self.rx_key)? else {
                return Err(TlsError::UnexpectedEof);
            };
            match ctype {
                23 => self.rx_buf.extend(payload),
                22 => self.handle_post_handshake(&payload)?,
                21 => {
                    if payload.len() >= 2 && payload[0] == 1 && payload[1] == 0 {
                        self.closed = true;
                        return Ok(0);
                    }
                    let desc = payload.get(1).copied().unwrap_or(0);
                    return Err(TlsError::Alert(desc));
                }
                _ => return Err(TlsError::HandshakeFailed),
            }
        }
        let take = out.len().min(self.rx_buf.len());
        for slot in &mut out[..take] {
            *slot = self.rx_buf.pop_front().unwrap();
        }
        Ok(take)
    }

    pub fn write_all(&mut self, data: &[u8]) -> Result<(), TlsError> {
        if self.closed {
            return Err(TlsError::UnexpectedEof);
        }
        for chunk in data.chunks(MAX_PLAINTEXT) {
            let rec = self.tx_key.seal_record(23, chunk)?;
            self.stream.write_all_bytes(&rec)?;
        }
        Ok(())
    }

    pub fn close_notify(&mut self) -> Result<(), TlsError> {
        if !self.closed {
            let rec = self.tx_key.seal_record(21, &[1, 0])?;
            self.stream.write_all_bytes(&rec)?;
            self.closed = true;
        }
        Ok(())
    }

    fn handle_post_handshake(&mut self, chunk: &[u8]) -> Result<(), TlsError> {
        self.hs_buf.extend_from_slice(chunk);
        while self.hs_buf.len() >= 4 {
            let msg_len =
                ((self.hs_buf[1] as usize) << 16) | ((self.hs_buf[2] as usize) << 8) | (self.hs_buf[3] as usize);
            if self.hs_buf.len() < 4 + msg_len {
                break;
            }
            let msg_type = self.hs_buf[0];
            let body = self.hs_buf[4..4 + msg_len].to_vec();
            self.hs_buf.drain(..4 + msg_len);
            match msg_type {
                4 => {} // NewSessionTicket: stateless/single-connection ignored.
                24 => {
                    if body.len() == 1 && body[0] <= 1 {
                        self.rx_key.update_secret();
                        if body[0] == 1 {
                            let rec = self.tx_key.seal_record(22, &[24, 0, 0, 1, 0])?;
                            self.stream.write_all_bytes(&rec)?;
                            self.tx_key.update_secret();
                        }
                    } else {
                        return Err(TlsError::HandshakeFailed);
                    }
                }
                _ => return Err(TlsError::HandshakeFailed),
            }
        }
        Ok(())
    }
}

fn read_exact_stream<S: ByteStream>(stream: &mut S, mut out: &mut [u8]) -> Result<bool, TlsError> {
    let mut read_any = false;
    while !out.is_empty() {
        let n = stream.read_bytes(out)?;
        if n == 0 {
            return if !read_any {
                Ok(false)
            } else {
                Err(TlsError::UnexpectedEof)
            };
        }
        read_any = true;
        out = &mut out[n..];
    }
    Ok(true)
}

fn read_raw_record<S: ByteStream>(stream: &mut S) -> Result<Option<([u8; 5], Vec<u8>)>, TlsError> {
    let mut hdr = [0u8; 5];
    if !read_exact_stream(stream, &mut hdr)? {
        return Ok(None);
    }
    let len = usize::from(u16::from_be_bytes([hdr[3], hdr[4]]));
    if len > MAX_RECORD {
        return Err(TlsError::HandshakeFailed);
    }
    let mut body = vec![0u8; len];
    if len > 0 && !read_exact_stream(stream, &mut body)? {
        return Err(TlsError::UnexpectedEof);
    }
    Ok(Some((hdr, body)))
}

fn read_record_encrypted<S: ByteStream>(
    stream: &mut S,
    rx_key: &mut TrafficKey,
) -> Result<Option<(u8, Vec<u8>)>, TlsError> {
    loop {
        let Some((hdr, body)) = read_raw_record(stream)? else {
            return Ok(None);
        };
        if hdr[0] == 20 && body == [1] {
            continue; // Ignore TLS 1.3 middlebox compatibility ChangeCipherSpec.
        }
        if hdr[0] == 21 {
            let desc = body.get(1).copied().unwrap_or(0);
            return Err(TlsError::Alert(desc));
        }
        if hdr[0] != 23 {
            return Err(TlsError::HandshakeFailed);
        }
        let (ctype, inner) = rx_key.open_record(&hdr, &body)?;
        return Ok(Some((ctype, inner)));
    }
}

fn pop_hs_message(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    if buf.len() < 4 {
        return None;
    }
    let msg_len = ((buf[1] as usize) << 16) | ((buf[2] as usize) << 8) | (buf[3] as usize);
    if buf.len() < 4 + msg_len {
        return None;
    }
    Some(buf.drain(..4 + msg_len).collect())
}

fn read_handshake_plain<S: ByteStream>(stream: &mut S, buf: &mut Vec<u8>) -> Result<Vec<u8>, TlsError> {
    loop {
        if let Some(msg) = pop_hs_message(buf) {
            return Ok(msg);
        }
        let Some((hdr, body)) = read_raw_record(stream)? else {
            return Err(TlsError::UnexpectedEof);
        };
        if hdr[0] == 20 && body == [1] {
            continue;
        }
        if hdr[0] == 21 {
            return Err(TlsError::Alert(body.get(1).copied().unwrap_or(0)));
        }
        if hdr[0] != 22 {
            return Err(TlsError::HandshakeFailed);
        }
        buf.extend_from_slice(&body);
    }
}

fn read_handshake_encrypted<S: ByteStream>(
    stream: &mut S,
    rx_key: &mut TrafficKey,
    buf: &mut Vec<u8>,
) -> Result<Vec<u8>, TlsError> {
    loop {
        if let Some(msg) = pop_hs_message(buf) {
            return Ok(msg);
        }
        let Some((ctype, body)) = read_record_encrypted(stream, rx_key)? else {
            return Err(TlsError::UnexpectedEof);
        };
        if ctype == 21 {
            return Err(TlsError::Alert(body.get(1).copied().unwrap_or(0)));
        }
        if ctype != 22 {
            return Err(TlsError::HandshakeFailed);
        }
        buf.extend_from_slice(&body);
    }
}

fn build_client_hello(hostname: &str, random: &[u8; 32], x25519_pub: &[u8; 32]) -> Vec<u8> {
    let mut exts = Vec::new();

    // SNI (0x0000) when hostname is not an IPv4 literal.
    if !hostname.is_empty() && !is_ipv4_str(hostname) {
        let hbytes = hostname.as_bytes();
        let name_len = hbytes.len() as u16;
        let list_len = name_len + 3;
        let ext_len = list_len + 2;
        exts.extend_from_slice(&0x0000u16.to_be_bytes());
        exts.extend_from_slice(&ext_len.to_be_bytes());
        exts.extend_from_slice(&list_len.to_be_bytes());
        exts.push(0); // host_name
        exts.extend_from_slice(&name_len.to_be_bytes());
        exts.extend_from_slice(hbytes);
    }

    // supported_versions (0x002b): [0x03, 0x04]
    exts.extend_from_slice(&[0x00, 0x2b, 0x00, 0x03, 0x02, 0x03, 0x04]);
    // supported_groups (0x000a): [x25519 (0x001d)]
    exts.extend_from_slice(&[0x00, 0x0a, 0x00, 0x04, 0x00, 0x02, 0x00, 0x1d]);
    // signature_algorithms (0x000d): [ecdsa_secp256r1_sha256, rsa_pss_rsae_sha256, rsa_pkcs1_sha256]
    exts.extend_from_slice(&[
        0x00, 0x0d, 0x00, 0x08, 0x00, 0x06, 0x04, 0x03, 0x08, 0x04, 0x04, 0x01,
    ]);
    // key_share (0x0033): client X25519 public key
    exts.extend_from_slice(&[0x00, 0x33, 0x00, 0x26, 0x00, 0x24, 0x00, 0x1d, 0x00, 0x20]);
    exts.extend_from_slice(x25519_pub);

    let mut body = Vec::with_capacity(128 + exts.len());
    body.extend_from_slice(&[0x03, 0x03]); // legacy_version = TLS 1.2
    body.extend_from_slice(random);
    body.push(0); // legacy_session_id = empty
    body.extend_from_slice(&[0x00, 0x04]); // cipher_suites len = 4
    body.extend_from_slice(&CIPHER_CHACHA20_POLY1305_SHA256.to_be_bytes());
    body.extend_from_slice(&CIPHER_AES_128_GCM_SHA256.to_be_bytes());
    body.extend_from_slice(&[0x01, 0x00]); // legacy_compression = [null]
    body.extend_from_slice(&(exts.len() as u16).to_be_bytes());
    body.extend_from_slice(&exts);

    let mut hs = Vec::with_capacity(4 + body.len());
    hs.push(1); // ClientHello
    hs.push((body.len() >> 16) as u8);
    hs.push((body.len() >> 8) as u8);
    hs.push(body.len() as u8);
    hs.extend_from_slice(&body);
    hs
}

fn is_ipv4_str(s: &str) -> bool {
    let mut count = 0;
    for part in s.split('.') {
        if part.parse::<u8>().is_err() {
            return false;
        }
        count += 1;
    }
    count == 4
}

fn parse_server_hello(msg: &[u8]) -> Result<(Suite, [u8; 32]), TlsError> {
    if msg.len() < 4 || msg[0] != 2 {
        return Err(TlsError::HandshakeFailed);
    }
    let body = &msg[4..];
    if body.len() < 38 || body[0..2] != [0x03, 0x03] {
        return Err(TlsError::HandshakeFailed);
    }
    if body[2..34] == HRR_RANDOM {
        return Err(TlsError::HandshakeFailed);
    }
    let sid_len = usize::from(body[34]);
    let mut pos = 35 + sid_len;
    if pos + 5 > body.len() {
        return Err(TlsError::HandshakeFailed);
    }
    let suite_id = u16::from_be_bytes([body[pos], body[pos + 1]]);
    let suite = match suite_id {
        CIPHER_CHACHA20_POLY1305_SHA256 => Suite::ChaCha20Poly1305,
        CIPHER_AES_128_GCM_SHA256 => Suite::Aes128Gcm,
        _ => return Err(TlsError::UnsupportedAlgorithm),
    };
    pos += 2;
    if body[pos] != 0 {
        return Err(TlsError::HandshakeFailed);
    }
    pos += 1;
    let ext_len = usize::from(u16::from_be_bytes([body[pos], body[pos + 1]]));
    pos += 2;
    if pos + ext_len != body.len() {
        return Err(TlsError::HandshakeFailed);
    }
    let mut exts = &body[pos..];
    let mut got_v13 = false;
    let mut server_pub: Option<[u8; 32]> = None;

    while !exts.is_empty() {
        if exts.len() < 4 {
            return Err(TlsError::HandshakeFailed);
        }
        let etype = u16::from_be_bytes([exts[0], exts[1]]);
        let elen = usize::from(u16::from_be_bytes([exts[2], exts[3]]));
        if 4 + elen > exts.len() {
            return Err(TlsError::HandshakeFailed);
        }
        let eval = &exts[4..4 + elen];
        exts = &exts[4 + elen..];
        if etype == 0x002b {
            if eval != [0x03, 0x04] {
                return Err(TlsError::HandshakeFailed);
            }
            got_v13 = true;
        } else if etype == 0x0033 {
            if eval.len() != 36
                || u16::from_be_bytes([eval[0], eval[1]]) != GROUP_X25519
                || u16::from_be_bytes([eval[2], eval[3]]) != 32
            {
                return Err(TlsError::UnsupportedAlgorithm);
            }
            server_pub = Some(eval[4..36].try_into().unwrap());
        }
    }
    match (got_v13, server_pub) {
        (true, Some(pk)) => Ok((suite, pk)),
        _ => Err(TlsError::HandshakeFailed),
    }
}

fn parse_certificate_msg(body: &[u8]) -> Result<Vec<Vec<u8>>, TlsError> {
    if body.len() < 4 {
        return Err(TlsError::InvalidCertificate);
    }
    let ctx_len = usize::from(body[0]);
    let mut pos = 1 + ctx_len;
    if pos + 3 > body.len() {
        return Err(TlsError::InvalidCertificate);
    }
    let list_len = ((body[pos] as usize) << 16) | ((body[pos + 1] as usize) << 8) | (body[pos + 2] as usize);
    pos += 3;
    if pos + list_len != body.len() {
        return Err(TlsError::InvalidCertificate);
    }
    let mut out = Vec::new();
    let end = pos + list_len;
    while pos < end {
        if pos + 3 > end {
            return Err(TlsError::InvalidCertificate);
        }
        let clen = ((body[pos] as usize) << 16) | ((body[pos + 1] as usize) << 8) | (body[pos + 2] as usize);
        pos += 3;
        if pos + clen + 2 > end {
            return Err(TlsError::InvalidCertificate);
        }
        out.push(body[pos..pos + clen].to_vec());
        pos += clen;
        let ext_len = usize::from(u16::from_be_bytes([body[pos], body[pos + 1]]));
        pos += 2;
        if pos + ext_len > end {
            return Err(TlsError::InvalidCertificate);
        }
        pos += ext_len;
    }
    Ok(out)
}

fn verify_cert_verify(
    pubkey: &x509::PublicKey,
    body: &[u8],
    transcript_hash: &[u8; 32],
) -> Result<(), TlsError> {
    if body.len() < 4 {
        return Err(TlsError::BadSignature);
    }
    let sig_alg = u16::from_be_bytes([body[0], body[1]]);
    let sig_len = usize::from(u16::from_be_bytes([body[2], body[3]]));
    if 4 + sig_len != body.len() {
        return Err(TlsError::BadSignature);
    }
    let sig = &body[4..];
    let mut content = Vec::with_capacity(64 + 34 + 32);
    content.extend_from_slice(&[0x20u8; 64]);
    content.extend_from_slice(b"TLS 1.3, server CertificateVerify\x00");
    content.extend_from_slice(transcript_hash);

    let ok = match (pubkey, sig_alg) {
        (x509::PublicKey::Rsa { n, e }, SIG_RSA_PSS_RSAE_SHA256) => {
            rsa::verify_pss_sha256(n, e, &content, sig)
        }
        (x509::PublicKey::EcdsaP256(sec1), SIG_ECDSA_P256_SHA256) => {
            let (r, s) = x509::parse_ecdsa_sig_der(sig)?;
            ecdsa_p256::verify_p256_sha256(sec1, &sha256(&content), r, s)
        }
        _ => return Err(TlsError::UnsupportedAlgorithm),
    };
    if ok {
        Ok(())
    } else {
        Err(TlsError::BadSignature)
    }
}

#[cfg(test)]
mod tests;

