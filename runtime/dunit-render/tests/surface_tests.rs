//! Surface primitive tests: fills, clipping, alpha blending and borders.

use dunit_render::{pack_argb, Surface};
use dunit_style::value::Color;
use dunit_text::GlyphBitmap;
use dunit_ui::Rect;

const W: usize = 10;
const H: usize = 10;

fn blank() -> [u32; W * H] {
    [0u32; W * H]
}

#[test]
fn nested_clip_contains_clear_fill_and_disjoint_children_then_restores() {
    use dunit_ui::Rect;
    let mut pixels = blank();
    let mut surface = Surface::new(&mut pixels, W, H);
    let red = Color::rgb(255, 0, 0);
    let blue = Color::rgb(0, 0, 255);
    surface.with_clip(
        Rect {
            x: 2.0,
            y: 2.0,
            w: 4.0,
            h: 4.0,
        },
        |s| {
            s.clear(red);
            s.with_clip(
                Rect {
                    x: 3.0,
                    y: 3.0,
                    w: 1.0,
                    h: 1.0,
                },
                |s| s.fill_rect(0.0, 0.0, 10.0, 10.0, blue),
            );
            s.with_clip(
                Rect {
                    x: 8.0,
                    y: 8.0,
                    w: 2.0,
                    h: 2.0,
                },
                |s| s.clear(blue),
            );
        },
    );
    assert_eq!(surface.get(2, 2), pack_argb(red));
    assert_eq!(surface.get(3, 3), pack_argb(blue));
    assert_eq!(surface.get(8, 8), 0);
    surface.fill_rect(0.0, 0.0, 1.0, 1.0, blue);
    assert_eq!(surface.get(0, 0), pack_argb(blue));
}

#[test]
fn fill_is_clipped_and_placed() {
    let mut buf = blank();
    let mut s = Surface::new(&mut buf, W, H);
    let red = Color::rgb(255, 0, 0);
    s.fill_rect(2.0, 2.0, 4.0, 4.0, red);

    assert_eq!(s.get(2, 2), pack_argb(red)); // top-left corner of the fill
    assert_eq!(s.get(5, 5), pack_argb(red)); // last filled pixel (x1/y1 exclusive)
    assert_eq!(s.get(6, 6), 0); // just outside
    assert_eq!(s.get(0, 0), 0); // untouched
}

#[test]
fn fill_clips_past_the_edge() {
    let mut buf = blank();
    let mut s = Surface::new(&mut buf, W, H);
    // Extends well past the surface; must not panic and must clip.
    s.fill_rect(8.0, 8.0, 100.0, 100.0, Color::rgb(1, 2, 3));
    assert_eq!(s.get(9, 9), pack_argb(Color::rgb(1, 2, 3)));
}

#[test]
fn alpha_blends_over_existing() {
    let mut buf = blank();
    let mut s = Surface::new(&mut buf, W, H);
    s.fill_rect(0.0, 0.0, W as f32, H as f32, Color::rgb(0, 0, 255)); // opaque blue
    s.fill_rect(0.0, 0.0, W as f32, H as f32, Color::rgba(255, 0, 0, 128)); // half red

    let px = s.get(4, 4);
    let r = (px >> 16) & 0xff;
    let b = px & 0xff;
    assert_eq!(r, 128); // 255 * 128/255
    assert_eq!(b, 127); // 255 * 127/255, the surviving blue
}

#[test]
fn stroke_touches_only_the_frame() {
    let mut buf = blank();
    let mut s = Surface::new(&mut buf, W, H);
    let green = Color::rgb(0, 255, 0);
    s.stroke_rect(0.0, 0.0, W as f32, H as f32, 2.0, green);

    let g = pack_argb(green);
    assert_eq!(s.get(0, 0), g); // corner
    assert_eq!(s.get(5, 0), g); // top edge
    assert_eq!(s.get(5, 9), g); // bottom edge
    assert_eq!(s.get(0, 5), g); // left edge
    assert_eq!(s.get(9, 5), g); // right edge
    assert_eq!(s.get(5, 5), 0); // interior untouched
}

