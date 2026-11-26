//! Bounded Baseline Sequential JPEG (SOF0/SOF1) decoder -> ARGB8888 (integer IDCT).
use alloc::{vec, vec::Vec};

use super::check_dimensions;
use crate::{Error, Image};

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Clone, Copy)]
struct HuffTable {
    mincode: [i32; 17],
    maxcode: [i32; 17],
    valptr: [i32; 17],
    values: [u8; 256],
    valid: bool,
}

impl Default for HuffTable {
    fn default() -> Self {
        Self {
            mincode: [0; 17],
            maxcode: [-1; 17],
            valptr: [0; 17],
            values: [0; 256],
            valid: false,
        }
    }
}

impl HuffTable {
    fn build(counts: &[u8; 16], symbols: &[u8]) -> Result<Self, Error> {
        let total: usize = counts.iter().map(|&c| c as usize).sum();
        if total > 256 || symbols.len() != total {
            return Err(Error::Format);
        }
        let mut table = Self::default();
        table.values[..total].copy_from_slice(symbols);
        let mut code = 0i32;
        let mut si = 0usize;
        for bits in 1..=16usize {
            let count = counts[bits - 1] as usize;
            if count > 0 {
                table.valptr[bits] = si as i32;
                table.mincode[bits] = code;
                code += count as i32;
                table.maxcode[bits] = code - 1;
                si += count;
            } else {
                table.maxcode[bits] = -1;
            }
            code <<= 1;
        }
        table.valid = true;
        Ok(table)
    }
}

#[derive(Clone, Copy, Default)]
struct Component {
    id: u8,
    hsamp: usize,
    vsamp: usize,
    qtable: usize,
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bits_left: u8,
    pending_marker: Option<u8>,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self {
            data,
            pos,
            bit_buf: 0,
            bits_left: 0,
            pending_marker: None,
        }
    }

    fn reset_restart(&mut self, next_pos: usize) {
        self.pos = next_pos;
        self.bit_buf = 0;
        self.bits_left = 0;
        self.pending_marker = None;
    }

    fn next_entropy_byte(&mut self) -> Result<u8, Error> {
        while self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            if b != 0xFF {
                return Ok(b);
            }
            while self.pos < self.data.len() && self.data[self.pos] == 0xFF {
                self.pos += 1;
            }
            if self.pos >= self.data.len() {
                return Err(Error::Format);
            }
            let marker = self.data[self.pos];
            self.pos += 1;
            if marker == 0x00 {
                return Ok(0xFF);
            }
            if (0xD0..=0xD7).contains(&marker) || marker == 0xD9 {
                self.pending_marker = Some(marker);
                return Ok(0);
            }
            return Err(Error::Format);
        }
        Err(Error::Format)
    }

    fn ensure(&mut self, n: u8) -> Result<(), Error> {
        while self.bits_left < n {
            let byte = self.next_entropy_byte()? as u32;
            self.bit_buf = (self.bit_buf << 8) | byte;
            self.bits_left += 8;
        }
        Ok(())
    }

    fn read_bits(&mut self, n: u8) -> Result<u16, Error> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure(n)?;
        let shift = self.bits_left - n;
        let val = (self.bit_buf >> shift) & ((1u32 << n) - 1);
        self.bits_left -= n;
        self.bit_buf &= (1u32 << self.bits_left) - 1;
        Ok(val as u16)
    }

    fn decode_huff(&mut self, table: &HuffTable) -> Result<u8, Error> {
        if !table.valid {
            return Err(Error::Format);
        }
        let mut code = 0i32;
        for bits in 1..=16usize {
            let bit = self.read_bits(1)? as i32;
            code = (code << 1) | bit;
            if table.maxcode[bits] >= 0 && code <= table.maxcode[bits] {
                let idx = (table.valptr[bits] + (code - table.mincode[bits])) as usize;
                return table.values.get(idx).copied().ok_or(Error::Format);
            }
        }
        Err(Error::Format)
    }

    fn receive_extend(&mut self, nbits: u8) -> Result<i32, Error> {
        if nbits == 0 {
            return Ok(0);
        }
        if nbits > 15 {
            return Err(Error::Format);
        }
        let v = self.read_bits(nbits)? as i32;
        let vt = 1i32 << (nbits - 1);
        if v < vt {
            Ok(v + ((-1i32) << nbits) + 1)
        } else {
            Ok(v)
        }
    }
}

