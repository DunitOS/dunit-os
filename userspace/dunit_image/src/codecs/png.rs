//! Validated PNG decoder (Grayscale, RGB, Indexed PLTE+tRNS, Gray+Alpha, RGBA,
//! bit depths 1/2/4/8/16, standard & Adam7 interlace) reusing `dunit_http_core::inflate_zlib`.
use alloc::{vec, vec::Vec};

use super::check_dimensions;
use crate::{Error, Image};

const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const MAX_IDAT_BYTES: usize = 16 * 1024 * 1024;
const MAX_RAW_SCANLINES: usize = 4096 * 4096 * 8 + 4096 * 8;

const ADAM7_X0: [usize; 7] = [0, 4, 0, 2, 0, 1, 0];
const ADAM7_Y0: [usize; 7] = [0, 0, 4, 0, 2, 0, 1];
const ADAM7_DX: [usize; 7] = [8, 8, 4, 4, 2, 2, 1];
const ADAM7_DY: [usize; 7] = [8, 8, 8, 4, 4, 2, 2];

fn read_u32_be(data: &[u8], off: usize) -> Option<u32> {
    let b = data.get(off..off + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn channels_for(color_type: u8, bit_depth: u8) -> Option<usize> {
    match (color_type, bit_depth) {
        (0, 1 | 2 | 4 | 8 | 16) => Some(1),
        (2, 8 | 16) => Some(3),
        (3, 1 | 2 | 4 | 8) => Some(1),
        (4, 8 | 16) => Some(2),
        (6, 8 | 16) => Some(4),
        _ => None,
    }
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    let c = c as i32;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn unfilter_pass(
    raw: &[u8],
    offset: &mut usize,
    pass_w: usize,
    pass_h: usize,
    bpp: usize,
    row_bytes: usize,
) -> Result<Vec<u8>, Error> {
    let expected = row_bytes
        .checked_add(1)
        .and_then(|s| s.checked_mul(pass_h))
        .ok_or(Error::Limit)?;
    let slice = raw
        .get(*offset..(*offset).checked_add(expected).ok_or(Error::Limit)?)
        .ok_or(Error::Format)?;
    *offset += expected;

    let _ = pass_w;
    let mut out = vec![0u8; row_bytes.checked_mul(pass_h).ok_or(Error::Limit)?];
    for y in 0..pass_h {
        let src_row = &slice[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let filter = src_row[0];
        let src = &src_row[1..];
        let dst_start = y * row_bytes;
        let (prev_slice, dst) = if y == 0 {
            (&[][..], &mut out[dst_start..dst_start + row_bytes])
        } else {
            let (head, tail) = out.split_at_mut(dst_start);
            (&head[dst_start - row_bytes..dst_start], &mut tail[..row_bytes])
        };
        for i in 0..row_bytes {
            let a = if i >= bpp { dst[i - bpp] } else { 0 };
            let b = prev_slice.get(i).copied().unwrap_or(0);
            let c = if i >= bpp {
                prev_slice.get(i - bpp).copied().unwrap_or(0)
            } else {
                0
            };
            let recon = match filter {
                0 => src[i],
                1 => src[i].wrapping_add(a),
                2 => src[i].wrapping_add(b),
                3 => src[i].wrapping_add((((a as u16) + (b as u16)) >> 1) as u8),
                4 => src[i].wrapping_add(paeth(a, b, c)),
                _ => return Err(Error::Format),
            };
            dst[i] = recon;
        }
    }
    Ok(out)
}

fn sample_bits(row: &[u8], bit_depth: u8, idx: usize) -> u8 {
    match bit_depth {
        1 => (row[idx / 8] >> (7 - (idx % 8))) & 1,
        2 => (row[idx / 4] >> (6 - 2 * (idx % 4))) & 3,
        4 => (row[idx / 2] >> (4 - 4 * (idx % 2))) & 0x0F,
        8 => row[idx],
        16 => row[idx * 2],
        _ => 0,
    }
}

fn scale_gray(val: u8, bit_depth: u8) -> u8 {
    match bit_depth {
        1 => if val != 0 { 255 } else { 0 },
        2 => val * 85,
        4 => val * 17,
        8 | 16 => val,
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_row(
    row: &[u8],
    pass_w: usize,
    x0: usize,
    dx: usize,
    dst_y: usize,
    full_w: usize,
    color_type: u8,
    bit_depth: u8,
    palette: &[[u8; 3]],
    trns: &[u8],
    pixels: &mut [u32],
) -> Result<(), Error> {
    let trns_gray16 = (trns.len() >= 2).then(|| u16::from_be_bytes([trns[0], trns[1]]));
    let trns_rgb16 = (trns.len() >= 6).then(|| {
        [
            u16::from_be_bytes([trns[0], trns[1]]),
            u16::from_be_bytes([trns[2], trns[3]]),
            u16::from_be_bytes([trns[4], trns[5]]),
        ]
    });

    for px_i in 0..pass_w {
        let dst_x = x0 + px_i * dx;
        let argb = match (color_type, bit_depth) {
            (0, d) => {
                let raw = if d == 16 {
                    u16::from_be_bytes([row[px_i * 2], row[px_i * 2 + 1]])
                } else {
                    sample_bits(row, d, px_i) as u16
                };
                let g = scale_gray(if d == 16 { (raw >> 8) as u8 } else { raw as u8 }, d);
                let a = if trns_gray16 == Some(raw) { 0 } else { 255u32 };
                (a << 24) | ((g as u32) << 16) | ((g as u32) << 8) | g as u32
            }
            (2, 8) => {
                let r = row[px_i * 3];
                let g = row[px_i * 3 + 1];
                let b = row[px_i * 3 + 2];
                let a = if trns_rgb16 == Some([r as u16, g as u16, b as u16]) {
                    0
                } else {
                    255u32
                };
                (a << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
            }
            (2, 16) => {
                let base = px_i * 6;
                let r16 = u16::from_be_bytes([row[base], row[base + 1]]);
                let g16 = u16::from_be_bytes([row[base + 2], row[base + 3]]);
                let b16 = u16::from_be_bytes([row[base + 4], row[base + 5]]);
                let a = if trns_rgb16 == Some([r16, g16, b16]) {
                    0
                } else {
                    255u32
                };
                (a << 24) | (((r16 >> 8) as u32) << 16) | (((g16 >> 8) as u32) << 8) | (b16 >> 8) as u32
            }
            (3, d) => {
                let idx = sample_bits(row, d, px_i) as usize;
                let rgb = *palette.get(idx).ok_or(Error::Format)?;
                let a = trns.get(idx).copied().unwrap_or(255) as u32;
                (a << 24) | ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | rgb[2] as u32
            }
            (4, 8) => {
                let g = row[px_i * 2] as u32;
                let a = row[px_i * 2 + 1] as u32;
                (a << 24) | (g << 16) | (g << 8) | g
            }
            (4, 16) => {
                let g = row[px_i * 4] as u32;
                let a = row[px_i * 4 + 2] as u32;
                (a << 24) | (g << 16) | (g << 8) | g
            }
            (6, 8) => {
                let b = &row[px_i * 4..px_i * 4 + 4];
                ((b[3] as u32) << 24) | ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32
            }
            (6, 16) => {
                let b = &row[px_i * 8..px_i * 8 + 8];
                ((b[6] as u32) << 24) | ((b[0] as u32) << 16) | ((b[2] as u32) << 8) | b[4] as u32
            }
            _ => return Err(Error::Format),
        };
        pixels[dst_y * full_w + dst_x] = argb;
    }
    Ok(())
}

pub fn decode_png(data: &[u8]) -> Result<Image, Error> {
    if data.get(0..8) != Some(PNG_SIG) {
        return Err(Error::Format);
    }
    let mut pos = 8usize;
    let mut ihdr: Option<(usize, usize, u8, u8, u8)> = None;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    let mut seen_iend = false;

    while pos + 12 <= data.len() {
        let len = read_u32_be(data, pos).ok_or(Error::Format)? as usize;
        let kind = data.get(pos + 4..pos + 8).ok_or(Error::Format)?;
        let body_start = pos + 8;
        let body_end = body_start.checked_add(len).ok_or(Error::Limit)?;
        let next = body_end.checked_add(4).ok_or(Error::Limit)?;
        if next > data.len() {
            return Err(Error::Format);
        }
        let body = &data[body_start..body_end];
        match kind {
            b"IHDR" => {
                if ihdr.is_some() || body.len() != 13 {
                    return Err(Error::Format);
                }
                let w = read_u32_be(body, 0).ok_or(Error::Format)? as usize;
                let h = read_u32_be(body, 4).ok_or(Error::Format)? as usize;
                let _ = check_dimensions(w, h)?;
                let depth = body[8];
                let color_type = body[9];
                let comp = body[10];
                let filt = body[11];
                let interlace = body[12];
                if comp != 0 || filt != 0 || interlace > 1 || channels_for(color_type, depth).is_none() {
                    return Err(Error::Format);
                }
                ihdr = Some((w, h, depth, color_type, interlace));
            }
            b"PLTE" => {
                if body.is_empty() || body.len() % 3 != 0 || body.len() > 256 * 3 {
                    return Err(Error::Format);
                }
                palette = body.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"tRNS" => {
                if body.len() <= 256 {
                    trns.extend_from_slice(body);
                }
            }
            b"IDAT" => {
                if ihdr.is_none() {
                    return Err(Error::Format);
                }
                if idat.len().saturating_add(body.len()) > MAX_IDAT_BYTES {
                    return Err(Error::Limit);
                }
                idat.extend_from_slice(body);
            }
            b"IEND" => {
                seen_iend = true;
                break;
            }
            _ => {}
        }
        pos = next;
    }

    let (width, height, depth, color_type, interlace) = ihdr.ok_or(Error::Format)?;
    if !seen_iend || idat.is_empty() || (color_type == 3 && palette.is_empty()) {
        return Err(Error::Format);
    }

    let channels = channels_for(color_type, depth).ok_or(Error::Format)?;
    let bits_per_pixel = channels * (depth as usize);
    let bpp = (bits_per_pixel + 7) / 8;

    let raw = dunit_http_core::inflate::inflate_zlib(&idat, MAX_RAW_SCANLINES).map_err(|_| Error::Format)?;
    let count = check_dimensions(width, height)?;
    let mut pixels = vec![0u32; count];
    let mut raw_offset = 0usize;

    if interlace == 0 {
        let row_bytes = (width * bits_per_pixel + 7) / 8;
        let flat = unfilter_pass(&raw, &mut raw_offset, width, height, bpp, row_bytes)?;
        for y in 0..height {
            let row = &flat[y * row_bytes..(y + 1) * row_bytes];
            emit_row(
                row, width, 0, 1, y, width, color_type, depth, &palette, &trns, &mut pixels,
            )?;
        }
    } else {
        for pass in 0..7 {
            let x0 = ADAM7_X0[pass];
            let y0 = ADAM7_Y0[pass];
            let dx = ADAM7_DX[pass];
            let dy = ADAM7_DY[pass];
            if x0 >= width || y0 >= height {
                continue;
            }
            let pw = (width - x0 + dx - 1) / dx;
            let ph = (height - y0 + dy - 1) / dy;
            let row_bytes = (pw * bits_per_pixel + 7) / 8;
            let flat = unfilter_pass(&raw, &mut raw_offset, pw, ph, bpp, row_bytes)?;
            for py in 0..ph {
                let row = &flat[py * row_bytes..(py + 1) * row_bytes];
                emit_row(
                    row,
                    pw,
                    x0,
                    dx,
                    y0 + py * dy,
                    width,
                    color_type,
                    depth,
                    &palette,
                    &trns,
                    &mut pixels,
                )?;
            }
        }
    }

    Ok(Image {
        width,
        height,
        pixels,
    })
}
