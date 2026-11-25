//! Bounded RFC 1951/1950/1952 (raw/zlib/gzip) inflate with strict output limits.
use crate::HttpError;
use alloc::{vec, vec::Vec};

const MAX_BITS: usize = 15;
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bitbuf: u32,
    bitcnt: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bitbuf: 0,
            bitcnt: 0,
        }
    }

    fn ensure(&mut self, n: u8) -> Result<(), HttpError> {
        while self.bitcnt < n {
            if self.pos >= self.data.len() {
                return Err(HttpError::Decompress);
            }
            self.bitbuf |= u32::from(self.data[self.pos]) << self.bitcnt;
            self.pos += 1;
            self.bitcnt += 8;
        }
        Ok(())
    }

    fn read_bits(&mut self, n: u8) -> Result<u32, HttpError> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure(n)?;
        let val = self.bitbuf & ((1u32 << n) - 1);
        self.bitbuf >>= n;
        self.bitcnt -= n;
        Ok(val)
    }

    fn align_byte(&mut self) {
        let drop = self.bitcnt & 7;
        self.bitbuf >>= drop;
        self.bitcnt -= drop;
    }

    fn read_u16_aligned(&mut self) -> Result<u16, HttpError> {
        while self.bitcnt >= 8 {
            self.pos -= 1;
            self.bitcnt -= 8;
        }
        self.bitbuf = 0;
        if self.pos + 2 > self.data.len() {
            return Err(HttpError::Decompress);
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn read_bytes_aligned(&mut self, len: usize) -> Result<&'a [u8], HttpError> {
        if self.pos + len > self.data.len() {
            return Err(HttpError::Decompress);
        }
        let s = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(s)
    }

    fn bytes_consumed(&self) -> usize {
        self.pos - usize::from(self.bitcnt / 8)
    }
}

struct HuffTable {
    counts: [u16; MAX_BITS + 1],
    symbols: Vec<u16>,
}