fn idct_1d(s0: i32, s1: i32, s2: i32, s3: i32, s4: i32, s5: i32, s6: i32, s7: i32) -> [i32; 8] {
    // Loeffler / stb_image style fixed-point scaled 1D IDCT (12-bit fractional weights).
    #[inline]
    fn f2f(x: i64) -> i32 {
        x as i32
    }
    let p2 = s2;
    let p3 = s6;
    let p1 = (p2 + p3) * f2f(2217); // 0.5411961 * 4096
    let t2 = p1 + p3 * f2f(-7567); // -1.847759065 * 4096
    let t3 = p1 + p2 * f2f(3135); // 0.765366865 * 4096
    let p2 = s0;
    let p3 = s4;
    let t0 = (p2 + p3) << 12;
    let t1 = (p2 - p3) << 12;

    let x0 = t0 + t3;
    let x3 = t0 - t3;
    let x1 = t1 + t2;
    let x2 = t1 - t2;

    let mut t0 = s7;
    let mut t1 = s5;
    let mut t2 = s3;
    let mut t3 = s1;
    let p3 = t0 + t2;
    let p4 = t1 + t3;
    let p1 = t0 + t3;
    let p2 = t1 + t2;
    let p5 = (p3 + p4) * f2f(4816); // 1.175875602 * 4096
    t0 *= f2f(1223); // 0.298631336 * 4096
    t1 *= f2f(8410); // 2.053119869 * 4096
    t2 *= f2f(12586); // 3.072711026 * 4096
    t3 *= f2f(6149); // 1.501321110 * 4096
    let p1 = p5 + p1 * f2f(-3685); // -0.899976223 * 4096
    let p2 = p5 + p2 * f2f(-10497); // -2.562915447 * 4096
    let p3 = p3 * f2f(-8034); // -1.961570560 * 4096
    let p4 = p4 * f2f(-1597); // -0.390180644 * 4096
    t3 += p1 + p4;
    t2 += p2 + p3;
    t1 += p2 + p4;
    t0 += p1 + p3;

    [
        x0 + t3,
        x1 + t2,
        x2 + t1,
        x3 + t0,
        x3 - t0,
        x2 - t1,
        x1 - t2,
        x0 - t3,
    ]
}

fn idct_block(coeffs: &[i32; 64], quant: &[u16; 64]) -> [u8; 64] {
    let mut work = [0i32; 64];
    for i in 0..8 {
        let mut r = [0i32; 8];
        for j in 0..8 {
            let idx = i + j * 8;
            r[j] = coeffs[idx].wrapping_mul(quant[idx] as i32);
        }
        // Standard row pass (indices i*8 + 0..7)
        let row = [
            coeffs[i * 8] * quant[i * 8] as i32,
            coeffs[i * 8 + 1] * quant[i * 8 + 1] as i32,
            coeffs[i * 8 + 2] * quant[i * 8 + 2] as i32,
            coeffs[i * 8 + 3] * quant[i * 8 + 3] as i32,
            coeffs[i * 8 + 4] * quant[i * 8 + 4] as i32,
            coeffs[i * 8 + 5] * quant[i * 8 + 5] as i32,
            coeffs[i * 8 + 6] * quant[i * 8 + 6] as i32,
            coeffs[i * 8 + 7] * quant[i * 8 + 7] as i32,
        ];
        let mut out = idct_1d(row[0], row[1], row[2], row[3], row[4], row[5], row[6], row[7]);
        for j in 0..8 {
            work[i * 8 + j] = (out[j] + 512) >> 10;
        }
        let _ = r;
        let _ = &mut out;
    }

    let mut pixels = [0u8; 64];
    for x in 0..8 {
        let col = idct_1d(
            work[x],
            work[8 + x],
            work[16 + x],
            work[24 + x],
            work[32 + x],
            work[40 + x],
            work[48 + x],
            work[56 + x],
        );
        for y in 0..8 {
            let sample = ((col[y] + 65536 + (128 << 17)) >> 17).clamp(0, 255);
            pixels[y * 8 + x] = sample as u8;
        }
    }
    pixels
}

fn clamp_u8(v: i32) -> u32 {
    v.clamp(0, 255) as u32
}

fn ycbcr_to_argb(y: u8, cb: u8, cr: u8) -> u32 {
    let y = y as i32;
    let cb = cb as i32 - 128;
    let cr = cr as i32 - 128;
    let r = clamp_u8(y + ((91881 * cr + 32768) >> 16));
    let g = clamp_u8(y - ((22554 * cb + 46802 * cr + 32768) >> 16));
    let b = clamp_u8(y + ((116130 * cb + 32768) >> 16));
    0xFF00_0000 | (r << 16) | (g << 8) | b
}

