use dunit_text::{Font, GlyphBitmap, GlyphCache};
use std::sync::Arc;
fn font() -> Font {
    Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap()
}
fn same(a: &GlyphBitmap, b: &GlyphBitmap) {
    assert_eq!(
        (a.width, a.height, a.left, a.top),
        (b.width, b.height, b.left, b.top)
    );
    assert_eq!(a.coverage, b.coverage);
}
#[test]
fn cached_masks_match_owned_rasterization_and_clones_share_source() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Font>();
    let f = font();
    let clone = f.clone();
    let mut cache = GlyphCache::default();
    for ch in ['A', 'g', 'é', 'Ж', '•'] {
        let gid = f.glyph_index(ch);
        let a = cache.rasterize(&f, gid, 16.).unwrap();
        same(&a, &f.rasterize(gid, 16.).unwrap());
        let b = cache.rasterize(&clone, gid, 16.).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
#[test]
fn size_bits_and_replaced_font_cannot_reuse_wrong_masks() {
    let f = font();
    let mut cache = GlyphCache::default();
    let gid = f.glyph_index('W');
    let a = cache.rasterize(&f, gid, 16.).unwrap();
    let b = cache.rasterize(&f, gid, 16.5).unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    same(&b, &f.rasterize(gid, 16.5).unwrap());
    assert!(Arc::ptr_eq(&a, &cache.rasterize(&f, gid, 16.).unwrap()));
    let mono =
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf").to_vec()).unwrap();
    let c = cache.rasterize(&mono, gid, 16.).unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
    same(&c, &mono.rasterize(gid, 16.).unwrap());
    assert_eq!(cache.len(), 1);
}
#[test]
fn metadata_and_coverage_budgets_evict_least_recently_used_masks() {
    let f = font();
    let (a, b, c) = (f.glyph_index('A'), f.glyph_index('B'), f.glyph_index('C'));
    let mut cache = GlyphCache::new(2, 1024 * 1024);
    let aa = cache.rasterize(&f, a, 16.).unwrap();
    let bb = cache.rasterize(&f, b, 16.).unwrap();
    assert!(Arc::ptr_eq(&aa, &cache.rasterize(&f, a, 16.).unwrap()));
    cache.rasterize(&f, c, 16.);
    assert_eq!(cache.len(), 2);
    assert!(Arc::ptr_eq(&aa, &cache.rasterize(&f, a, 16.).unwrap()));
    assert!(!Arc::ptr_eq(&bb, &cache.rasterize(&f, b, 16.).unwrap()));
    let bytes = aa.coverage.len().max(bb.coverage.len());
    let mut cache = GlyphCache::new(10, bytes);
    let aa = cache.rasterize(&f, a, 16.).unwrap();
    cache.rasterize(&f, b, 16.);
    assert_eq!(cache.len(), 1);
    assert!(cache.coverage_bytes() <= bytes);
    assert!(!Arc::ptr_eq(&aa, &cache.rasterize(&f, a, 16.).unwrap()));
}
#[test]
fn oversized_masks_bypass_cache_and_empty_glyphs_remain_bounded() {
    let f = font();
    let mut cache = GlyphCache::new(2, 1);
    let gid = f.glyph_index('A');
    let a = cache.rasterize(&f, gid, 16.).unwrap();
    let b = cache.rasterize(&f, gid, 16.).unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    assert!(cache.is_empty());
    for size in [12., 14., 16., 18., 20.] {
        assert!(cache.rasterize(&f, f.glyph_index(' '), size).is_none());
        assert!(cache.len() <= 2);
        assert_eq!(cache.coverage_bytes(), 0);
    }
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(cache.coverage_bytes(), 0);
    let mut off = GlyphCache::new(0, usize::MAX);
    assert!(off.rasterize(&f, gid, 16.).is_some());
    assert!(off.is_empty());
}
