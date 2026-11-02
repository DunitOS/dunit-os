//! Bounded coverage-mask reuse owned by a renderer, never by global state.
use crate::{Font, GlyphBitmap, GlyphId};
use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

// Technical memory ceilings for this optional derived-data cache. Coverage and
// entry budgets are separate so empty glyphs cannot grow metadata without bound.
pub const DEFAULT_GLYPH_CACHE_ENTRIES: usize = 512;
pub const DEFAULT_GLYPH_CACHE_BYTES: usize = 1024 * 1024;
struct Entry {
    bitmap: Option<Arc<GlyphBitmap>>,
    touched: u64,
}
impl Entry {
    fn bytes(&self) -> usize {
        self.bitmap.as_ref().map_or(0, |b| b.coverage.len())
    }
}
/// Reuses immutable glyph masks for one font source. Font clones share identity;
/// parsing another font replaces the source and discards the old cache. A Weak
/// source identity keeps the allocation identity reserved without pinning bytes.
pub struct GlyphCache {
    source: Option<Weak<alloc::vec::Vec<u8>>>,
    entries: BTreeMap<(GlyphId, u32), Entry>,
    bytes: usize,
    tick: u64,
    max_entries: usize,
    max_bytes: usize,
}
impl Default for GlyphCache {
    fn default() -> Self {
        Self::new(DEFAULT_GLYPH_CACHE_ENTRIES, DEFAULT_GLYPH_CACHE_BYTES)
    }
}
impl GlyphCache {
    pub fn new(max_entries: usize, max_coverage_bytes: usize) -> Self {
        Self {
            source: None,
            entries: BTreeMap::new(),
            bytes: 0,
            tick: 0,
            max_entries,
            max_bytes: max_coverage_bytes,
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn coverage_bytes(&self) -> usize {
        self.bytes
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.tick = 0;
        self.source = None;
    }
    pub fn rasterize(&mut self, font: &Font, glyph: GlyphId, px: f32) -> Option<Arc<GlyphBitmap>> {
        if !self
            .source
            .as_ref()
            .is_some_and(|source| font.source_matches(source))
        {
            self.clear();
            self.source = Some(font.source_weak());
        }
        // Invalid sizes retain the original rasterizer's behavior without
        // becoming cache keys; no unbounded NaN payload variants are retained.
        if !px.is_finite() || px <= 0. {
            return font.rasterize(glyph, px).map(Arc::new);
        }
        if self.tick == u64::MAX {
            // A wrap must not reverse recency ordering. Flushing derived data is
            // sufficient and leaves the source identity valid.
            self.entries.clear();
            self.bytes = 0;
            self.tick = 0;
        }
        self.tick += 1;
        let key = (glyph, px.to_bits());
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.touched = self.tick;
            return entry.bitmap.clone();
        }
        let bitmap = font.rasterize(glyph, px).map(Arc::new);
        let bytes = bitmap.as_ref().map_or(0, |b| b.coverage.len());
        if self.max_entries == 0 || bytes > self.max_bytes {
            return bitmap;
        }
        while self.entries.len() >= self.max_entries || self.bytes > self.max_bytes - bytes {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| *key);
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.bytes -= old.bytes();
            }
        }
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                bitmap: bitmap.clone(),
                touched: self.tick,
            },
        );
        bitmap
    }
}