pub fn decode_jpeg(data: &[u8]) -> Result<Image, Error> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(Error::Format);
    }
    let mut pos = 2usize;
    let mut quants = [[0u16; 64]; 4];
    let mut quant_valid = [false; 4];
    let mut dc_huff = [HuffTable::default(); 4];
    let mut ac_huff = [HuffTable::default(); 4];
    let mut restart_interval = 0usize;

    let mut width = 0usize;
    let mut height = 0usize;
    let mut comps: Vec<Component> = Vec::new();

    while pos + 2 <= data.len() {
        while pos < data.len() && data[pos] == 0xFF {
            pos += 1;
        }
        if pos >= data.len() {
            return Err(Error::Format);
        }
        let marker = data[pos];
        pos += 1;
        if marker == 0xD9 {
            break;
        }
        if marker == 0x00 || (0xD0..=0xD8).contains(&marker) {
            continue;
        }
        if pos + 2 > data.len() {
            return Err(Error::Format);
        }
        let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > data.len() {
            return Err(Error::Format);
        }
        let body = &data[pos + 2..pos + seg_len];
        pos += seg_len;

        match marker {
            0xDB => {
                // DQT
                let mut i = 0usize;
                while i < body.len() {
                    let info = body[i];
                    i += 1;
                    let prec = info >> 4;
                    let tid = (info & 0x0F) as usize;
                    if tid >= 4 || prec > 1 {
                        return Err(Error::Format);
                    }
                    let elem_size = if prec == 0 { 1 } else { 2 };
                    if i + 64 * elem_size > body.len() {
                        return Err(Error::Format);
                    }
                    for z in 0..64 {
                        let q = if prec == 0 {
                            body[i + z] as u16
                        } else {
                            u16::from_be_bytes([body[i + z * 2], body[i + z * 2 + 1]])
                        };
                        quants[tid][ZIGZAG[z]] = q.max(1);
                    }
                    quant_valid[tid] = true;
                    i += 64 * elem_size;
                }
            }
            0xC4 => {
                // DHT
                let mut i = 0usize;
                while i < body.len() {
                    let info = body[i];
                    i += 1;
                    let class = info >> 4;
                    let tid = (info & 0x0F) as usize;
                    if class > 1 || tid >= 4 || i + 16 > body.len() {
                        return Err(Error::Format);
                    }
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&body[i..i + 16]);
                    i += 16;
                    let syms: usize = counts.iter().map(|&c| c as usize).sum();
                    if syms > 256 || i + syms > body.len() {
                        return Err(Error::Format);
                    }
                    let tbl = HuffTable::build(&counts, &body[i..i + syms])?;
                    if class == 0 {
                        dc_huff[tid] = tbl;
                    } else {
                        ac_huff[tid] = tbl;
                    }
                    i += syms;
                }
            }
            0xDD => {
                if body.len() != 2 {
                    return Err(Error::Format);
                }
                restart_interval = u16::from_be_bytes([body[0], body[1]]) as usize;
            }
            0xC0 | 0xC1 => {
                if body.len() < 6 {
                    return Err(Error::Format);
                }
                let prec = body[0];
                height = u16::from_be_bytes([body[1], body[2]]) as usize;
                width = u16::from_be_bytes([body[3], body[4]]) as usize;
                let ncomp = body[5] as usize;
                let _ = check_dimensions(width, height)?;
                if prec != 8 || (ncomp != 1 && ncomp != 3) || body.len() != 6 + ncomp * 3 {
                    return Err(Error::Format);
                }
                comps.clear();
                for c in 0..ncomp {
                    let id = body[6 + c * 3];
                    let hv = body[7 + c * 3];
                    let qt = body[8 + c * 3] as usize;
                    let hs = (hv >> 4) as usize;
                    let vs = (hv & 0x0F) as usize;
                    if !(1..=2).contains(&hs) || !(1..=2).contains(&vs) || qt >= 4 {
                        return Err(Error::Format);
                    }
                    comps.push(Component {
                        id,
                        hsamp: hs,
                        vsamp: vs,
                        qtable: qt,
                    });
                }
            }
            0xC2 => return Err(Error::Format), // Progressive SOF2 unsupported in baseline decoder.
            0xDA => {
                if comps.is_empty() || body.is_empty() {
                    return Err(Error::Format);
                }
                let ns = body[0] as usize;
                if ns != comps.len() || body.len() != 1 + ns * 2 + 3 {
                    return Err(Error::Format);
                }
                let mut scan_map = [(0usize, 0usize); 3];
                for s in 0..ns {
                    let cid = body[1 + s * 2];
                    let tab = body[2 + s * 2];
                    let ci = comps.iter().position(|c| c.id == cid).ok_or(Error::Format)?;
                    let dc_t = (tab >> 4) as usize;
                    let ac_t = (tab & 0x0F) as usize;
                    if dc_t >= 4 || ac_t >= 4 || !quant_valid[comps[ci].qtable] {
                        return Err(Error::Format);
                    }
                    scan_map[ci] = (dc_t, ac_t);
                }
                return decode_scan(
                    data,
                    pos,
                    width,
                    height,
                    &comps,
                    &scan_map,
                    &quants,
                    &dc_huff,
                    &ac_huff,
                    restart_interval,
                );
            }
            _ => {}
        }
    }
    Err(Error::Format)
}

