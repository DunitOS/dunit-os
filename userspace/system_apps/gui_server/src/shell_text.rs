//! Retained glyph atlas and clipped text painting; rasterization is confined to configuration/event boundaries.
use super::{blend_pixel, round_i32, Font, Rect};
use alloc::{string::String, vec::Vec};

#[derive(Clone,Default)]
pub(super) struct GlyphAtlas {
    items: Vec<CachedGlyph>,
    bytes: usize,
}
#[derive(Clone)]
struct CachedGlyph {
    ch: char,
    px: i32,
    advance: f32,
    bitmap: Option<dunit_text::GlyphBitmap>,
}
impl GlyphAtlas {
    pub(super) fn finish(&mut self) {
        self.items.sort_unstable_by_key(|g| (g.px, g.ch));
    }

    pub(super) fn add(&mut self, font: &Font, px: i32, text: &str) -> Result<(), String> {
        for ch in text.chars() {
            if self.items.iter().any(|g| g.ch == ch && g.px == px) {
                continue;
            }
            let id = font.glyph_index(ch);
            let bitmap = font.rasterize(id, px as f32);
            self.bytes += bitmap.as_ref().map_or(0, |b| b.coverage.len());
            if self.bytes > 16 * 1024 * 1024 || self.items.len() >= 8192 {
                return Err("shell glyph atlas exceeds resource budget".into());
            }
            let advance = font.glyph_metrics(id).advance as f32 * font.scale_for_px(px as f32);
            self.items.push(CachedGlyph {
                ch,
                px,
                advance,
                bitmap,
            });
        }
        Ok(())
    }
    fn get(&self, px: i32, ch: char) -> Option<&CachedGlyph> {
        self.items
            .binary_search_by_key(&(px, ch), |g| (g.px, g.ch))
            .ok()
            .map(|i| &self.items[i])
            .or_else(|| {
                self.items
                    .binary_search_by_key(&(px, '?'), |g| (g.px, g.ch))
                    .ok()
                    .map(|i| &self.items[i])
            })
    }
}
pub(super) fn text(
    glyphs: &GlyphAtlas,
    back: &mut [u32],
    bw: usize,
    bh: usize,
    _font: Option<&Font>,
    r: Rect,
    s: &str,
    px: i32,
    color: u32,
) {
    let max = (r.2 - 8).max(0) as f32;
    let mut width = 0.0;
    let mut end = 0;
    for (i, ch) in s.char_indices() {
        let advance = glyphs.get(px, ch).map_or(0.0, |g| g.advance);
        if width + advance > max {
            break;
        }
        width += advance;
        end = i + ch.len_utf8();
    }
    let mut pen = r.0 + (r.2 - round_i32(width)) / 2;
    let baseline = r.1 + (r.3 + px) / 2 - 2;
    let mut fractional = 0.0;
    for ch in s[..end].chars() {
        if let Some(glyph) = glyphs.get(px, ch) {
            if let Some(bitmap) = &glyph.bitmap {
                let ox = pen + bitmap.left;
                let oy = baseline - bitmap.top;
                for row in 0..bitmap.height {
                    for col in 0..bitmap.width {
                        let x = ox + col as i32;
                        let y = oy + row as i32;
                        if x < r.0 || y < r.1 || x >= r.0 + r.2 || y >= r.1 + r.3 {
                            continue;
                        }
                        let alpha = bitmap.coverage[row * bitmap.width + col] as u32
                            * ((color >> 24) & 255)
                            / 255;
                        if alpha > 0 {
                            blend_pixel(back, bw, bh, x, y, (alpha << 24) | (color & 0x00FFFFFF));
                        }
                    }
                }
            }
            fractional += glyph.advance;
            let advance = round_i32(fractional);
            pen += advance;
            fractional -= advance as f32;
        }
    }
}
