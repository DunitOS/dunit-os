//! Host integration tests for the TTF stack, exercised against the bundled
//! DejaVu Sans font. Run with `cargo test` from `runtime/dunit-text`.

use dunit_text::Font;

fn load_font() -> Font {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/DejaVuSans.ttf");
    let bytes = std::fs::read(path).expect("bundled DejaVuSans.ttf must be readable");
    Font::parse(bytes).expect("DejaVuSans.ttf must parse")
}

#[test]
fn parses_basic_metadata() {
    let font = load_font();
    assert!(font.units_per_em() > 0, "unitsPerEm should be positive");
    assert!(font.num_glyphs() > 100, "font should have many glyphs");
    let lm = font.line_metrics(16.0);
    assert!(lm.ascent > 0.0, "ascent should be positive");
    assert!(lm.descent <= 0.0, "descent should be non-positive");
    assert!(lm.line_height() > 0.0, "line height should be positive");
}

#[test]
fn maps_characters_to_glyphs() {
    let font = load_font();
    let a = font.glyph_index('A');
    assert_ne!(a.0, 0, "'A' should map to a real glyph");
    let space = font.glyph_index(' ');
    assert_ne!(space.0, 0, "' ' should map to a real (blank) glyph");
    // Distinct characters map to distinct glyphs.
    assert_ne!(font.glyph_index('A').0, font.glyph_index('B').0);
}

#[test]
fn glyph_metrics_are_sane() {
    let font = load_font();
    let a = font.glyph_index('A');
    let m = font.glyph_metrics(a);
    assert!(m.advance > 0, "'A' should have a positive advance");
    // A wide 'W' should advance more than a narrow 'i'.
    let w = font.glyph_metrics(font.glyph_index('W')).advance;
    let i = font.glyph_metrics(font.glyph_index('i')).advance;
    assert!(w > i, "'W' should be wider than 'i' ({} vs {})", w, i);
}

#[test]
fn outline_has_contours() {
    let font = load_font();
    let outline = font.outline(font.glyph_index('A'));
    assert!(!outline.is_empty(), "'A' outline should not be empty");
    assert!(!outline.contours.is_empty(), "'A' should have at least one contour");
    assert!(outline.x_max > outline.x_min, "outline should have positive width");
    assert!(outline.y_max > outline.y_min, "outline should have positive height");
}

#[test]
fn rasterizes_visible_glyph() {
    let font = load_font();
    let bmp = font
        .rasterize(font.glyph_index('A'), 32.0)
        .expect("'A' should rasterize to a bitmap");
    assert!(bmp.width > 0 && bmp.height > 0, "bitmap should have area");
    assert_eq!(bmp.coverage.len(), bmp.width * bmp.height);

    let max = *bmp.coverage.iter().max().unwrap();
    let inked: usize = bmp.coverage.iter().filter(|&&c| c > 0).count();
    assert!(max > 200, "a solid glyph should have near-opaque pixels (max={})", max);
    assert!(inked > 0, "some pixels should be inked");
    // Anti-aliasing: expect at least a few partial-coverage edge pixels.
    let partial = bmp.coverage.iter().filter(|&&c| c > 0 && c < 255).count();
    assert!(partial > 0, "anti-aliased edges should produce partial coverage");
}

#[test]
fn blank_glyph_has_no_bitmap() {
    let font = load_font();
    // The space glyph carries no contours, so there is nothing to rasterize.
    assert!(font.rasterize(font.glyph_index(' '), 32.0).is_none());
}

#[test]
fn layout_line_accumulates_advances() {
    let font = load_font();
    let (glyphs, width) = font.layout_line("AVA", 32.0);
    assert_eq!(glyphs.len(), 3);
    assert_eq!(glyphs[0].x, 0.0, "first glyph starts at the origin");
    assert!(glyphs[1].x > 0.0, "second glyph is advanced right");
    assert!(glyphs[2].x > glyphs[1].x, "third glyph advances further");
    assert!(width > glyphs[2].x, "total width includes the last advance");

    // Total width equals the sum of scaled advances.
    let s = font.scale_for_px(32.0);
    let expected: f32 = "AVA"
        .chars()
        .map(|c| font.glyph_metrics(font.glyph_index(c)).advance as f32 * s)
        .sum();
    assert!((width - expected).abs() < 0.01, "width {} vs expected {}", width, expected);
}

#[test]
fn font_wide_bounds_cover_negative_bearings_and_positive_overhang() {
    let font = load_font();
    let (left, right) = font.horizontal_bounds_px(64.);
    assert!(left < 0.);
    for ch in ['J', 'j', 'f', 'Ж', 'é'] {
        let outline = font.outline(font.glyph_index(ch));
        let scale = font.scale_for_px(64.);
        assert!(left <= outline.x_min * scale);
        assert!(right >= outline.x_max * scale);
    }
    let (twice_left, twice_right) = font.horizontal_bounds_px(128.);
    assert_eq!((twice_left, twice_right), (left * 2., right * 2.));
}

#[test]
fn reversed_head_bounds_are_rejected() {
    let original = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/DejaVuSans.ttf")).unwrap();
    let count = u16::from_be_bytes(original[4..6].try_into().unwrap()) as usize;
    let record = (0..count).map(|i| 12 + i * 16).find(|&i| &original[i..i + 4] == b"head").unwrap();
    let head = u32::from_be_bytes(original[record + 8..record + 12].try_into().unwrap()) as usize;
    for (min, max) in [(36, 40), (38, 42)] {
        let mut bytes = original.clone();
        bytes[head + min..head + min + 2].copy_from_slice(&1i16.to_be_bytes());
        bytes[head + max..head + max + 2].copy_from_slice(&0i16.to_be_bytes());
        assert!(matches!(Font::parse(bytes), Err(dunit_text::TextError::UnsupportedFormat("head bounds"))));
    }
}

#[test]
fn vertical_ink_bounds_cover_glyphs_extending_past_logical_rows() {
    let font = load_font();
    let (bottom, top) = font.vertical_bounds_px(64.);
    let metrics = font.line_metrics(64.);
    assert!(top > metrics.ascent);
    assert!(bottom < metrics.descent);
    for ch in ['\u{1eb2}', '\u{06b8}'] {
        let outline = font.outline(font.glyph_index(ch));
        let scale = font.scale_for_px(64.);
        assert!(bottom <= outline.y_min * scale);
        assert!(top >= outline.y_max * scale);
    }
    let (twice_bottom, twice_top) = font.vertical_bounds_px(128.);
    assert_eq!((twice_bottom, twice_top), (bottom * 2., top * 2.));
}
