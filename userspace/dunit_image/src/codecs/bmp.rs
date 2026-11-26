//! Validated BMP24 and BMP32 (BI_RGB / BI_BITFIELDS) decoder -> ARGB8888.
use alloc::vec;

use super::check_dimensions;
use crate::{Error, Image};

fn read_u16(data: &[u8], off: usize) -> Option<u16> {
    let b = data.get(off..off + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(data: &[u8], off: usize) -> Option<u32> {
    let b = data.get(off..off + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_i32(data: &[u8], off: usize) -> Option<i32> {
    let b = data.get(off..off + 4)?;
    Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn extract_channel(word: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let shifted = mask >> shift;
    let bits = shifted.count_ones().min(8);
    if bits == 0 {
        return 0;
    }
    let val = (word & mask) >> shift;
    if bits == 8 {
        val as u8
    } else {
        ((val * 255) / ((1u32 << bits) - 1)) as u8
    }
}

pub fn decode_bmp(data: &[u8]) -> Result<Image, Error> {
    if data.len() < 54 || data.get(0..2) != Some(b"BM") {
        return Err(Error::Format);
    }
    let data_offset = read_u32(data, 10).ok_or(Error::Format)? as usize;
    let dib_size = read_u32(data, 14).ok_or(Error::Format)?;
    if dib_size < 40 {
        return Err(Error::Format);
    }
    let header_end = 14usize.checked_add(dib_size as usize).ok_or(Error::Format)?;
    if header_end > data.len() || data_offset < header_end {
        return Err(Error::Format);
    }
    let raw_w = read_i32(data, 18).ok_or(Error::Format)?;
    let raw_h = read_i32(data, 22).ok_or(Error::Format)?;
    let planes = read_u16(data, 26).ok_or(Error::Format)?;
    let bpp = read_u16(data, 28).ok_or(Error::Format)?;
    let compression = read_u32(data, 30).ok_or(Error::Format)?;
    if raw_w <= 0 || raw_h == 0 || planes != 1 {
        return Err(Error::Format);
    }
    let width = raw_w as usize;
    let height = raw_h.unsigned_abs() as usize;
    let top_down = raw_h < 0;
    let count = check_dimensions(width, height)?;

    if !(bpp == 24 && compression == 0) && !(bpp == 32 && (compression == 0 || compression == 3)) {
        return Err(Error::Format);
    }
    let bytes_per_px = (bpp / 8) as usize;
    let row_stride = width
        .checked_mul(bytes_per_px)
        .and_then(|n| n.checked_add(3))
        .ok_or(Error::Limit)?
        & !3;
    let needed = row_stride
        .checked_mul(height)
        .and_then(|n| data_offset.checked_add(n))
        .ok_or(Error::Limit)?;
    if needed > data.len() {
        return Err(Error::Format);
    }

    let (r_mask, g_mask, b_mask, a_mask) = if compression == 3 {
        let rm = read_u32(data, 54).ok_or(Error::Format)?;
        let gm = read_u32(data, 58).ok_or(Error::Format)?;
        let bm = read_u32(data, 62).ok_or(Error::Format)?;
        let am = if dib_size >= 56 {
            read_u32(data, 66).unwrap_or(0)
        } else {
            0
        };
        (rm, gm, bm, am)
    } else {
        (0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0)
    };

    let mut pixels = vec![0u32; count];
    let mut has_nonzero_alpha = false;

    for dst_y in 0..height {
        let src_y = if top_down { dst_y } else { height - 1 - dst_y };
        let row_start = data_offset + src_y * row_stride;
        let row = &data[row_start..row_start + width * bytes_per_px];
        let out_row = &mut pixels[dst_y * width..(dst_y + 1) * width];
        if bpp == 24 {
            for (x, px) in row.chunks_exact(3).enumerate() {
                out_row[x] = 0xFF00_0000
                    | ((px[2] as u32) << 16)
                    | ((px[1] as u32) << 8)
                    | (px[0] as u32);
            }
        } else if compression == 3 {
            for (x, px) in row.chunks_exact(4).enumerate() {
                let word = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                let r = extract_channel(word, r_mask) as u32;
                let g = extract_channel(word, g_mask) as u32;
                let b = extract_channel(word, b_mask) as u32;
                let a = if a_mask != 0 {
                    extract_channel(word, a_mask)
                } else {
                    0xFF
                };
                if a != 0 {
                    has_nonzero_alpha = true;
                }
                out_row[x] = ((a as u32) << 24) | (r << 16) | (g << 8) | b;
            }
        } else {
            for (x, px) in row.chunks_exact(4).enumerate() {
                let a = px[3];
                if a != 0 {
                    has_nonzero_alpha = true;
                }
                out_row[x] = ((a as u32) << 24)
                    | ((px[2] as u32) << 16)
                    | ((px[1] as u32) << 8)
                    | (px[0] as u32);
            }
        }
    }

    // Standard BI_RGB / RGBX 32bpp BMP files frequently store 0x00 in the padding byte.
    if bpp == 32 && !has_nonzero_alpha {
        for px in &mut pixels {
            *px |= 0xFF00_0000;
        }
    }

    Ok(Image {
        width,
        height,
        pixels,
    })
}
