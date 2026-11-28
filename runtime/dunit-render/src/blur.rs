//! Shared separable opaque-RGB box filters. Buffers must contain w*h pixels.
#[inline(always)]
fn avg_div(sum: u32, cnt: u32, mul: u64) -> u32 {
    if mul != 0 {
        ((sum as u64 * mul) >> 32) as u32
    } else {
        sum / cnt
    }
}

#[inline(always)]
fn recip_u32(cnt: u32) -> u64 {
    if cnt > 0 && cnt <= 4096 {
        ((1u64 << 32) + cnt as u64 - 1) / cnt as u64
    } else {
        0
    }
}

/// One horizontal box-blur pass: `src` -> `dst` (both `w*h` packed), radius `r`,
/// sliding-window average of RGB with edge clamp. Alpha forced opaque.
pub fn box_h(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize) {
    let r = r.min(w);
    if w == 0 || h == 0 {
        return;
    }
    if r == 0 {
        dst[..w * h].copy_from_slice(&src[..w * h]);
        for p in &mut dst[..w * h] {
            *p |= 0xFF00_0000;
        }
        return;
    }
    let first_hi = r.min(w - 1);
    let mid_cnt = (2 * r + 1) as u32;
    let mid_mul = recip_u32(mid_cnt);
    for y in 0..h {
        let row = y * w;
        let srow = &src[row..row + w];
        let drow = &mut dst[row..row + w];
        let mut sr = 0u32;
        let mut sg = 0u32;
        let mut sb = 0u32;
        for &p in &srow[..=first_hi] {
            sr += (p >> 16) & 0xFF;
            sg += (p >> 8) & 0xFF;
            sb += p & 0xFF;
        }
        let mut lo = 0usize;
        let mut hi = first_hi;
        let left_end = r.min(w);
        for x in 0..left_end {
            let cnt = (hi - lo + 1) as u32;
            let mul = recip_u32(cnt);
            drow[x] = 0xFF00_0000
                | (avg_div(sr, cnt, mul) << 16)
                | (avg_div(sg, cnt, mul) << 8)
                | avg_div(sb, cnt, mul);
            let add = x + 1 + r;
            if add < w {
                let p = srow[add];
                sr += (p >> 16) & 0xFF;
                sg += (p >> 8) & 0xFF;
                sb += p & 0xFF;
                hi = add;
            }
        }
        let right_start = w.saturating_sub(r + 1).max(left_end);
        for x in left_end..right_start {
            drow[x] = 0xFF00_0000
                | (avg_div(sr, mid_cnt, mid_mul) << 16)
                | (avg_div(sg, mid_cnt, mid_mul) << 8)
                | avg_div(sb, mid_cnt, mid_mul);
            let add = x + 1 + r;
            let rem = x - r;
            let p = srow[add];
            sr += (p >> 16) & 0xFF;
            sg += (p >> 8) & 0xFF;
            sb += p & 0xFF;
            hi = add;
            let pr = srow[rem];
            sr -= (pr >> 16) & 0xFF;
            sg -= (pr >> 8) & 0xFF;
            sb -= pr & 0xFF;
            lo = rem + 1;
        }
        for x in right_start..w {
            let cnt = (hi - lo + 1) as u32;
            let mul = recip_u32(cnt);
            drow[x] = 0xFF00_0000
                | (avg_div(sr, cnt, mul) << 16)
                | (avg_div(sg, cnt, mul) << 8)
                | avg_div(sb, cnt, mul);
            let add = x + 1 + r;
            if add < w {
                let p = srow[add];
                sr += (p >> 16) & 0xFF;
                sg += (p >> 8) & 0xFF;
                sb += p & 0xFF;
                hi = add;
            }
            if x >= r {
                let rem = x - r;
                let pr = srow[rem];
                sr -= (pr >> 16) & 0xFF;
                sg -= (pr >> 8) & 0xFF;
                sb -= pr & 0xFF;
                lo = rem + 1;
            }
        }
    }
}

