//! Shared resources for domain-specific canvases (terminal grids, images/icons).
//! Standard controls belong to `ui::View`; this module supplies no second widget toolkit.
use crate::{Error, Result};
use alloc::{string::String, sync::Arc, vec::Vec};
use core::{cell::RefCell, ops::Deref};
pub use dunit_component::Color;
pub use dunit_render::Surface;
use dunit_text::{GlyphBitmap, GlyphCache, GlyphId};

/// One renderer-owned font and bounded coverage cache, shared across its frames.
pub struct Font {
    font: dunit_text::Font,
    cache: RefCell<GlyphCache>,
}
impl Font {
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        Ok(Self {
            font: dunit_text::Font::parse(bytes).map_err(|_| Error(libdunit::EINVAL))?,
            cache: RefCell::new(GlyphCache::default()),
        })
    }
    /// Ordered, caller-configured candidates. Every file read is bounded; both
    /// unavailable and malformed fonts advance to the next explicit candidate.
    pub fn load(paths: &[&str]) -> Result<Self> {
        let mut error = Error(libdunit::ENOENT);
        for path in paths.iter().filter(|p| !p.is_empty()) {
            match Self::load_one(path) {
                Ok(font) => return Ok(font),
                Err(e) => error = e,
            }
        }
        Err(error)
    }
    fn load_one(path: &str) -> Result<Self> {
        Self::parse(crate::fs::read_bounded(path, 4 * 1024 * 1024)?)
    }
    pub fn into_inner(self) -> dunit_text::Font {
        self.font
    }
    pub fn rasterize(&self, glyph: GlyphId, px: f32) -> Option<Arc<GlyphBitmap>> {
        self.cache.borrow_mut().rasterize(&self.font, glyph, px)
    }
    pub fn text_width(&self, text: &str, px: f32) -> f32 {
        self.font.layout_line(text, px).1
    }
    pub fn fit(&self, text: &str, px: f32, width: f32) -> String {
        if self.text_width(text, px) <= width {
            return String::from(text);
        }
        let mut result = String::new();
        for ch in text.chars() {
            let mut trial = result.clone();
            trial.push(ch);
            trial.push('…');
            if self.text_width(&trial, px) > width {
                result.push('…');
                return result;
            }
            result.push(ch);
        }
        result
    }
    pub fn draw_baseline(
        &self,
        surface: &mut Surface<'_>,
        x: i32,
        baseline: i32,
        px: f32,
        text: &str,
        color: Color,
    ) {
        for g in self.font.layout_line(text, px).0 {
            if let Some(bitmap) = self.rasterize(g.glyph, px) {
                let ox = x + if g.x <= 0. { 0 } else { (g.x + 0.5) as i32 } + bitmap.left;
                surface.blit_glyph(&bitmap, ox, baseline - bitmap.top, color);
            }
        }
    }
    pub fn cached_glyphs(&self) -> usize {
        self.cache.borrow().len()
    }
}
impl Deref for Font {
    type Target = dunit_text::Font;
    fn deref(&self) -> &Self::Target {
        &self.font
    }
}
