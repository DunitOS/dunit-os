//! Standard symmetric & hash primitives for TLS 1.3 (SHA-256, HKDF, ChaCha20-Poly1305, AES-128-GCM).
use alloc::vec::Vec;

const SHA256_IV: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_bits: u64,
}

impl Sha256 {
    pub const fn new() -> Self {
        Self {
            state: SHA256_IV,
            buf: [0; 64],
            buf_len: 0,
            total_bits: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total_bits = self.total_bits.wrapping_add((data.len() as u64).wrapping_mul(8));
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let blk = self.buf;
                compress_sha256(&mut self.state, &blk);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let blk: &[u8; 64] = data[..64].try_into().unwrap();
            compress_sha256(&mut self.state, blk);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            self.buf[self.buf_len..64].fill(0);
            let blk = self.buf;
            compress_sha256(&mut self.state, &blk);
            self.buf_len = 0;
        }
        self.buf[self.buf_len..56].fill(0);
        self.buf[56..64].copy_from_slice(&self.total_bits.to_be_bytes());
        let blk = self.buf;
        compress_sha256(&mut self.state, &blk);
        let mut out = [0u8; 32];
        for i in 0..8 {
            out[4 * i..4 * (i + 1)].copy_from_slice(&self.state[i].to_be_bytes());
        }
        out
    }

    pub fn digest_clone(&self) -> [u8; 32] {
        self.clone().finalize()
    }
}

fn compress_sha256(state: &mut [u32; 8], blk: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes(blk[4 * i..4 * (i + 1)].try_into().unwrap());
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    let add = [a, b, c, d, e, f, g, h];
    for i in 0..8 {
        state[i] = state[i].wrapping_add(add[i]);
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize()
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k_block = [0u8; 64];
    if key.len() > 64 {
        k_block[..32].copy_from_slice(&sha256(key));
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k_block[i];
        opad[i] ^= k_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    outer.finalize()
}

pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    let salt_eff = if salt.is_empty() { &[0u8; 32][..] } else { salt };
    hmac_sha256(salt_eff, ikm)
}

pub fn hkdf_expand(prk: &[u8; 32], info: &[u8], out: &mut [u8]) {
    let mut prev = [0u8; 32];
    let mut pos = 0usize;
    let mut counter = 1u8;
    while pos < out.len() {
        let mut msg = Vec::with_capacity(32 + info.len() + 1);
        if counter > 1 {
            msg.extend_from_slice(&prev);
        }
        msg.extend_from_slice(info);
        msg.push(counter);
        prev = hmac_sha256(prk, &msg);
        let take = (out.len() - pos).min(32);
        out[pos..pos + take].copy_from_slice(&prev[..take]);
        pos += take;
        counter = counter.wrapping_add(1);
    }
}

pub fn hkdf_expand_label(secret: &[u8; 32], label: &[u8], context: &[u8], out: &mut [u8]) {
    let full_len = 6 + label.len();
    let mut info = Vec::with_capacity(2 + 1 + full_len + 1 + context.len());
    info.extend_from_slice(&(out.len() as u16).to_be_bytes());
    info.push(full_len as u8);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label);
    info.push(context.len() as u8);
    info.extend_from_slice(context);
    hkdf_expand(secret, &info, out);
}

pub fn derive_secret(secret: &[u8; 32], label: &[u8], transcript_hash: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    hkdf_expand_label(secret, label, transcript_hash, &mut out);
    out
}

pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// --- ChaCha20-Poly1305 (RFC 8439) ---

const CHACHA_SIGMA: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

