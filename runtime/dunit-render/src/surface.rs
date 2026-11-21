//! The ARGB8888 pixel surface and its drawing primitives.

use alloc::sync::Arc;
use core::ops::Deref;
use dunit_style::value::Color;
use dunit_text::{Font, GlyphBitmap, GlyphCache, GlyphId};
use dunit_ui::Rect;

/// Pack an DSS [`Color`] into a `0xAARRGGBB` pixel.
pub fn pack_argb(c: Color) -> u32 {
    ((c.a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)
}

pub(crate) enum PaintGlyph {
    Owned(GlyphBitmap),
    Shared(Arc<GlyphBitmap>),
}
impl Deref for PaintGlyph {
    type Target = GlyphBitmap;
    fn deref(&self) -> &GlyphBitmap {
        match self {
            Self::Owned(bitmap) => bitmap,
            Self::Shared(bitmap) => bitmap,
        }
    }
}
/// A mutable ARGB8888 pixel buffer. Pixels are `0xAARRGGBB`; `stride` is the
/// number of pixels per row (`>= width`), so a surface can address a sub-window
/// of a larger buffer.
pub struct Surface<'a> {
    pub pixels: &'a mut [u32],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    clip: (usize, usize, usize, usize),
    glyph_cache: Option<&'a mut GlyphCache>,
}

impl<'a> Surface<'a> {
    /// A surface over a tightly packed `width * height` buffer (stride = width).
    pub fn new(pixels: &'a mut [u32], width: usize, height: usize) -> Surface<'a> {
        Surface {
            pixels,
            width,
            height,
            stride: width,
            clip: (0, 0, width, height),
            glyph_cache: None,
        }
    }

    /// A surface using a renderer-owned mask cache. Colors/positions remain
    /// paint-time data; the cache stores coverage only.
    pub fn with_glyph_cache(
        pixels: &'a mut [u32],
        width: usize,
        height: usize,
        cache: &'a mut GlyphCache,
    ) -> Self {
        Self {
            glyph_cache: Some(cache),
            ..Self::new(pixels, width, height)
        }
    }
    pub(crate) fn rasterize(&mut self, font: &Font, glyph: GlyphId, px: f32) -> Option<PaintGlyph> {
        match self.glyph_cache.as_deref_mut() {
            Some(cache) => cache.rasterize(font, glyph, px).map(PaintGlyph::Shared),
            None => font.rasterize(glyph, px).map(PaintGlyph::Owned),
        }
    }

    /// Intersect a temporary clip with the current one. Drawing coordinates
    /// stay in surface space; nested clips cannot escape their parent. Empty
    /// intersections skip the callback (and expensive glyph rasterization).
    pub fn with_clip(&mut self, rect: Rect, draw: impl FnOnce(&mut Self)) {
        let previous = self.clip;
        self.clip = self.clip_rect(rect.x, rect.y, rect.w.max(0.0), rect.h.max(0.0));
        if self.clip.0 < self.clip.2 && self.clip.1 < self.clip.3 {
            draw(self);
        }
        self.clip = previous;
    }

    pub(crate) fn clip_bounds(&self) -> (usize, usize, usize, usize) {
        self.clip
    }

    /// The pixel at `(x, y)`, or 0 if out of bounds (test/debug helper).
    pub fn get(&self, x: usize, y: usize) -> u32 {
        if x < self.width && y < self.height {
            self.pixels[y * self.stride + x]
        } else {
            0
        }
    }

    /// Alpha-blend `color` over the pixel at `(x, y)` (src-over). Out-of-bounds
    /// and fully transparent writes are no-ops.
    pub(crate) fn blend(&mut self, x: usize, y: usize, color: Color, cov: u8) {
        if x < self.clip.0 || y < self.clip.1 || x >= self.clip.2 || y >= self.clip.3 {
            return;
        }
        let a = (color.a as u32 * cov as u32) / 255;
        if a != 0 {
            blend_pixel(&mut self.pixels[y * self.stride + x], color, a);
        }
    }

    /// Composite a complete, tightly packed premultiplied ARGB layer through this surface's
    /// existing clip and alpha kernels. Invalid dimensions are rejected.
    pub fn blit_premultiplied(&mut self, pixels: &[u32], width: usize, height: usize) -> bool {
        if width.checked_mul(height) != Some(pixels.len()) {
            return false;
        }
        for y in self.clip.1..self.clip.3.min(height) {
            for x in self.clip.0..self.clip.2.min(width) {
                let word = pixels[y * width + x];
                let a = word >> 24;
                if a == 0 {
                    continue;
                }
                let idx = y * self.stride + x;
                if a == 255 {
                    self.pixels[idx] = word;
                    continue;
                }
                let dst = self.pixels[idx];
                let inv = 255 - a;
                let channel = |shift: u32| {
                    (((word >> shift) & 255) + (((dst >> shift) & 255) * inv) / 255).min(255)
                };
                self.pixels[idx] = ((a + (dst >> 24) * inv / 255) << 24)
                    | (channel(16) << 16)
                    | (channel(8) << 8)
                    | channel(0);
            }
        }
        true
    }

    /// Overwrite every pixel in the current clip with `color` (a straight store,
    /// NOT a blend). Without a clip this resets the entire surface. Use
    /// this — never `fill_rect` — for a per-frame background reset when the
    /// surface is presented as translucent ARGB: `fill_rect` composites src-over
    /// onto whatever the buffer held last frame, so a `bg_alpha < 255` fill would
    /// accumulate alpha over the previous frame's already-blended contents and
    /// drift toward opaque. `clear` writes the exact `0xAARRGGBB` the client wants
    /// the compositor to see, so the desktop shows through with a stable alpha.
    pub fn clear(&mut self, color: Color) {
        let packed = pack_argb(color);
        for py in self.clip.1..self.clip.3 {
            let row = py * self.stride;
            self.pixels[row + self.clip.0..row + self.clip.2].fill(packed);
        }
    }

    /// Fill the rect `(x, y, w, h)` (surface pixels, clipped) with `color`,
    /// alpha-blended. Fractional coordinates are rounded to the nearest pixel.
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        if color.a == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let (x0, y0, x1, y1) = self.clip_rect(x, y, w, h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for py in y0..y1 {
            let row = py * self.stride;
            let pixels = &mut self.pixels[row + x0..row + x1];
            if color.a == 255 {
                pixels.fill(pack_argb(color));
            } else {
                for pixel in pixels {
                    blend_pixel(pixel, color, u32::from(color.a));
                }
            }
        }
    }

    /// Stroke a `width`-thick border just inside the rect `(x, y, w, h)`, so the
    /// border stays within the node's box (border-box semantics).
    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32, color: Color) {
        if color.a == 0 || width <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        // Clamp thickness so opposite edges never overlap past the box centre.
        let t = fmin(width, fmin(w, h) / 2.0);
        self.fill_rect(x, y, w, t, color); // top
        self.fill_rect(x, y + h - t, w, t, color); // bottom
        self.fill_rect(x, y, t, h, color); // left
        self.fill_rect(x + w - t, y, t, h, color); // right
    }

