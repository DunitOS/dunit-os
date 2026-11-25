//! RFC 7748 X25519 Diffie-Hellman key exchange over Curve25519 (5x51-bit limbs).

const MASK51: u64 = (1u64 << 51) - 1;

#[derive(Clone, Copy)]
struct Fe([u64; 5]);

impl Fe {
    const ZERO: Self = Self([0; 5]);
    const ONE: Self = Self([1, 0, 0, 0, 0]);
    const A24: Self = Self([121_665, 0, 0, 0, 0]);

    fn from_bytes(b: &[u8; 32]) -> Self {
        let load64 = |off: usize| u64::from_le_bytes(b[off..off + 8].try_into().unwrap());
        let l0 = load64(0) & MASK51;
        let l1 = ((load64(6) >> 3)) & MASK51;
        let l2 = ((load64(12) >> 6)) & MASK51;
        let l3 = ((load64(19) >> 1)) & MASK51;
        let l4 = (load64(24) >> 12) & MASK51;
        Self([l0, l1, l2, l3, l4])
    }

    fn to_bytes(self) -> [u8; 32] {
        let mut t = self.0;
        let mut c: u64;
        for _ in 0..2 {
            c = t[0] >> 51; t[0] &= MASK51; t[1] += c;
            c = t[1] >> 51; t[1] &= MASK51; t[2] += c;
            c = t[2] >> 51; t[2] &= MASK51; t[3] += c;
            c = t[3] >> 51; t[3] &= MASK51; t[4] += c;
            c = t[4] >> 51; t[4] &= MASK51; t[0] += c * 19;
        }
        // Conditional subtract p = 2^255 - 19 = [2^51-19, 2^51-1, 2^51-1, 2^51-1, 2^51-1].
        let mut q = (t[0] + 19) >> 51;
        q = (t[1] + q) >> 51;
        q = (t[2] + q) >> 51;
        q = (t[3] + q) >> 51;
        q = (t[4] + q) >> 51;

        t[0] += 19 * q;
        c = t[0] >> 51; t[0] &= MASK51; t[1] += c;
        c = t[1] >> 51; t[1] &= MASK51; t[2] += c;
        c = t[2] >> 51; t[2] &= MASK51; t[3] += c;
        c = t[3] >> 51; t[3] &= MASK51; t[4] += c;
        t[4] &= MASK51;

        let mut out = [0u8; 32];
        let w0 = t[0] | (t[1] << 51);
        let w1 = (t[1] >> 13) | (t[2] << 38);
        let w2 = (t[2] >> 26) | (t[3] << 25);
        let w3 = (t[3] >> 39) | (t[4] << 12);
        out[0..8].copy_from_slice(&w0.to_le_bytes());
        out[8..16].copy_from_slice(&w1.to_le_bytes());
        out[16..24].copy_from_slice(&w2.to_le_bytes());
        out[24..32].copy_from_slice(&w3.to_le_bytes());
        out
    }

    fn add(self, o: Self) -> Self {
        Self([
            self.0[0] + o.0[0],
            self.0[1] + o.0[1],
            self.0[2] + o.0[2],
            self.0[3] + o.0[3],
            self.0[4] + o.0[4],
        ])
    }

    fn sub(self, o: Self) -> Self {
        // Add 4*p before subtracting so each limb stays positive cleanly.
        Self([
            self.0[0] + 4 * ((1u64 << 51) - 19) - o.0[0],
            self.0[1] + 4 * MASK51 - o.0[1],
            self.0[2] + 4 * MASK51 - o.0[2],
            self.0[3] + 4 * MASK51 - o.0[3],
            self.0[4] + 4 * MASK51 - o.0[4],
        ])
    }

