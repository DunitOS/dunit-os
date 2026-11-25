//! BigUint Montgomery modular exponentiation & RSA PKCS#1 v1.5 / PSS verification (SHA-256).
use crate::crypto::{ct_eq, sha256, Sha256};
use alloc::{vec, vec::Vec};

const MAX_LIMBS: usize = 64; // Up to 4096-bit RSA moduli.

const DIGEST_INFO_SHA256: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

fn from_be_bytes(bytes: &[u8]) -> Vec<u64> {
    let trimmed = match bytes.iter().position(|&b| b != 0) {
        Some(pos) => &bytes[pos..],
        None => return vec![0],
    };
    let nlimbs = trimmed.len().div_ceil(8);
    let mut out = vec![0u64; nlimbs];
    for (i, &b) in trimmed.iter().rev().enumerate() {
        out[i >> 3] |= u64::from(b) << ((i & 7) * 8);
    }
    trim(out)
}

fn trim(mut limbs: Vec<u64>) -> Vec<u64> {
    while limbs.len() > 1 && *limbs.last().unwrap() == 0 {
        limbs.pop();
    }
    limbs
}

fn bit_len(limbs: &[u64]) -> usize {
    for (idx, &w) in limbs.iter().enumerate().rev() {
        if w != 0 {
            return idx * 64 + (64 - w.leading_zeros() as usize);
        }
    }
    0
}

fn cmp_ge(a: &[u64], b: &[u64]) -> bool {
    let k = a.len().max(b.len());
    for i in (0..k).rev() {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        if av != bv {
            return av > bv;
        }
    }
    true
}

fn n0_inv(n0: u64) -> u64 {
    // -n0^(-1) mod 2^64 via Newton's method.
    let mut inv = 1u64;
    for _ in 0..6 {
        inv = inv.wrapping_mul(2u64.wrapping_sub(n0.wrapping_mul(inv)));
    }
    inv.wrapping_neg()
}

fn mon_pro(a: &[u64], b: &[u64], n: &[u64], n0p: u64) -> Vec<u64> {
    let k = n.len();
    let mut t = vec![0u64; k + 2];
    for i in 0..k {
        let ai = u128::from(a.get(i).copied().unwrap_or(0));
        let mut c = 0u128;
        for j in 0..k {
            let bj = u128::from(b.get(j).copied().unwrap_or(0));
            let s = u128::from(t[j]) + ai * bj + c;
            t[j] = s as u64;
            c = s >> 64;
        }
        let s = u128::from(t[k]) + c;
        t[k] = s as u64;
        t[k + 1] = t[k + 1].wrapping_add((s >> 64) as u64);

        let m = t[0].wrapping_mul(n0p);
        let s = u128::from(t[0]) + u128::from(m) * u128::from(n[0]);
        let mut c = s >> 64;
        for j in 1..k {
            let s = u128::from(t[j]) + u128::from(m) * u128::from(n[j]) + c;
            t[j - 1] = s as u64;
            c = s >> 64;
        }
        let s = u128::from(t[k]) + c;
        t[k - 1] = s as u64;
        t[k] = t[k + 1].wrapping_add((s >> 64) as u64);
        t[k + 1] = 0;
    }
    let mut res = t[..k].to_vec();
    if t[k] != 0 || cmp_ge(&res, n) {
        let mut borrow = 0i128;
        for i in 0..k {
            let d = i128::from(res[i]) - i128::from(n[i]) - borrow;
            res[i] = d as u64;
            borrow = if d < 0 { 1 } else { 0 };
        }
    }
    res
}

fn r_mod_n(n: &[u64]) -> Vec<u64> {
    // 2^(64*k) mod n
    let k = n.len();
    let mut rem = vec![0u64; k];
    rem[0] = 1;
    for _ in 0..(k * 64) {
        let top = rem[k - 1] >> 63;
        for i in (1..k).rev() {
            rem[i] = (rem[i] << 1) | (rem[i - 1] >> 63);
        }
        rem[0] <<= 1;
        if top != 0 || cmp_ge(&rem, n) {
            let mut borrow = 0i128;
            for i in 0..k {
                let d = i128::from(rem[i]) - i128::from(n[i]) - borrow;
                rem[i] = d as u64;
                borrow = if d < 0 { 1 } else { 0 };
            }
        }
    }
    rem
}

