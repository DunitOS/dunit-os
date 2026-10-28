//! Shared separable opaque-RGB box filters. Buffers must contain w*h pixels.
/// One horizontal box-blur pass: `src` -> `dst` (both `w*h` packed), radius `r`,
/// sliding-window average of RGB with edge clamp. Alpha forced opaque.
pub fn box_h(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize) {
    let r = r.min(w);
    if w == 0 || h == 0 {
        return;
    }
    for y in 0..h {
        let row = y * w;
        let mut sr = 0u32;
        let mut sg = 0u32;
        let mut sb = 0u32;
        let first_hi = r.min(w - 1);
        for i in 0..=first_hi {
            let p = src[row + i];
            sr += (p >> 16) & 0xFF;
            sg += (p >> 8) & 0xFF;
            sb += p & 0xFF;
        }
        let mut lo = 0i32;
        let mut hi = first_hi as i32;
        for x in 0..w {
            let cnt = (hi - lo + 1) as u32;
            dst[row + x] = 0xFF00_0000 | ((sr / cnt) << 16) | ((sg / cnt) << 8) | (sb / cnt);
            let add = x as i32 + 1 + r as i32;
            if add < w as i32 {
                let p = src[row + add as usize];
                sr += (p >> 16) & 0xFF;
                sg += (p >> 8) & 0xFF;
                sb += p & 0xFF;
                hi = add;
            }
            let rem = x as i32 - r as i32;
            if rem >= 0 {
                let p = src[row + rem as usize];
                sr -= (p >> 16) & 0xFF;
                sg -= (p >> 8) & 0xFF;
                sb -= p & 0xFF;
                lo = rem + 1;
            }
        }
    }
}

/// One vertical box-blur pass: `src` -> `dst` (both `w*h` packed), radius `r`.
pub fn box_v(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: usize) {
    let r = r.min(h);
    if w == 0 || h == 0 {
        return;
    }
    for x in 0..w {
        let mut sr = 0u32;
        let mut sg = 0u32;
        let mut sb = 0u32;
        let first_hi = r.min(h - 1);
        for i in 0..=first_hi {
            let p = src[i * w + x];
            sr += (p >> 16) & 0xFF;
            sg += (p >> 8) & 0xFF;
            sb += p & 0xFF;
        }
        let mut lo = 0i32;
        let mut hi = first_hi as i32;
        for y in 0..h {
            let cnt = (hi - lo + 1) as u32;
            dst[y * w + x] = 0xFF00_0000 | ((sr / cnt) << 16) | ((sg / cnt) << 8) | (sb / cnt);
            let add = y as i32 + 1 + r as i32;
            if add < h as i32 {
                let p = src[add as usize * w + x];
                sr += (p >> 16) & 0xFF;
                sg += (p >> 8) & 0xFF;
                sb += p & 0xFF;
                hi = add;
            }
            let rem = y as i32 - r as i32;
            if rem >= 0 {
                let p = src[rem as usize * w + x];
                sr -= (p >> 16) & 0xFF;
                sg -= (p >> 8) & 0xFF;
                sb -= p & 0xFF;
                lo = rem + 1;
            }
        }
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
impl Surface<'_> {
    /// Blur the already-painted opaque canvas behind a node, using two shared
    /// horizontal/vertical box passes. Samples include the complete filter halo
    /// beyond the output clip; only output pixels inside the shape are changed.
    pub fn backdrop_blur(&mut self, rect: Rect, radius: f32, corner: f32) -> PaintResult {
        if !radius.is_finite() || !(0. ..=64.).contains(&radius) {
            return Ok(());
        }
        let radius = (radius + 0.5) as usize;
        if radius == 0 {
            return Ok(());
        }
        let (x0, y0, x1, y1) = self.clip_rect(rect.x, rect.y, rect.w, rect.h);
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        // Two passes have a support radius of 2*r. Cropping there produces the
        // same output as filtering the whole canvas, including at window edges.
        let halo = radius * 2;
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
        for _ in 0..2 {
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