    /// Blit a glyph coverage mask in `color`, with the mask's top-left at pixel
    /// `(ox, oy)`. Coverage scales the color's alpha per pixel (anti-aliasing).
    pub fn blit_glyph(&mut self, glyph: &GlyphBitmap, ox: i32, oy: i32, color: Color) {
        if color.a == 0 || glyph.width == 0 || glyph.height == 0
            || glyph.coverage.len() < glyph.width.saturating_mul(glyph.height)
        {
            return;
        }
        let (ox, oy) = (i64::from(ox), i64::from(oy));
        let (left, top, right, bottom) = self.clip;
        let x0 = ox.max(left as i64);
        let x1 = ox.saturating_add(glyph.width as i64).min(right as i64);
        let y0 = oy.max(top as i64);
        let y1 = oy.saturating_add(glyph.height as i64).min(bottom as i64);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let width = (x1 - x0) as usize;
        let source_x = (x0 - ox) as usize;
        for py in y0..y1 {
            let dest = py as usize * self.stride + x0 as usize;
            let source = (py - oy) as usize * glyph.width + source_x;
            let pixels = &mut self.pixels[dest..dest + width];
            let coverage = &glyph.coverage[source..source + width];
            for (pixel, &cov) in pixels.iter_mut().zip(coverage) {
                let a = (u32::from(color.a) * u32::from(cov)) / 255;
                if a != 0 {
                    blend_pixel(pixel, color, a);
                }
            }
        }
    }

    /// Blit a straight-alpha RGBA image over the surface, its top-left at pixel
    /// `(ox, oy)`, alpha-blended (src-over) per pixel. `rgba` is row-major,
    /// 4 bytes/pixel in `R, G, B, A` byte order — the raw `.rgba` asset format
    /// the compositor already loads. Out-of-bounds pixels are skipped; a slice
    /// shorter than `w * h * 4` clips the trailing rows rather than panicking.
    pub fn blit_image(&mut self, ox: i32, oy: i32, rgba: &[u8], w: usize, h: usize) {
        for iy in 0..h {
            let py = oy + iy as i32;
            for ix in 0..w {
                let o = (iy * w + ix) * 4;
                if o + 4 > rgba.len() {
                    return; // ran off the end of the data: nothing more to draw.
                }
                let px = ox + ix as i32;
                if px < 0 || py < 0 || px as usize >= self.width || py as usize >= self.height {
                    continue;
                }
                let a = rgba[o + 3];
                if a == 0 {
                    continue;
                }
                let color = Color::rgba(rgba[o], rgba[o + 1], rgba[o + 2], a);
                self.blend(px as usize, py as usize, color, 255);
            }
        }
    }

    /// Round `(x, y, w, h)` to integer pixels and clip to the surface.
    pub(crate) fn clip_rect(&self, x: f32, y: f32, w: f32, h: f32) -> (usize, usize, usize, usize) {
        let x0 = clamp_usize(round_i32(x), self.width).max(self.clip.0);
        let y0 = clamp_usize(round_i32(y), self.height).max(self.clip.1);
        let x1 = clamp_usize(round_i32(x + w), self.width).min(self.clip.2);
        let y1 = clamp_usize(round_i32(y + h), self.height).min(self.clip.3);
        (x0, y0, x1, y1)
    }
}

#[inline]
fn blend_pixel(pixel: &mut u32, color: Color, a: u32) {
    if a == 255 {
        *pixel = pack_argb(color);
        return;
    }
    let dst = *pixel;
    let inv = 255 - a;
    let r = (u32::from(color.r) * a + ((dst >> 16) & 0xff) * inv) / 255;
    let g = (u32::from(color.g) * a + ((dst >> 8) & 0xff) * inv) / 255;
    let b = (u32::from(color.b) * a + (dst & 0xff) * inv) / 255;
    let out_a = a + (dst >> 24) * inv / 255;
    *pixel = (out_a << 24) | (r << 16) | (g << 8) | b;
}

/// Round to nearest, ties toward +infinity; clamps below at 0.
fn round_i32(v: f32) -> i32 {
    if v <= 0.0 {
        0
    } else {
        (v + 0.5) as i32
    }
}

fn clamp_usize(v: i32, max: usize) -> usize {
    if v < 0 {
        0
    } else if v as usize > max {
        max
    } else {
        v as usize
    }
}

fn fmin(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