#[inline]
fn qr(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut s = [0u32; 16];
    s[0..4].copy_from_slice(&CHACHA_SIGMA);
    for i in 0..8 {
        s[4 + i] = u32::from_le_bytes(key[4 * i..4 * (i + 1)].try_into().unwrap());
    }
    s[12] = counter;
    for i in 0..3 {
        s[13 + i] = u32::from_le_bytes(nonce[4 * i..4 * (i + 1)].try_into().unwrap());
    }
    let init = s;
    for _ in 0..10 {
        qr(&mut s, 0, 4, 8, 12);
        qr(&mut s, 1, 5, 9, 13);
        qr(&mut s, 2, 6, 10, 14);
        qr(&mut s, 3, 7, 11, 15);
        qr(&mut s, 0, 5, 10, 15);
        qr(&mut s, 1, 6, 11, 12);
        qr(&mut s, 2, 7, 8, 13);
        qr(&mut s, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[4 * i..4 * (i + 1)].copy_from_slice(&s[i].wrapping_add(init[i]).to_le_bytes());
    }
    out
}

fn chacha20_xor(key: &[u8; 32], mut counter: u32, nonce: &[u8; 12], data: &mut [u8]) {
    let mut pos = 0;
    while pos < data.len() {
        let ks = chacha20_block(key, counter, nonce);
        let take = (data.len() - pos).min(64);
        for i in 0..take {
            data[pos + i] ^= ks[i];
        }
        pos += take;
        counter = counter.wrapping_add(1);
    }
}

fn poly1305_tag(one_time_key: &[u8; 32], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let mut r_bytes = [0u8; 16];
    r_bytes.copy_from_slice(&one_time_key[..16]);
    r_bytes[3] &= 15;
    r_bytes[7] &= 15;
    r_bytes[11] &= 15;
    r_bytes[15] &= 15;
    r_bytes[4] &= 252;
    r_bytes[8] &= 252;
    r_bytes[12] &= 252;

    let r = u128::from_le_bytes(r_bytes);
    let r0 = r & 0x03ff_ffff;
    let r1 = (r >> 26) & 0x03ff_ffff;
    let r2 = (r >> 52) & 0x03ff_ffff;
    let r3 = (r >> 78) & 0x03ff_ffff;
    let r4 = (r >> 104) & 0x03ff_ffff;

    let s1 = r1 * 5;
    let s2 = r2 * 5;
    let s3 = r3 * 5;
    let s4 = r4 * 5;

    let (mut h0, mut h1, mut h2, mut h3, mut h4) = (0u128, 0u128, 0u128, 0u128, 0u128);
    let mut feed_block = |blk: &[u8], pad_bit: u128| {
        let mut tmp = [0u8; 16];
        tmp[..blk.len()].copy_from_slice(blk);
        let t = u128::from_le_bytes(tmp);
        h0 += t & 0x03ff_ffff;
        h1 += (t >> 26) & 0x03ff_ffff;
        h2 += (t >> 52) & 0x03ff_ffff;
        h3 += (t >> 78) & 0x03ff_ffff;
        h4 += ((t >> 104) & 0x03ff_ffff) | pad_bit;

        let d0 = h0 * r0 + h1 * s4 + h2 * s3 + h3 * s2 + h4 * s1;
        let mut d1 = h0 * r1 + h1 * r0 + h2 * s4 + h3 * s3 + h4 * s2;
        let mut d2 = h0 * r2 + h1 * r1 + h2 * r0 + h3 * s4 + h4 * s3;
        let mut d3 = h0 * r3 + h1 * r2 + h2 * r1 + h3 * r0 + h4 * s4;
        let mut d4 = h0 * r4 + h1 * r3 + h2 * r2 + h3 * r1 + h4 * r0;

        let c = d0 >> 26;
        h0 = d0 & 0x03ff_ffff;
        d1 += c;
        let c = d1 >> 26;
        h1 = d1 & 0x03ff_ffff;
        d2 += c;
        let c = d2 >> 26;
        h2 = d2 & 0x03ff_ffff;
        d3 += c;
        let c = d3 >> 26;
        h3 = d3 & 0x03ff_ffff;
        d4 += c;
        let c = d4 >> 26;
        h4 = d4 & 0x03ff_ffff;
        h0 += c * 5;
        let c = h0 >> 26;
        h0 &= 0x03ff_ffff;
        h1 += c;
    };

    for slice in [aad, ct] {
        for chunk in slice.chunks(16) {
            if chunk.len() == 16 {
                feed_block(chunk, 1 << 24);
            } else {
                let mut padded = [0u8; 16];
                padded[..chunk.len()].copy_from_slice(chunk);
                feed_block(&padded, 1 << 24);
            }
        }
    }
    let mut lens = [0u8; 16];
    lens[..8].copy_from_slice(&(aad.len() as u64).to_le_bytes());
    lens[8..16].copy_from_slice(&(ct.len() as u64).to_le_bytes());
    feed_block(&lens, 1 << 24);

    let c = h1 >> 26;
    h1 &= 0x03ff_ffff;
    h2 += c;
    let c = h2 >> 26;
    h2 &= 0x03ff_ffff;
    h3 += c;
    let c = h3 >> 26;
    h3 &= 0x03ff_ffff;
    h4 += c;
    let c = h4 >> 26;
    h4 &= 0x03ff_ffff;
    h0 += c * 5;
    let c = h0 >> 26;
    h0 &= 0x03ff_ffff;
    h1 += c;

    // Compute h - (2^130 - 5) and select in constant time.
    let mut g0 = h0 + 5;
    let c = g0 >> 26;
    g0 &= 0x03ff_ffff;
    let mut g1 = h1 + c;
    let c = g1 >> 26;
    g1 &= 0x03ff_ffff;
    let mut g2 = h2 + c;
    let c = g2 >> 26;
    g2 &= 0x03ff_ffff;
    let mut g3 = h3 + c;
    let c = g3 >> 26;
    g3 &= 0x03ff_ffff;
    let g4 = h4.wrapping_add(c).wrapping_sub(1 << 26);

    let mask = ((g4 >> 127) & 1).wrapping_sub(1);
    g0 &= mask;
    g1 &= mask;
    g2 &= mask;
    g3 &= mask;
    g4_select(&mut h0, &mut h1, &mut h2, &mut h3, &mut h4, g0, g1, g2, g3, g4 & mask, !mask);

    let mut f = (h0) | (h1 << 26) | (h2 << 52) | (h3 << 78) | (h4 << 104);
    let s = u128::from_le_bytes(one_time_key[16..32].try_into().unwrap());
    f = f.wrapping_add(s);
    f.to_le_bytes()
}

#[allow(clippy::too_many_arguments)]
fn g4_select(
    h0: &mut u128,
    h1: &mut u128,
    h2: &mut u128,
    h3: &mut u128,
    h4: &mut u128,
    g0: u128,
    g1: u128,
    g2: u128,
    g3: u128,
    g4: u128,
    inv_mask: u128,
) {
    *h0 = (*h0 & inv_mask) | g0;
    *h1 = (*h1 & inv_mask) | g1;
    *h2 = (*h2 & inv_mask) | g2;
    *h3 = (*h3 & inv_mask) | g3;
    *h4 = (*h4 & inv_mask) | g4;
}

pub fn chacha20_poly1305_seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], pt: &[u8]) -> Vec<u8> {
    let otk: [u8; 32] = chacha20_block(key, 0, nonce)[..32].try_into().unwrap();
    let mut out = Vec::with_capacity(pt.len() + 16);
    out.extend_from_slice(pt);
    chacha20_xor(key, 1, nonce, &mut out);
    let tag = poly1305_tag(&otk, aad, &out);
    out.extend_from_slice(&tag);
    out
}

