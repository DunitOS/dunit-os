//! NIST P-256 (secp256r1) ECDSA signature verification (SHA-256).

#[derive(Clone, Copy, PartialEq, Eq)]
struct U256([u64; 4]);

const P: U256 = U256([
    0xffff_ffff_ffff_ffff,
    0x0000_0000_ffff_ffff,
    0x0000_0000_0000_0000,
    0xffff_ffff_0000_0001,
]);

const N: U256 = U256([
    0xf3b9_cac2_fc63_2551,
    0xbce6_faad_a717_9e84,
    0xffff_ffff_ffff_ffff,
    0xffff_ffff_0000_0000,
]);

const B_COEFF: U256 = U256([
    0x3bce_3c3e_27d2_604b,
    0x651d_06b0_cc53_b0f6,
    0xb3eb_bd55_7698_86bc,
    0x5ac6_35d8_aa3a_93e7,
]);

const GX: U256 = U256([
    0xf4a1_3945_d898_c296,
    0x7703_7d81_2deb_33a0,
    0xf8bc_e6e5_63a4_40f2,
    0x6b17_d1f2_e12c_4247,
]);

const GY: U256 = U256([
    0xcbb6_4068_37bf_51f5,
    0x2bce_3357_6b31_5ece,
    0x8ee7_eb4a_7c0f_9e16,
    0x4fe3_42e2_fe1a_7f9b,
]);

impl U256 {
    const ZERO: Self = Self([0; 4]);
    const ONE: Self = Self([1, 0, 0, 0]);

    fn from_be_bytes(bytes: &[u8; 32]) -> Self {
        Self([
            u64::from_be_bytes(bytes[24..32].try_into().unwrap()),
            u64::from_be_bytes(bytes[16..24].try_into().unwrap()),
            u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
            u64::from_be_bytes(bytes[0..8].try_into().unwrap()),
        ])
    }

    fn is_zero(self) -> bool {
        self.0 == [0; 4]
    }

    fn cmp_ge(self, o: Self) -> bool {
        for i in (0..4).rev() {
            if self.0[i] != o.0[i] {
                return self.0[i] > o.0[i];
            }
        }
        true
    }

    fn add_raw(self, o: Self) -> (Self, bool) {
        let mut out = [0u64; 4];
        let mut c = 0u128;
        for i in 0..4 {
            let s = u128::from(self.0[i]) + u128::from(o.0[i]) + c;
            out[i] = s as u64;
            c = s >> 64;
        }
        (Self(out), c != 0)
    }

    fn sub_wrapping(self, o: Self) -> (Self, bool) {
        let mut out = [0u64; 4];
        let mut borrow = 0i128;
        for i in 0..4 {
            let d = i128::from(self.0[i]) - i128::from(o.0[i]) - borrow;
            out[i] = d as u64;
            borrow = if d < 0 { 1 } else { 0 };
        }
        (Self(out), borrow != 0)
    }

    fn bit(&self, idx: usize) -> bool {
        ((self.0[idx >> 6] >> (idx & 63)) & 1) != 0
    }
}

fn add_mod(a: U256, b: U256, m: U256) -> U256 {
    let (sum, carry) = a.add_raw(b);
    if carry || sum.cmp_ge(m) {
        sum.sub_wrapping(m).0
    } else {
        sum
    }
}

fn sub_mod(a: U256, b: U256, m: U256) -> U256 {
    let (diff, borrow) = a.sub_wrapping(b);
    if borrow {
        diff.add_raw(m).0
    } else {
        diff
    }
}

