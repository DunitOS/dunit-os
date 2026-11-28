//! Kernel ChaCha20 CSPRNG fed by CPU hardware entropy (RDSEED/RDRAND) and timing jitter.
use crate::sync::IrqSafeSpinLock;
use core::sync::atomic::{compiler_fence, Ordering};

const CHACHA_SIGMA: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];
const RESEED_BYTES: usize = 64 * 1024;

#[inline]
fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags));
    }
    (u64::from(hi) << 32) | u64::from(lo)
}

#[cfg(not(test))]
fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let eax: u32;
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    unsafe {
        core::arch::asm!(
            "xchg rbx, {tmp:r}",
            "cpuid",
            "xchg rbx, {tmp:r}",
            tmp = out(reg) ebx,
            inout("eax") leaf => eax,
            inout("ecx") subleaf => ecx,
            out("edx") edx,
            options(nomem, nostack, preserves_flags)
        );
    }
    (eax, ebx, ecx, edx)
}

#[cfg(test)]
fn cpuid(_leaf: u32, _subleaf: u32) -> (u32, u32, u32, u32) {
    (0, 0, 0, 0)
}

pub fn hardware_features() -> (bool, bool) {
    let (max_leaf, _, _, _) = cpuid(0, 0);
    let rdrand = if max_leaf >= 1 {
        let (_, _, ecx, _) = cpuid(1, 0);
        (ecx & (1 << 30)) != 0
    } else {
        false
    };
    let rdseed = if max_leaf >= 7 {
        let (_, ebx, _, _) = cpuid(7, 0);
        (ebx & (1 << 18)) != 0
    } else {
        false
    };
    (rdseed, rdrand)
}

#[cfg(not(test))]
fn hw_rand64(has_rdseed: bool, has_rdrand: bool) -> Option<u64> {
    if has_rdseed {
        for _ in 0..8 {
            let mut val: u64;
            let ok: u8;
            unsafe {
                core::arch::asm!(
                    "rdseed {v}",
                    "setc {ok}",
                    v = out(reg) val,
                    ok = out(reg_byte) ok,
                    options(nomem, nostack)
                );
            }
            if ok != 0 {
                return Some(val);
            }
        }
    }
    if has_rdrand {
        for _ in 0..10 {
            let mut val: u64;
            let ok: u8;
            unsafe {
                core::arch::asm!(
                    "rdrand {v}",
                    "setc {ok}",
                    v = out(reg) val,
                    ok = out(reg_byte) ok,
                    options(nomem, nostack)
                );
            }
            if ok != 0 {
                return Some(val);
            }
        }
    }
    None
}

#[cfg(test)]
fn hw_rand64(_has_rdseed: bool, _has_rdrand: bool) -> Option<u64> {
    None
}

fn zeroize(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    compiler_fence(Ordering::SeqCst);
}

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

