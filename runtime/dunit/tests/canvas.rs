//! Custom document canvases share cached coverage with retained SDK controls.
#[cfg(feature = "gui")]
#[test]
fn font_masks_are_reused_and_keep_identical_pixels_at_new_positions() {
    use dunit::canvas::{Color, Font, Surface};
    let bytes = include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec();
    let font = Font::parse(bytes).unwrap();
    let glyph = font.glyph_index('Ж');
    let first = font.rasterize(glyph, 14.).unwrap();
    let again = font.rasterize(glyph, 14.).unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    assert_eq!(font.cached_glyphs(), 1);
    let mut cached = vec![0; 160 * 40];
    font.draw_baseline(
        &mut Surface::new(&mut cached, 160, 40),
        8,
        25,
        14.,
        "Ж часы",
        Color::rgb(240, 100, 90),
    );
    let mut expected = vec![0; 160 * 40];
    let raw = font.into_inner();
    for g in raw.layout_line("Ж часы", 14.).0 {
        if let Some(bitmap) = raw.rasterize(g.glyph, 14.) {
            Surface::new(&mut expected, 160, 40).blit_glyph(
                &bitmap,
                8 + (g.x + 0.5) as i32 + bitmap.left,
                25 - bitmap.top,
                Color::rgb(240, 100, 90),
            );
        }
    }
    assert_eq!(cached, expected);
}