/// One vertical box-blur pass: `src` -> `dst` (both `w*h` packed), radius `r`.
/// Processed in row-major column tiles so reads and writes stay sequential in L1.
pub fn box_v(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize) {
    let r = r.min(h);
    if w == 0 || h == 0 {
        return;
    }
    if r == 0 {
        dst[..w * h].copy_from_slice(&src[..w * h]);
        for p in &mut dst[..w * h] {
            *p |= 0xFF00_0000;
        }
        return;
    }
    const TILE_W: usize = 256;
    let first_hi = r.min(h - 1);
    let mut sum_r = [0u32; TILE_W];
    let mut sum_g = [0u32; TILE_W];
    let mut sum_b = [0u32; TILE_W];

    let mut x0 = 0usize;
    while x0 < w {
        let tw = (w - x0).min(TILE_W);
        sum_r[..tw].fill(0);
        sum_g[..tw].fill(0);
        sum_b[..tw].fill(0);
        for i in 0..=first_hi {
            let srow = &src[i * w + x0..i * w + x0 + tw];
            for k in 0..tw {
                let p = srow[k];
                sum_r[k] += (p >> 16) & 0xFF;
                sum_g[k] += (p >> 8) & 0xFF;
                sum_b[k] += p & 0xFF;
            }
        }
        let mut lo = 0usize;
        let mut hi = first_hi;
        for y in 0..h {
            let cnt = (hi - lo + 1) as u32;
            let mul = recip_u32(cnt);
            let drow = &mut dst[y * w + x0..y * w + x0 + tw];
            for k in 0..tw {
                drow[k] = 0xFF00_0000
                    | (avg_div(sum_r[k], cnt, mul) << 16)
                    | (avg_div(sum_g[k], cnt, mul) << 8)
                    | avg_div(sum_b[k], cnt, mul);
            }
            let add = y + 1 + r;
            if add < h {
                let srow = &src[add * w + x0..add * w + x0 + tw];
                for k in 0..tw {
                    let p = srow[k];
                    sum_r[k] += (p >> 16) & 0xFF;
                    sum_g[k] += (p >> 8) & 0xFF;
                    sum_b[k] += p & 0xFF;
                }
                hi = add;
            }
            if y >= r {
                let rem = y - r;
                let srow = &src[rem * w + x0..rem * w + x0 + tw];
                for k in 0..tw {
                    let p = srow[k];
                    sum_r[k] -= (p >> 16) & 0xFF;
                    sum_g[k] -= (p >> 8) & 0xFF;
                    sum_b[k] -= p & 0xFF;
                }
                lo = rem + 1;
            }
        }
        x0 += tw;
    }
}

use crate::{
    effects::{coverage, integer},
    PaintResult, Surface,
};
use alloc::{collections::TryReserveError, vec::Vec};
use dunit_ui::Rect;

fn scratch(length: usize) -> Result<Vec<u32>, TryReserveError> {
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(length)?;
    pixels.resize(length, 0);
    Ok(pixels)
}

const BACKDROP_PASSES: usize = 2;

fn backdrop_radius(radius: f32) -> usize {
    if radius.is_finite() && (0. ..=64.).contains(&radius) {
        (radius + 0.5) as usize
    } else {
        0
    }
}

/// Maximum distance sampled beyond a backdrop's output, in canvas pixels.
/// Uses the same quantization, validity limits and pass count as the painter.
/// Zero means the effect is disabled (or its radius is invalid).
pub fn backdrop_support(radius: f32) -> usize {
    backdrop_radius(radius) * BACKDROP_PASSES
}