#[allow(clippy::too_many_arguments)]
fn decode_scan(
    data: &[u8],
    scan_pos: usize,
    width: usize,
    height: usize,
    comps: &[Component],
    scan_map: &[(usize, usize); 3],
    quants: &[[u16; 64]; 4],
    dc_huff: &[HuffTable; 4],
    ac_huff: &[HuffTable; 4],
    restart_interval: usize,
) -> Result<Image, Error> {
    let max_h = comps.iter().map(|c| c.hsamp).max().unwrap_or(1);
    let max_v = comps.iter().map(|c| c.vsamp).max().unwrap_or(1);
    let mcu_w = max_h * 8;
    let mcu_h = max_v * 8;
    let mcus_x = (width + mcu_w - 1) / mcu_w;
    let mcus_y = (height + mcu_h - 1) / mcu_h;

    let count = check_dimensions(width, height)?;
    let mut pixels = vec![0xFF00_0000u32; count];
    let mut dc_pred = [0i32; 3];
    let mut reader = BitReader::new(data, scan_pos);
    let mut mcu_seen = 0usize;

    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            if restart_interval > 0 && mcu_seen > 0 && (mcu_seen % restart_interval) == 0 {
                dc_pred = [0; 3];
                if let Some(m) = reader.pending_marker.take() {
                    if (0xD0..=0xD7).contains(&m) {
                        reader.reset_restart(reader.pos);
                    }
                } else {
                    reader.bit_buf = 0;
                    reader.bits_left = 0;
                    while reader.pos < data.len() {
                        if data[reader.pos] == 0xFF
                            && reader.pos + 1 < data.len()
                            && (0xD0..=0xD7).contains(&data[reader.pos + 1])
                        {
                            reader.pos += 2;
                            break;
                        }
                        reader.pos += 1;
                    }
                }
            }
            mcu_seen += 1;

            let mut blocks = [[[0u8; 64]; 4]; 3];
            for (ci, comp) in comps.iter().enumerate() {
                let (dc_t, ac_t) = scan_map[ci];
                let qtable = &quants[comp.qtable];
                let total_blocks = comp.hsamp * comp.vsamp;
                for bi in 0..total_blocks {
                    let mut coeffs = [0i32; 64];
                    let cat = reader.decode_huff(&dc_huff[dc_t])?;
                    let diff = reader.receive_extend(cat & 0x0F)?;
                    dc_pred[ci] = dc_pred[ci].wrapping_add(diff);
                    coeffs[0] = dc_pred[ci];

                    let mut k = 1usize;
                    while k < 64 {
                        let rs = reader.decode_huff(&ac_huff[ac_t])?;
                        if rs == 0 {
                            break;
                        }
                        let run = (rs >> 4) as usize;
                        let size = rs & 0x0F;
                        if rs == 0xF0 {
                            k += 16;
                            continue;
                        }
                        k += run;
                        if k >= 64 {
                            return Err(Error::Format);
                        }
                        coeffs[ZIGZAG[k]] = reader.receive_extend(size)?;
                        k += 1;
                    }
                    blocks[ci][bi] = idct_block(&coeffs, qtable);
                }
            }

            for dy in 0..mcu_h {
                let py = my * mcu_h + dy;
                if py >= height {
                    break;
                }
                for dx in 0..mcu_w {
                    let px = mx * mcu_w + dx;
                    if px >= width {
                        break;
                    }
                    let argb = if comps.len() == 1 {
                        let bx = dx / 8;
                        let by = dy / 8;
                        let yv = blocks[0][by * comps[0].hsamp + bx][(dy % 8) * 8 + (dx % 8)] as u32;
                        0xFF00_0000 | (yv << 16) | (yv << 8) | yv
                    } else {
                        let sample = |ci: usize| -> u8 {
                            let c = &comps[ci];
                            let sx = (dx * c.hsamp) / max_h;
                            let sy = (dy * c.vsamp) / max_v;
                            let bx = sx / 8;
                            let by = sy / 8;
                            blocks[ci][by * c.hsamp + bx][(sy % 8) * 8 + (sx % 8)]
                        };
                        ycbcr_to_argb(sample(0), sample(1), sample(2))
                    };
                    pixels[py * width + px] = argb;
                }
            }
        }
    }

    Ok(Image {
        width,
        height,
        pixels,
    })
}