fn mul_mod(a: U256, b: U256, m: U256) -> U256 {
    let mut prod = [0u64; 8];
    for i in 0..4 {
        let mut c = 0u128;
        for j in 0..4 {
            let v = u128::from(a.0[i]) * u128::from(b.0[j]) + u128::from(prod[i + j]) + c;
            prod[i + j] = v as u64;
            c = v >> 64;
        }
        prod[i + 4] = c as u64;
    }
    let mut rem = U256::ZERO;
    for i in (0..512).rev() {
        let bit = (prod[i >> 6] >> (i & 63)) & 1;
        let carry = rem.0[3] >> 63;
        rem.0[3] = (rem.0[3] << 1) | (rem.0[2] >> 63);
        rem.0[2] = (rem.0[2] << 1) | (rem.0[1] >> 63);
        rem.0[1] = (rem.0[1] << 1) | (rem.0[0] >> 63);
        rem.0[0] = (rem.0[0] << 1) | bit;
        if carry != 0 || rem.cmp_ge(m) {
            rem = rem.sub_wrapping(m).0;
        }
    }
    rem
}

fn inv_mod(a: U256, m: U256) -> Option<U256> {
    if a.is_zero() {
        return None;
    }
    let exp = m.sub_wrapping(U256([2, 0, 0, 0])).0;
    let mut base = a;
    let mut res = U256::ONE;
    for i in 0..256 {
        if exp.bit(i) {
            res = mul_mod(res, base, m);
        }
        base = mul_mod(base, base, m);
    }
    Some(res)
}

#[derive(Clone, Copy)]
struct Point {
    x: U256,
    y: U256,
    z: U256,
}

impl Point {
    const INFINITY: Self = Self {
        x: U256::ZERO,
        y: U256::ONE,
        z: U256::ZERO,
    };

    fn from_affine(x: U256, y: U256) -> Self {
        Self { x, y, z: U256::ONE }
    }

    fn is_inf(&self) -> bool {
        self.z.is_zero()
    }
}

fn p_add(a: U256, b: U256) -> U256 {
    add_mod(a, b, P)
}
fn p_sub(a: U256, b: U256) -> U256 {
    sub_mod(a, b, P)
}
fn p_mul(a: U256, b: U256) -> U256 {
    mul_mod(a, b, P)
}

fn point_double(pt: Point) -> Point {
    if pt.is_inf() || pt.y.is_zero() {
        return Point::INFINITY;
    }
    let delta = p_mul(pt.z, pt.z);
    let gamma = p_mul(pt.y, pt.y);
    let beta = p_mul(pt.x, gamma);
    let xp = p_add(pt.x, delta);
    let xm = p_sub(pt.x, delta);
    let xp_xm = p_mul(xm, xp);
    let alpha = p_add(p_add(xp_xm, xp_xm), xp_xm); // 3 * (X - Z^2)(X + Z^2)

    let b4 = p_add(p_add(beta, beta), p_add(beta, beta));
    let b8 = p_add(b4, b4);
    let x3 = p_sub(p_mul(alpha, alpha), b8);

    let ypz = p_add(pt.y, pt.z);
    let z3 = p_sub(p_sub(p_mul(ypz, ypz), gamma), delta);

    let g_sq = p_mul(gamma, gamma);
    let g2 = p_add(g_sq, g_sq);
    let g4 = p_add(g2, g2);
    let g8 = p_add(g4, g4);
    let y3 = p_sub(p_mul(alpha, p_sub(b4, x3)), g8);

    Point { x: x3, y: y3, z: z3 }
}

fn point_add(p1: Point, p2: Point) -> Point {
    if p1.is_inf() {
        return p2;
    }
    if p2.is_inf() {
        return p1;
    }
    let z1z1 = p_mul(p1.z, p1.z);
    let z2z2 = p_mul(p2.z, p2.z);
    let u1 = p_mul(p1.x, z2z2);
    let u2 = p_mul(p2.x, z1z1);
    let s1 = p_mul(p1.y, p_mul(p2.z, z2z2));
    let s2 = p_mul(p2.y, p_mul(p1.z, z1z1));

    let h = p_sub(u2, u1);
    let r = p_sub(s2, s1);
    if h.is_zero() {
        return if r.is_zero() {
            point_double(p1)
        } else {
            Point::INFINITY
        };
    }
    let hh = p_mul(h, h);
    let hhh = p_mul(hh, h);
    let v = p_mul(u1, hh);
    let v2 = p_add(v, v);
    let x3 = p_sub(p_sub(p_mul(r, r), hhh), v2);
    let y3 = p_sub(p_mul(r, p_sub(v, x3)), p_mul(s1, hhh));
    let z3 = p_mul(h, p_mul(p1.z, p2.z));
    Point { x: x3, y: y3, z: z3 }
}