pub fn chacha20_poly1305_open(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    ct_and_tag: &[u8],
) -> Option<Vec<u8>> {
    if ct_and_tag.len() < 16 {
        return None;
    }
    let (ct, tag) = ct_and_tag.split_at(ct_and_tag.len() - 16);
    let otk: [u8; 32] = chacha20_block(key, 0, nonce)[..32].try_into().unwrap();
    let expected = poly1305_tag(&otk, aad, ct);
    if !ct_eq(&expected, tag) {
        return None;
    }
    let mut pt = ct.to_vec();
    chacha20_xor(key, 1, nonce, &mut pt);
    Some(pt)
}

// --- AES-128-GCM (NIST FIPS 197 + SP 800-38D) ---

const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];

fn aes128_expand(key: &[u8; 16]) -> [[u8; 16]; 11] {
    let mut w = [[0u8; 4]; 44];
    for i in 0..4 {
        w[i] = key[4 * i..4 * (i + 1)].try_into().unwrap();
    }
    for i in 4..44 {
        let mut t = w[i - 1];
        if i % 4 == 0 {
            t = [SBOX[t[1] as usize], SBOX[t[2] as usize], SBOX[t[3] as usize], SBOX[t[0] as usize]];
            t[0] ^= RCON[i / 4 - 1];
        }
        for j in 0..4 {
            w[i][j] = w[i - 4][j] ^ t[j];
        }
    }
    let mut rk = [[0u8; 16]; 11];
    for r in 0..11 {
        for c in 0..4 {
            rk[r][4 * c..4 * (c + 1)].copy_from_slice(&w[4 * r + c]);
        }
    }
    rk
}

#[inline]
fn xtime(x: u8) -> u8 {
    (x << 1) ^ (if x & 0x80 != 0 { 0x1b } else { 0 })
}