impl HuffTable {
    fn from_lengths(lengths: &[u8]) -> Result<Self, HttpError> {
        let mut counts = [0u16; MAX_BITS + 1];
        for &len in lengths {
            if usize::from(len) > MAX_BITS {
                return Err(HttpError::Decompress);
            }
            if len != 0 {
                counts[usize::from(len)] += 1;
            }
        }
        let mut left = 1i32;
        for &c in &counts[1..=MAX_BITS] {
            left <<= 1;
            left -= i32::from(c);
            if left < 0 {
                return Err(HttpError::Decompress);
            }
        }
        let mut offs = [0u16; MAX_BITS + 1];
        let mut total = 0u16;
        for b in 1..=MAX_BITS {
            offs[b] = total;
            total += counts[b];
        }
        let mut symbols = vec![0u16; usize::from(total)];
        for (sym, &len) in lengths.iter().enumerate() {
            if len != 0 {
                let idx = &mut offs[usize::from(len)];
                symbols[usize::from(*idx)] = sym as u16;
                *idx += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, r: &mut BitReader<'_>) -> Result<u16, HttpError> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0usize;
        for len in 1..=MAX_BITS {
            code |= r.read_bits(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return Ok(self.symbols[index + (code - first) as usize]);
            }
            index += count as usize;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(HttpError::Decompress)
    }
}

fn fixed_tables() -> Result<(HuffTable, HuffTable), HttpError> {
    let mut lit = [0u8; 288];
    lit[..144].fill(8);
    lit[144..256].fill(9);
    lit[256..280].fill(7);
    lit[280..288].fill(8);
    let dist = [5u8; 32];
    Ok((HuffTable::from_lengths(&lit)?, HuffTable::from_lengths(&dist)?))
}

fn dynamic_tables(r: &mut BitReader<'_>) -> Result<(HuffTable, HuffTable), HttpError> {
    let hlit = r.read_bits(5)? as usize + 257;
    let hdist = r.read_bits(5)? as usize + 1;
    let hclen = r.read_bits(4)? as usize + 4;
    if hlit > 286 || hdist > 32 {
        return Err(HttpError::Decompress);
    }
    let mut cl_lens = [0u8; 19];
    for &idx in &CL_ORDER[..hclen] {
        cl_lens[idx] = r.read_bits(3)? as u8;
    }
    let cl_table = HuffTable::from_lengths(&cl_lens)?;
    let total = hlit + hdist;
    let mut lens = vec![0u8; total];
    let mut i = 0;
    while i < total {
        let sym = cl_table.decode(r)?;
        match sym {
            0..=15 => {
                lens[i] = sym as u8;
                i += 1;
            }
            16..=18 => {
                let (val, rep) = match sym {
                    16 => {
                        if i == 0 {
                            return Err(HttpError::Decompress);
                        }
                        (lens[i - 1], 3 + r.read_bits(2)? as usize)
                    }
                    17 => (0u8, 3 + r.read_bits(3)? as usize),
                    _ => (0u8, 11 + r.read_bits(7)? as usize),
                };
                if i + rep > total {
                    return Err(HttpError::Decompress);
                }
                lens[i..i + rep].fill(val);
                i += rep;
            }
            _ => return Err(HttpError::Decompress),
        }
    }
    if lens[256] == 0 {
        return Err(HttpError::Decompress);
    }
    Ok((
        HuffTable::from_lengths(&lens[..hlit])?,
        HuffTable::from_lengths(&lens[hlit..])?,
    ))
}

pub fn inflate_raw(input: &[u8], limit: usize) -> Result<(Vec<u8>, usize), HttpError> {
    let mut r = BitReader::new(input);
    let mut out = Vec::new();
    loop {
        let bfinal = r.read_bits(1)?;
        let btype = r.read_bits(2)?;
        match btype {
            0 => {
                r.align_byte();
                let len = r.read_u16_aligned()?;
                let nlen = r.read_u16_aligned()?;
                if len ^ nlen != 0xffff {
                    return Err(HttpError::Decompress);
                }
                let ulen = usize::from(len);
                if out.len() + ulen > limit {
                    return Err(HttpError::TooLarge);
                }
                out.extend_from_slice(r.read_bytes_aligned(ulen)?);
            }
            1 | 2 => {
                let (lit, dist) = if btype == 1 {
                    fixed_tables()?
                } else {
                    dynamic_tables(&mut r)?
                };
                loop {
                    let sym = lit.decode(&mut r)?;
                    if sym < 256 {
                        if out.len() >= limit {
                            return Err(HttpError::TooLarge);
                        }
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let lidx = usize::from(sym - 257);
                        if lidx >= LEN_BASE.len() {
                            return Err(HttpError::Decompress);
                        }
                        let length = usize::from(LEN_BASE[lidx])
                            + r.read_bits(LEN_EXTRA[lidx])? as usize;
                        let dsym = usize::from(dist.decode(&mut r)?);
                        if dsym >= DIST_BASE.len() {
                            return Err(HttpError::Decompress);
                        }
                        let distance = usize::from(DIST_BASE[dsym])
                            + r.read_bits(DIST_EXTRA[dsym])? as usize;
                        if distance == 0 || distance > out.len() {
                            return Err(HttpError::Decompress);
                        }
                        if out.len() + length > limit {
                            return Err(HttpError::TooLarge);
                        }
                        for _ in 0..length {
                            let b = out[out.len() - distance];
                            out.push(b);
                        }
                    }
                }
            }
            _ => return Err(HttpError::Decompress),
        }
        if bfinal == 1 {
            break;
        }
    }
    r.align_byte();
    Ok((out, r.bytes_consumed()))
}

pub fn inflate_zlib(input: &[u8], limit: usize) -> Result<Vec<u8>, HttpError> {
    if input.len() < 6 {
        return Err(HttpError::Decompress);
    }
    let cmf = input[0];
    let flg = input[1];
    if (cmf & 0x0f) != 8
        || (cmf >> 4) > 7
        || ((u16::from(cmf) << 8) | u16::from(flg)) % 31 != 0
        || (flg & 0x20) != 0
    {
        return Err(HttpError::Decompress);
    }
    let (out, used) = inflate_raw(&input[2..input.len() - 4], limit)?;
    let tail = 2 + used;
    if tail + 4 != input.len() {
        return Err(HttpError::Decompress);
    }
    let expected = u32::from_be_bytes([
        input[tail],
        input[tail + 1],
        input[tail + 2],
        input[tail + 3],
    ]);
    if adler32(&out) != expected {
        return Err(HttpError::Decompress);
    }
    Ok(out)
}

pub fn inflate_gzip(input: &[u8], limit: usize) -> Result<Vec<u8>, HttpError> {
    if input.len() < 18 || input[0] != 0x1f || input[1] != 0x8b || input[2] != 8 {
        return Err(HttpError::Decompress);
    }
    let flg = input[3];
    if flg & 0xe0 != 0 {
        return Err(HttpError::Decompress);
    }
    let mut pos = 10usize;
    let body_end = input.len() - 8;
    if flg & 0x04 != 0 {
        if pos + 2 > body_end {
            return Err(HttpError::Decompress);
        }
        let xlen = usize::from(u16::from_le_bytes([input[pos], input[pos + 1]]));
        pos += 2;
        if pos + xlen > body_end {
            return Err(HttpError::Decompress);
        }
        pos += xlen;
    }
    for flag in [0x08u8, 0x10u8] {
        if flg & flag != 0 {
            let rel = input[pos..body_end]
                .iter()
                .position(|&b| b == 0)
                .ok_or(HttpError::Decompress)?;
            pos += rel + 1;
        }
    }
    if flg & 0x02 != 0 {
        if pos + 2 > body_end {
            return Err(HttpError::Decompress);
        }
        pos += 2;
    }
    let (out, used) = inflate_raw(&input[pos..body_end], limit)?;
    let tail = pos + used;
    if tail + 8 != input.len() {
        return Err(HttpError::Decompress);
    }
    let crc = u32::from_le_bytes([input[tail], input[tail + 1], input[tail + 2], input[tail + 3]]);
    let isize = u32::from_le_bytes([
        input[tail + 4],
        input[tail + 5],
        input[tail + 6],
        input[tail + 7],
    ]);
    if crc32(&out) != crc || (out.len() as u32) != isize {
        return Err(HttpError::Decompress);
    }
    Ok(out)
}

pub(crate) fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + u32::from(byte)) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}