fn scalar_mul(k: U256, base: Point) -> Point {
    let mut acc = Point::INFINITY;
    for i in (0..256).rev() {
        acc = point_double(acc);
        if k.bit(i) {
            acc = point_add(acc, base);
        }
    }
    acc
}

pub fn verify_p256_sha256(pubkey_sec1: &[u8], msg_hash: &[u8; 32], r_bytes: &[u8], s_bytes: &[u8]) -> bool {
    verify_p256_step(pubkey_sec1, msg_hash, r_bytes, s_bytes).is_ok()
}

#[cfg(test)]
pub(crate) fn debug_self_check() -> Result<(), &'static str> {
    let g = Point::from_affine(GX, GY);
    if !scalar_mul(N, g).is_inf() {
        return Err("N*G != inf");
    }
    Ok(())
}

pub fn verify_p256_step(pubkey_sec1: &[u8], msg_hash: &[u8; 32], r_bytes: &[u8], s_bytes: &[u8]) -> Result<(), &'static str> {
    if pubkey_sec1.len() != 65 || pubkey_sec1[0] != 0x04 {
        return Err("bad pubkey len");
    }
    if r_bytes.len() > 32 || s_bytes.len() > 32 || r_bytes.is_empty() || s_bytes.is_empty() {
        return Err("bad r/s len");
    }
    let mut r_pad = [0u8; 32];
    let mut s_pad = [0u8; 32];
    r_pad[32 - r_bytes.len()..].copy_from_slice(r_bytes);
    s_pad[32 - s_bytes.len()..].copy_from_slice(s_bytes);

    let r = U256::from_be_bytes(&r_pad);
    let s = U256::from_be_bytes(&s_pad);
    if r.is_zero() || s.is_zero() || r.cmp_ge(N) || s.cmp_ge(N) {
        return Err("r/s out of range");
    }

    let qx = U256::from_be_bytes(pubkey_sec1[1..33].try_into().unwrap());
    let qy = U256::from_be_bytes(pubkey_sec1[33..65].try_into().unwrap());
    if qx.cmp_ge(P) || qy.cmp_ge(P) {
        return Err("q out of field");
    }
    let y2 = p_mul(qy, qy);
    let x3 = p_mul(p_mul(qx, qx), qx);
    let three_x = p_add(p_add(qx, qx), qx);
    let rhs = p_add(p_sub(x3, three_x), B_COEFF);
    if y2 != rhs {
        return Err("not on curve");
    }

    let s_inv = inv_mod(s, N).ok_or("no inv")?;
    let mut z = U256::from_be_bytes(msg_hash);
    if z.cmp_ge(N) {
        z = z.sub_wrapping(N).0;
    }
    let u1 = mul_mod(z, s_inv, N);
    let u2 = mul_mod(r, s_inv, N);

    let g = Point::from_affine(GX, GY);
    let q = Point::from_affine(qx, qy);
    let res = point_add(scalar_mul(u1, g), scalar_mul(u2, q));
    if res.is_inf() {
        return Err("inf");
    }
    let z_inv = inv_mod(res.z, P).ok_or("no z_inv")?;
    let rx = p_mul(res.x, p_mul(z_inv, z_inv));
    let rx_mod_n = if rx.cmp_ge(N) { rx.sub_wrapping(N).0 } else { rx };
    if rx_mod_n == r { Ok(()) } else { Err("rx != r") }
}