fn aes128_encrypt_block(rk: &[[u8; 16]; 11], input: &[u8; 16]) -> [u8; 16] {
    let mut s = *input;
    for i in 0..16 {
        s[i] ^= rk[0][i];
    }
    for r in 1..=10 {
        for b in &mut s {
            *b = SBOX[*b as usize];
        }
        // ShiftRows
        s = [
            s[0], s[5], s[10], s[15],
            s[4], s[9], s[14], s[3],
            s[8], s[13], s[2], s[7],
            s[12], s[1], s[6], s[11],
        ];
        if r < 10 {
            for c in 0..4 {
                let i = 4 * c;
                let (a0, a1, a2, a3) = (s[i], s[i + 1], s[i + 2], s[i + 3]);
                let t = a0 ^ a1 ^ a2 ^ a3;
                s[i] = a0 ^ t ^ xtime(a0 ^ a1);
                s[i + 1] = a1 ^ t ^ xtime(a1 ^ a2);
                s[i + 2] = a2 ^ t ^ xtime(a2 ^ a3);
                s[i + 3] = a3 ^ t ^ xtime(a3 ^ a0);
            }
        }
        for i in 0..16 {
            s[i] ^= rk[r][i];
        }
    }
    s
}

fn gf128_mul(mut x: u128, mut y: u128) -> u128 {
    // IEEE/NIST bit order: MSB of byte 0 is bit 127 of u128::from_be_bytes.
    let mut z = 0u128;
    const R: u128 = 0xe100_0000_0000_0000_0000_0000_0000_0000;
    for i in 0..128 {
        if (y >> (127 - i)) & 1 != 0 {
            z ^= x;
        }
        let lsb = x & 1;
        x >>= 1;
        if lsb != 0 {
            x ^= R;
        }
        let _ = &mut y;
    }
    z
}

fn ghash(h: u128, aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let mut y = 0u128;
    for slice in [aad, ct] {
        for chunk in slice.chunks(16) {
            let mut blk = [0u8; 16];
            blk[..chunk.len()].copy_from_slice(chunk);
            y = gf128_mul(y ^ u128::from_be_bytes(blk), h);
        }
    }
    let mut lens = [0u8; 16];
    lens[..8].copy_from_slice(&(aad.len() as u64 * 8).to_be_bytes());
    lens[8..16].copy_from_slice(&(ct.len() as u64 * 8).to_be_bytes());
    y = gf128_mul(y ^ u128::from_be_bytes(lens), h);
    y.to_be_bytes()
}

fn gcm_ctr_xor(rk: &[[u8; 16]; 11], nonce: &[u8; 12], data: &mut [u8]) {
    let mut cb = [0u8; 16];
    cb[..12].copy_from_slice(nonce);
    let mut ctr = 2u32;
    for chunk in data.chunks_mut(16) {
        cb[12..16].copy_from_slice(&ctr.to_be_bytes());
        let ks = aes128_encrypt_block(rk, &cb);
        for (d, k) in chunk.iter_mut().zip(ks.iter()) {
            *d ^= *k;
        }
        ctr = ctr.wrapping_add(1);
    }
}

pub fn aes128_gcm_seal(key: &[u8; 16], nonce: &[u8; 12], aad: &[u8], pt: &[u8]) -> Vec<u8> {
    let rk = aes128_expand(key);
    let h = u128::from_be_bytes(aes128_encrypt_block(&rk, &[0u8; 16]));
    let mut out = Vec::with_capacity(pt.len() + 16);
    out.extend_from_slice(pt);
    gcm_ctr_xor(&rk, nonce, &mut out);

    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;
    let s0 = aes128_encrypt_block(&rk, &j0);
    let mut tag = ghash(h, aad, &out);
    for i in 0..16 {
        tag[i] ^= s0[i];
    }
    out.extend_from_slice(&tag);
    out
}

pub fn aes128_gcm_open(
    key: &[u8; 16],
    nonce: &[u8; 12],
    aad: &[u8],
    ct_and_tag: &[u8],
) -> Option<Vec<u8>> {
    if ct_and_tag.len() < 16 {
        return None;
    }
    let (ct, tag) = ct_and_tag.split_at(ct_and_tag.len() - 16);
    let rk = aes128_expand(key);
    let h = u128::from_be_bytes(aes128_encrypt_block(&rk, &[0u8; 16]));
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;
    let s0 = aes128_encrypt_block(&rk, &j0);
    let mut expected = ghash(h, aad, ct);
    for i in 0..16 {
        expected[i] ^= s0[i];
    }
    if !ct_eq(&expected, tag) {
        return None;
    }
    let mut pt = ct.to_vec();
    gcm_ctr_xor(&rk, nonce, &mut pt);
    Some(pt)
}