#[test]
fn blit_image_places_and_blends_rgba() {
    let mut buf = blank();
    let mut s = Surface::new(&mut buf, W, H);
    // A 2x2 image: opaque red, opaque green, transparent, half-white (R,G,B,A).
    let img: [u8; 16] = [
        255, 0, 0, 255, // (0,0) opaque red
        0, 255, 0, 255, // (1,0) opaque green
        9, 9, 9, 0, //     (0,1) fully transparent -> skipped
        255, 255, 255, 128, // (1,1) half white over black
    ];
    s.blit_image(3, 4, &img, 2, 2);

    assert_eq!(s.get(3, 4), pack_argb(Color::rgb(255, 0, 0)));
    assert_eq!(s.get(4, 4), pack_argb(Color::rgb(0, 255, 0)));
    assert_eq!(s.get(3, 5), 0); // transparent pixel left the surface untouched
                                // Half-white over opaque black: 255 * 128/255 = 128 per channel.
    let hw = s.get(4, 5);
    assert_eq!((hw >> 16) & 0xff, 128);
}

#[test]
fn blit_image_clips_out_of_bounds() {
    let mut buf = blank();
    let mut s = Surface::new(&mut buf, W, H);
    let img = [255u8; 4 * 4]; // 2x2 opaque white
                              // Straddle the bottom-right corner: only (9,9) lands on the surface.
    s.blit_image(9, 9, &img, 2, 2);
    assert_eq!(s.get(9, 9), pack_argb(Color::rgb(255, 255, 255)));
    // A short slice must not panic (clips trailing rows).
    let short = [255u8; 5];
    s.blit_image(0, 0, &short, 2, 2);
    assert_eq!(s.get(0, 0), pack_argb(Color::rgb(255, 255, 255)));
}

#[test]
fn glyph_blit_matches_scalar_coverage_on_a_strided_clipped_surface() {
    const STRIDE: usize = 13;
    let glyph = GlyphBitmap {
        width: 5,
        height: 4,
        left: 0,
        top: 0,
        coverage: (0u8..20).map(|i| i.wrapping_mul(51)).collect(),
    };
    let clip = Rect { x: 1., y: 1., w: 8., h: 8. };
    for (ox, oy) in [(-2, -1), (0, 0), (7, 7), (9, 9), (i32::MAX, i32::MAX)] {
        for alpha in [0, 1, 128, 255] {
            let ink = Color::rgba(31, 185, 247, alpha);
            let mut pixels = [0x9d325487; H * STRIDE];
            let mut expected = pixels;
            for y in 1..9 {
                for x in 1..9 {
                    let gx = x as i64 - i64::from(ox);
                    let gy = y as i64 - i64::from(oy);
                    if gx < 0 || gy < 0 {
                        continue;
                    }
                    let a = u32::from(alpha) * u32::from(glyph.at(gx as usize, gy as usize)) / 255;
                    if a != 0 {
                        let pixel = &mut expected[y * STRIDE + x];
                        let original = *pixel;
                        let channel = |shift: u32, source: u32| {
                            (source * a + ((original >> shift) & 255u32) * (255 - a)) / 255
                        };
                        *pixel = (a + (original >> 24) * (255 - a) / 255) << 24
                            | channel(16, 31) << 16
                            | channel(8, 185) << 8
                            | channel(0, 247);
                    }
                }
            }
            let mut surface = Surface::new(&mut pixels, W, H);
            surface.stride = STRIDE;
            surface.with_clip(clip, |s| s.blit_glyph(&glyph, ox, oy, ink));
            assert_eq!(pixels, expected, "origin ({ox}, {oy}), alpha {alpha}");
        }
    }
}

#[test]
fn argb_canvas_composites_through_clip_without_mutating_the_snapshot() {
    use dunit_ui::Rect;
    let base = 0xff102030;
    let mut pixels = [base; 4];
    let overlay = [0x80405060, 0xffff0000, 0xff00ff00, 0];
    let snapshot = overlay;
    let mut surface = Surface::new(&mut pixels, 2, 2);
    surface.with_clip(
        Rect {
            x: 0.,
            y: 0.,
            w: 1.,
            h: 2.,
        },
        |s| {
            assert!(s.blit_premultiplied(&overlay, 2, 2));
        },
    );
    assert_eq!(surface.get(0, 0), 0xff475f77);
    assert_eq!(surface.get(1, 0), base);
    assert_eq!(surface.get(0, 1), 0xff00ff00);
    assert_eq!(surface.get(1, 1), base);
    assert_eq!(overlay, snapshot);
    assert!(!surface.blit_premultiplied(&overlay, 3, 2));
    assert_eq!(surface.get(1, 1), base);
}