fn rsa_public_op(n_bytes: &[u8], e_bytes: &[u8], sig_bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
    let n = from_be_bytes(n_bytes);
    let e = from_be_bytes(e_bytes);
    let s = from_be_bytes(sig_bytes);
    if n.len() > MAX_LIMBS || n.is_empty() || (n[0] & 1) == 0 {
        return None;
    }
    let bits = bit_len(&n);
    if bits < 1024 || e == [0] || e == [1] || cmp_ge(&s, &n) {
        return None;
    }
    let mod_len = bits.div_ceil(8);
    if sig_bytes.len() != mod_len {
        return None;
    }

    let k = n.len();
    let n0p = n0_inv(n[0]);
    let r = r_mod_n(&n);
    let r2 = r.clone();
    // Compute R^2 mod n via doubling R mod n k*64 times.
    let mut r2_mod = r2;
    for _ in 0..(k * 64) {
        let mut carry = 0u64;
        for limb in &mut r2_mod {
            let next = *limb >> 63;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
        if carry != 0 || cmp_ge(&r2_mod, &n) {
            let mut borrow = 0i128;
            for i in 0..k {
                let d = i128::from(r2_mod[i]) - i128::from(n[i]) - borrow;
                r2_mod[i] = d as u64;
                borrow = if d < 0 { 1 } else { 0 };
            }
        }
    }

    let mut s_pad = vec![0u64; k];
    s_pad[..s.len()].copy_from_slice(&s);
    let mut base = mon_pro(&s_pad, &r2_mod, &n, n0p);
    let mut acc = r;
    let e_bits = bit_len(&e);
    for b in 0..e_bits {
        if ((e[b >> 6] >> (b & 63)) & 1) != 0 {
            acc = mon_pro(&acc, &base, &n, n0p);
        }
        if b + 1 < e_bits {
            base = mon_pro(&base, &base, &n, n0p);
        }
    }
    let mut one = vec![0u64; k];
    one[0] = 1;
    let plain = mon_pro(&acc, &one, &n, n0p);

    let mut em = vec![0u8; mod_len];
    for (i, out) in em.iter_mut().rev().enumerate() {
        let word = plain.get(i >> 3).copied().unwrap_or(0);
        *out = (word >> ((i & 7) * 8)) as u8;
    }
    Some((em, bits))
}

pub fn verify_pkcs1_sha256(n_bytes: &[u8], e_bytes: &[u8], msg_hash: &[u8; 32], sig_bytes: &[u8]) -> bool {
    let Some((em, _)) = rsa_public_op(n_bytes, e_bytes, sig_bytes) else {
        return false;
    };
    let t_len = DIGEST_INFO_SHA256.len() + 32;
    if em.len() < t_len + 11 || em[0] != 0x00 || em[1] != 0x01 {
        return false;
    }
    let ps_end = em.len() - t_len - 1;
    if em[ps_end] != 0x00 || !em[2..ps_end].iter().all(|&b| b == 0xff) {
        return false;
    }
    ct_eq(&em[ps_end + 1..ps_end + 1 + DIGEST_INFO_SHA256.len()], &DIGEST_INFO_SHA256)
        && ct_eq(&em[em.len() - 32..], msg_hash)
}

fn mgf1_sha256(seed: &[u8], mask_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(mask_len + 32);
    let mut ctr = 0u32;
    while out.len() < mask_len {
        let mut h = Sha256::new();
        h.update(seed);
        h.update(&ctr.to_be_bytes());
        out.extend_from_slice(&h.finalize());
        ctr += 1;
    }
    out.truncate(mask_len);
    out
}

pub fn verify_pss_sha256(n_bytes: &[u8], e_bytes: &[u8], msg: &[u8], sig_bytes: &[u8]) -> bool {
    let Some((em, mod_bits)) = rsa_public_op(n_bytes, e_bytes, sig_bytes) else {
        return false;
    };
    let em_bits = mod_bits - 1;
    let em_len = em_bits.div_ceil(8);
    if em.len() != em_len {
        return false;
    }
    const HLEN: usize = 32;
    const SLEN: usize = 32;
    if em_len < HLEN + SLEN + 2 || em[em_len - 1] != 0xbc {
        return false;
    }
    let top_mask = 0xffu8 >> (8 * em_len - em_bits);
    if (em[0] & !top_mask) != 0 {
        return false;
    }
    let db_len = em_len - HLEN - 1;
    let (masked_db, h_slice) = em[..em_len - 1].split_at(db_len);
    let mask = mgf1_sha256(h_slice, db_len);
    let mut db: Vec<u8> = masked_db.iter().zip(mask.iter()).map(|(&a, &b)| a ^ b).collect();
    db[0] &= top_mask;

    let ps_len = em_len - HLEN - SLEN - 2;
    if !db[..ps_len].iter().all(|&b| b == 0) || db[ps_len] != 0x01 {
        return false;
    }
    let salt = &db[db.len() - SLEN..];
    let m_hash = sha256(msg);
    let mut m_prime = [0u8; 8 + HLEN + SLEN];
    m_prime[8..8 + HLEN].copy_from_slice(&m_hash);
    m_prime[8 + HLEN..].copy_from_slice(salt);
    let h_prime = sha256(&m_prime);
    ct_eq(h_slice, &h_prime)
}
