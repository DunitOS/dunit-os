use dunit_render::{
    blur::{box_h, box_v},
    Surface,
};
use dunit_style::Color;
use dunit_ui::Rect;
fn pattern(w: usize, h: usize) -> Vec<u32> {
    (0..w * h)
        .map(|i| {
            0xff000000
                | (((i * 71) % 256) as u32) << 16
                | (((i * 13) % 256) as u32) << 8
                | ((i * 97) % 256) as u32
        })
        .collect()
}
#[test]
fn shared_box_filters_match_independent_neighbourhood_average() {
    for (w, h) in [(1, 1), (3, 2), (17, 11)] {
        for radius in [0, 1, 4, 64] {
            for horizontal in [true, false] {
                let source = pattern(w, h);
                let mut result = source.clone();
                if horizontal {
                    box_h(&source, &mut result, w, h, radius);
                } else {
                    box_v(&source, &mut result, w, h, radius);
                }
                for y in 0..h {
                    for x in 0..w {
                        let (pos, span) = if horizontal { (x, w) } else { (y, h) };
                        let lo = pos.saturating_sub(radius);
                        let hi = (pos + radius + 1).min(span);
                        let mut expected = 0xff000000;
                        for shift in [0, 8, 16] {
                            let sum: u32 = (lo..hi)
                                .map(|p| {
                                    (source[if horizontal { y * w + p } else { p * w + x }]
                                        >> shift)
                                        & 255
                                })
                                .sum();
                            expected |= (sum / (hi - lo) as u32) << shift;
                        }
                        assert_eq!(result[y * w + x], expected);
                    }
                }
            }
        }
    }
    box_h(&[], &mut [], 0, 0, 2);
    box_v(&[], &mut [], 0, 0, 2);
}
#[test]
fn backdrop_halo_matches_full_canvas_filter_and_preserves_clipped_pixels() {
    let (w, h) = (51, 37);
    let source = pattern(w, h);
    let mut full = source.clone();
    let mut partial = source.clone();
    Surface::new(&mut full, w, h)
        .backdrop_blur(
            Rect {
                x: 0.,
                y: 0.,
                w: w as f32,
                h: h as f32,
            },
            4.,
            0.,
        )
        .unwrap();
    let rect = Rect {
        x: 9.,
        y: 6.,
        w: 21.,
        h: 19.,
    };
    Surface::new(&mut partial, w, h).with_clip(
        Rect {
            x: 11.,
            y: 8.,
            w: 14.,
            h: 13.,
        },
        |s| s.backdrop_blur(rect, 4., 0.).unwrap(),
    );
    for y in 0..h {
        for x in 0..w {
            assert_eq!(
                partial[y * w + x],
                if (11..25).contains(&x) && (8..21).contains(&y) {
                    full[y * w + x]
                } else {
                    source[y * w + x]
                }
            );
        }
    }
}
#[test]
fn rounded_group_clips_completed_pixels_once_for_overlapping_children() {
    let rect = Rect {
        x: 4.,
        y: 3.,
        w: 24.,
        h: 24.,
    };
    let mut actual = vec![0xff000000; 32 * 32];
    let mut expected = actual.clone();
    Surface::new(&mut expected, 32, 32).rounded_box(
        rect,
        10.,
        Some(Color::rgb(255, 255, 255)),
        None,
        0.,
        Color::rgb(0, 0, 0),
    );
    Surface::new(&mut actual, 32, 32)
        .with_rounded_clip(rect, 10., |s| {
            for _ in 0..2 {
                s.fill_rect(0., 0., 32., 32., Color::rgb(255, 255, 255));
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(actual, expected);
}
#[test]
fn nested_masks_and_backdrop_do_not_escape_the_parent() {
    let source = pattern(40, 40);
    let mut pixels = source.clone();
    Surface::new(&mut pixels, 40, 40)
        .with_rounded_clip(
            Rect {
                x: 5.,
                y: 5.,
                w: 25.,
                h: 25.,
            },
            12.,
            |s| {
                s.with_rounded_clip(
                    Rect {
                        x: 0.,
                        y: 0.,
                        w: 40.,
                        h: 40.,
                    },
                    4.,
                    |s| {
                        s.backdrop_blur(
                            Rect {
                                x: 0.,
                                y: 0.,
                                w: 40.,
                                h: 40.,
                            },
                            3.,
                            0.,
                        )?;
                        s.fill_rect(0., 0., 40., 40., Color::rgba(255, 255, 255, 80));
                        Ok(())
                    },
                )
            },
        )
        .unwrap();
    assert_eq!(pixels[5 * 40 + 5], source[5 * 40 + 5]);
    assert_eq!(pixels[4 * 40 + 20], source[4 * 40 + 20]);
    assert_ne!(pixels[18 * 40 + 18], source[18 * 40 + 18]);
}

#[test]
fn rounded_group_keeps_straight_alpha_and_exact_pixels_outside_mask() {
    let mut pixels = vec![0x00112233; 32 * 32];
    Surface::new(&mut pixels, 32, 32)
        .with_rounded_clip(
            Rect {
                x: 4.,
                y: 3.,
                w: 24.,
                h: 24.,
            },
            10.,
            |s| {
                s.fill_rect(0., 0., 32., 32., Color::rgb(255, 255, 255));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(pixels[3 * 32 + 4], 0x00112233);
    assert!(pixels.iter().any(|p| p >> 24 > 0 && p >> 24 < 255));
    for pixel in pixels {
        if pixel >> 24 > 0 {
            assert_eq!(pixel & 0x00ffffff, 0x00ffffff);
        }
    }
}

#[test]
fn backdrop_support_uses_the_painters_quantization_and_limits() {
    use dunit_render::blur::backdrop_support;
    for (radius, support) in [
        (0., 0),
        (0.49, 0),
        (0.5, 2),
        (1.49, 2),
        (1.5, 4),
        (63.5, 128),
        (64., 128),
        (64.1, 0),
        (-1., 0),
        (f32::NAN, 0),
        (f32::INFINITY, 0),
    ] {
        assert_eq!(backdrop_support(radius), support);
    }
}