pub(crate) fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut state = [0u32; 16];
    state[0..4].copy_from_slice(&CHACHA_SIGMA);
    for i in 0..8 {
        state[4 + i] = u32::from_le_bytes(key[4 * i..4 * (i + 1)].try_into().unwrap());
    }
    state[12] = counter;
    for i in 0..3 {
        state[13 + i] = u32::from_le_bytes(nonce[4 * i..4 * (i + 1)].try_into().unwrap());
    }
    let init = state;
    for _ in 0..10 {
        qr(&mut state, 0, 4, 8, 12);
        qr(&mut state, 1, 5, 9, 13);
        qr(&mut state, 2, 6, 10, 14);
        qr(&mut state, 3, 7, 11, 15);
        qr(&mut state, 0, 5, 10, 15);
        qr(&mut state, 1, 6, 11, 12);
        qr(&mut state, 2, 7, 8, 13);
        qr(&mut state, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        let word = state[i].wrapping_add(init[i]).to_le_bytes();
        out[4 * i..4 * (i + 1)].copy_from_slice(&word);
    }
    out
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let total = (data.len() + 9).div_ceil(64) * 64;
    let mut block_idx = 0usize;
    while block_idx < total {
        let mut blk = [0u8; 64];
        for i in 0..64 {
            let off = block_idx + i;
            if off < data.len() {
                blk[i] = data[off];
            } else if off == data.len() {
                blk[i] = 0x80;
            } else if off >= total - 8 {
                blk[i] = bit_len.to_be_bytes()[off - (total - 8)];
            }
        }
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(blk[4 * i..4 * (i + 1)].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        let vals = [a, b, c, d, e, f, g, hh];
        for i in 0..8 {
            h[i] = h[i].wrapping_add(vals[i]);
        }
        block_idx += 64;
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[4 * i..4 * (i + 1)].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

struct Csprng {
    key: [u8; 32],
    generation: u64,
    bytes_since_reseed: usize,
    seeded: bool,
    rdseed: bool,
    rdrand: bool,
    jitter: u64,
}

impl Csprng {
    const fn new() -> Self {
        Self {
            key: [0u8; 32],
            generation: 0,
            bytes_since_reseed: 0,
            seeded: false,
            rdseed: false,
            rdrand: false,
            jitter: 0,
        }
    }

    fn mix(&mut self, samples: &[u64]) {
        let mut msg = [0u8; 96];
        msg[..32].copy_from_slice(&self.key);
        msg[32..40].copy_from_slice(&self.generation.to_le_bytes());
        msg[40..48].copy_from_slice(&rdtsc().to_le_bytes());
        let mut off = 48;
        for &val in samples {
            if off + 8 > msg.len() {
                break;
            }
            msg[off..off + 8].copy_from_slice(&val.to_le_bytes());
            off += 8;
        }
        self.key = sha256(&msg[..off]);
        zeroize(&mut msg);
        self.generation = self.generation.wrapping_add(1);
    }

    fn mix_bytes(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(48) {
            let mut words = [0u64; 6];
            let mut count = 0usize;
            for slice in chunk.chunks(8) {
                let mut raw = [0u8; 8];
                raw[..slice.len()].copy_from_slice(slice);
                words[count] = u64::from_le_bytes(raw);
                count += 1;
            }
            self.mix(&words[..count]);
        }
    }

    fn ensure_seeded(&mut self) {
        if self.seeded && self.bytes_since_reseed < RESEED_BYTES {
            return;
        }
        let (rdseed, rdrand) = hardware_features();
        self.rdseed = rdseed;
        self.rdrand = rdrand;

        let mut samples = [0u64; 6];
        for round in 0..4u64 {
            for slot in &mut samples {
                *slot = if let Some(hw) = hw_rand64(rdseed, rdrand) {
                    hw ^ rdtsc()
                } else {
                    let t0 = rdtsc();
                    let ns = crate::clock::monotonic_ns();
                    let epoch = crate::clock::realtime().map_or(0, |(s, n)| s ^ n);
                    let stack_addr = &raw const *slot as u64;
                    t0.rotate_left(13) ^ ns ^ epoch ^ stack_addr ^ self.jitter ^ round
                };
            }
            self.mix(&samples);
        }
        self.bytes_since_reseed = 0;
        self.seeded = true;
    }

    fn fill(&mut self, out: &mut [u8]) {
        self.ensure_seeded();
        // Mix fresh per-call timing + hardware sample before output.
        let sample = hw_rand64(self.rdseed, self.rdrand).unwrap_or(0) ^ self.jitter;
        self.mix(&[crate::clock::monotonic_ns(), sample, out.len() as u64]);

        let nonce_bytes = {
            let mut n = [0u8; 12];
            n[..8].copy_from_slice(&self.generation.to_le_bytes());
            n[8..12].copy_from_slice(&(out.len() as u32).to_le_bytes());
            n
        };

        let mut pos = 0usize;
        let mut counter = 1u32;
        while pos < out.len() {
            let mut blk = chacha20_block(&self.key, counter, &nonce_bytes);
            let take = (out.len() - pos).min(64);
            out[pos..pos + take].copy_from_slice(&blk[..take]);
            zeroize(&mut blk);
            pos += take;
            counter = counter.wrapping_add(1);
        }

        // Fast-key-erasure: replace key immediately after output for backtracking resistance.
        let mut rekey = chacha20_block(&self.key, 0, &nonce_bytes);
        self.key.copy_from_slice(&rekey[..32]);
        zeroize(&mut rekey);
        self.bytes_since_reseed = self.bytes_since_reseed.saturating_add(out.len());
    }
}

static RNG: IrqSafeSpinLock<Csprng> = IrqSafeSpinLock::new(Csprng::new());

pub fn init() {
    let mut rng = RNG.lock();
    rng.ensure_seeded();
    let mut line = crate::FmtBuf::new();
    line.push_str("[ENTROPY] csprng=chacha20 rdseed=");
    line.push_u64(u64::from(rng.rdseed));
    line.push_str(" rdrand=");
    line.push_u64(u64::from(rng.rdrand));
    line.push_str("\r\n");
    crate::memory::serial_write(line.as_str());
}

#[inline]
pub fn mix_irq_jitter(source_tag: u64) {
    if let Some(mut rng) = RNG.try_lock() {
        rng.jitter = rng.jitter.rotate_left(19) ^ rdtsc() ^ source_tag;
    }
}

pub fn fill_bytes(out: &mut [u8]) {
    if out.is_empty() {
        return;
    }
    let mut rng = RNG.lock();
    let needs_reseed = !rng.seeded || rng.bytes_since_reseed >= RESEED_BYTES;
    if needs_reseed {
        drop(rng);
        let mut hw = [0u8; 32];
        let got = crate::drivers::virtio_rng::try_fill_bytes(&mut hw);
        let mut rng = RNG.lock();
        if got > 0 {
            rng.mix_bytes(&hw[..got]);
        }
        zeroize(&mut hw);
        rng.fill(out);
        return;
    }
    rng.fill(out);
}

/// Mix hardware entropy bytes (e.g. VirtIO-RNG) into the active CSPRNG pool.
pub fn mix_hardware_bytes(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let mut rng = RNG.lock();
    rng.ensure_seeded();
    rng.mix_bytes(bytes);
    rng.bytes_since_reseed = 0;
}