    fn mul(self, o: Self) -> Self {
        let a = self.0.map(u128::from);
        let b = o.0.map(u128::from);
        let b1_19 = b[1] * 19;
        let b2_19 = b[2] * 19;
        let b3_19 = b[3] * 19;
        let b4_19 = b[4] * 19;

        let mut t0 = a[0] * b[0] + a[1] * b4_19 + a[2] * b3_19 + a[3] * b2_19 + a[4] * b1_19;
        let mut t1 = a[0] * b[1] + a[1] * b[0] + a[2] * b4_19 + a[3] * b3_19 + a[4] * b2_19;
        let mut t2 = a[0] * b[2] + a[1] * b[1] + a[2] * b[0] + a[3] * b4_19 + a[4] * b3_19;
        let mut t3 = a[0] * b[3] + a[1] * b[2] + a[2] * b[1] + a[3] * b[0] + a[4] * b4_19;
        let mut t4 = a[0] * b[4] + a[1] * b[3] + a[2] * b[2] + a[3] * b[1] + a[4] * b[0];

        let mut c = t0 >> 51;
        t0 &= u128::from(MASK51);
        t1 += c;
        c = t1 >> 51;
        t1 &= u128::from(MASK51);
        t2 += c;
        c = t2 >> 51;
        t2 &= u128::from(MASK51);
        t3 += c;
        c = t3 >> 51;
        t3 &= u128::from(MASK51);
        t4 += c;
        c = t4 >> 51;
        t4 &= u128::from(MASK51);
        t0 += c * 19;
        c = t0 >> 51;
        t0 &= u128::from(MASK51);
        t1 += c;

        Self([t0 as u64, t1 as u64, t2 as u64, t3 as u64, t4 as u64])
    }

    fn sq(self) -> Self {
        self.mul(self)
    }

    fn invert(self) -> Self {
        // p - 2 = 2^255 - 21
        let z2 = self.sq();
        let z9 = z2.sq().sq().mul(self);
        let z11 = z9.mul(z2);
        let z2_5_0 = z11.sq().mul(z9);
        let mut t = z2_5_0;
        for _ in 0..5 {
            t = t.sq();
        }
        let z2_10_0 = t.mul(z2_5_0);
        t = z2_10_0;
        for _ in 0..10 {
            t = t.sq();
        }
        let z2_20_0 = t.mul(z2_10_0);
        t = z2_20_0;
        for _ in 0..20 {
            t = t.sq();
        }
        let z2_40_0 = t.mul(z2_20_0);
        t = z2_40_0;
        for _ in 0..10 {
            t = t.sq();
        }
        let z2_50_0 = t.mul(z2_10_0);
        t = z2_50_0;
        for _ in 0..50 {
            t = t.sq();
        }
        let z2_100_0 = t.mul(z2_50_0);
        t = z2_100_0;
        for _ in 0..100 {
            t = t.sq();
        }
        t = t.mul(z2_100_0);
        for _ in 0..50 {
            t = t.sq();
        }
        t = t.mul(z2_50_0);
        for _ in 0..5 {
            t = t.sq();
        }
        t.mul(z11)
    }
}

fn cswap(swap: u64, a: &mut Fe, b: &mut Fe) {
    let mask = 0u64.wrapping_sub(swap);
    for i in 0..5 {
        let d = mask & (a.0[i] ^ b.0[i]);
        a.0[i] ^= d;
        b.0[i] ^= d;
    }
}

pub fn x25519(scalar: &[u8; 32], u_coord: &[u8; 32]) -> [u8; 32] {
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    let x1 = Fe::from_bytes(u_coord);
    let mut x2 = Fe::ONE;
    let mut z2 = Fe::ZERO;
    let mut x3 = x1;
    let mut z3 = Fe::ONE;
    let mut swap = 0u64;

    for t in (0..255).rev() {
        let kt = u64::from((k[t >> 3] >> (t & 7)) & 1);
        swap ^= kt;
        cswap(swap, &mut x2, &mut x3);
        cswap(swap, &mut z2, &mut z3);
        swap = kt;

        let a = x2.add(z2);
        let aa = a.sq();
        let b = x2.sub(z2);
        let bb = b.sq();
        let e = aa.sub(bb);
        let c = x3.add(z3);
        let d = x3.sub(z3);
        let da = d.mul(a);
        let cb = c.mul(b);
        x3 = da.add(cb).sq();
        z3 = x1.mul(da.sub(cb).sq());
        x2 = aa.mul(bb);
        z2 = e.mul(aa.add(Fe::A24.mul(e)));
    }
    cswap(swap, &mut x2, &mut x3);
    cswap(swap, &mut z2, &mut z3);

    x2.mul(z2.invert()).to_bytes()
}

pub fn x25519_base(scalar: &[u8; 32]) -> [u8; 32] {
    let mut base = [0u8; 32];
    base[0] = 9;
    x25519(scalar, &base)
}