impl Surface<'_> {
    /// Blur the already-painted opaque canvas behind a node, using two shared
    /// horizontal/vertical box passes. Samples include the complete filter halo
    /// beyond the output clip; only output pixels inside the shape are changed.
    pub fn backdrop_blur(&mut self, rect: Rect, radius: f32, corner: f32) -> PaintResult {
        let radius = backdrop_radius(radius);
        if radius == 0 {
            return Ok(());
        }
        let (x0, y0, x1, y1) = self.clip_rect(rect.x, rect.y, rect.w, rect.h);
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        // Two passes have a support radius of 2*r. Cropping there produces the
        // same output as filtering the whole canvas, including at window edges.
        let halo = radius * BACKDROP_PASSES;
        let sx0 = x0.saturating_sub(halo);
        let sy0 = y0.saturating_sub(halo);
        let sx1 = x1.saturating_add(halo).min(self.width);
        let sy1 = y1.saturating_add(halo).min(self.height);
        let (w, h) = (sx1 - sx0, sy1 - sy0);
        let mut a = scratch(w * h)?;
        let mut b = scratch(w * h)?;
        for row in 0..h {
            let source = (sy0 + row) * self.stride + sx0;
            a[row * w..(row + 1) * w].copy_from_slice(&self.pixels[source..source + w]);
        }
        for _ in 0..BACKDROP_PASSES {
            box_h(&a, &mut b, w, h, radius);
            box_v(&b, &mut a, w, h, radius);
        }
        let geometry = integer(rect);
        let corner = ((corner.max(0.) + 0.5) as i32).min(geometry.2.min(geometry.3).max(0) / 2);
        for py in y0..y1 {
            for px in x0..x1 {
                let cov = coverage(px as i32, py as i32, geometry, corner);
                let source = a[(py - sy0) * w + px - sx0];
                let index = py * self.stride + px;
                let before = self.pixels[index];
                // The backdrop is opaque; use exactly the same RGB rounding as
                // the desktop interpolation kernel, retaining the canvas alpha.
                self.pixels[index] =
                    (before & 0xff000000) | crate::raster::lerp_color(before, source, cov);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn naive_h(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize) {
        let r = r.min(w);
        for y in 0..h {
            for x in 0..w {
                let lo = x.saturating_sub(r);
                let hi = (x + r).min(w - 1);
                let cnt = (hi - lo + 1) as u32;
                let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
                for i in lo..=hi {
                    let p = src[y * w + i];
                    sr += (p >> 16) & 0xFF;
                    sg += (p >> 8) & 0xFF;
                    sb += p & 0xFF;
                }
                dst[y * w + x] = 0xFF00_0000 | ((sr / cnt) << 16) | ((sg / cnt) << 8) | (sb / cnt);
            }
        }
    }
    fn naive_v(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize) {
        let r = r.min(h);
        for x in 0..w {
            for y in 0..h {
                let lo = y.saturating_sub(r);
                let hi = (y + r).min(h - 1);
                let cnt = (hi - lo + 1) as u32;
                let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
                for i in lo..=hi {
                    let p = src[i * w + x];
                    sr += (p >> 16) & 0xFF;
                    sg += (p >> 8) & 0xFF;
                    sb += p & 0xFF;
                }
                dst[y * w + x] = 0xFF00_0000 | ((sr / cnt) << 16) | ((sg / cnt) << 8) | (sb / cnt);
            }
        }
    }
    #[test]
    fn box_blur_matches_naive_across_tile_and_edge_boundaries() {
        for (w, h, r) in [(1, 1, 3), (7, 5, 2), (257, 19, 5), (300, 12, 0)] {
            let src: Vec<u32> = (0..w * h)
                .map(|i| 0x8000_0000 | ((i as u32 * 37) & 0xFF) << 16 | ((i as u32 * 91) & 0xFF) << 8 | ((i as u32 * 13) & 0xFF))
                .collect();
            let mut got_h = vec![0; w * h];
            let mut ref_h = vec![0; w * h];
            box_h(&src, &mut got_h, w, h, r);
            naive_h(&src, &mut ref_h, w, h, r);
            assert_eq!(got_h, ref_h, "box_h mismatch for ({w}, {h}, {r})");

            let mut got_v = vec![0; w * h];
            let mut ref_v = vec![0; w * h];
            box_v(&got_h, &mut got_v, w, h, r);
            naive_v(&got_h, &mut ref_v, w, h, r);
            assert_eq!(got_v, ref_v, "box_v mismatch for ({w}, {h}, {r})");
        }
    }
}
