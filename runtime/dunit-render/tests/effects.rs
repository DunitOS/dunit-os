use dunit_render::Surface;
use dunit_style::{
    effects::{Gradient, Shadow},
    Color,
};
use dunit_ui::Rect;
const WHITE: Color = Color::rgb(255, 255, 255);
#[test]
fn rounded_gradient_has_endpoints_coverage_and_continuous_inside_border() {
    let mut pixels = vec![0xff202020; 40 * 30];
    let mut s = Surface::new(&mut pixels, 40, 30);
    let rect = Rect {
        x: 4.,
        y: 4.,
        w: 30.,
        h: 20.,
    };
    let gradient = Gradient {
        horizontal: false,
        start: Color::rgb(255, 0, 0),
        end: Color::rgb(0, 0, 255),
    };
    s.rounded_box(rect, 8., None, Some(gradient), 0., WHITE);
    assert_eq!(s.get(4, 4), 0xff202020);
    assert_eq!(s.get(18, 4), 0xffff0000);
    assert_eq!(s.get(18, 23), 0xff0000ff);
    // A flat antialiased arc has partial coverage, rather than a binary cutout.
    s.rounded_box(rect, 8., Some(WHITE), None, 0., WHITE);
    let edge = s.get(6, 6) & 255;
    assert!(edge > 32 && edge < 255);
    s.rounded_box(rect, 8., None, Some(gradient), 0., WHITE);
    let centre = s.get(18, 10);
    s.rounded_box(rect, 8., None, None, 2., WHITE);
    assert_eq!(s.get(18, 4), 0xffffffff);
    assert_eq!(s.get(18, 10), centre);
}
#[test]
fn negative_origin_gradient_is_anchored_to_geometry_not_clip() {
    let mut a = vec![0xff000000; 20 * 20];
    let mut b = a.clone();
    let gradient = Gradient {
        horizontal: true,
        start: WHITE,
        end: Color::rgb(0, 0, 0),
    };
    let rect = Rect {
        x: -4.,
        y: -3.,
        w: 20.,
        h: 20.,
    };
    Surface::new(&mut a, 20, 20).rounded_box(rect, 6., None, Some(gradient), 0., WHITE);
    let mut surface = Surface::new(&mut b, 20, 20);
    surface.with_clip(
        Rect {
            x: 4.,
            y: 4.,
            w: 8.,
            h: 8.,
        },
        |s| s.rounded_box(rect, 6., None, Some(gradient), 0., WHITE),
    );
    for y in 0..20 {
        for x in 0..20 {
            assert_eq!(
                b[y * 20 + x],
                if (4..12).contains(&x) && (4..12).contains(&y) {
                    a[y * 20 + x]
                } else {
                    0xff000000
                }
            );
        }
    }
}
#[test]
fn shadow_extends_beyond_box_respects_clip_and_does_not_fill_interior() {
    let mut pixels = vec![0xffffffff; 40 * 40];
    let mut s = Surface::new(&mut pixels, 40, 40);
    s.with_clip(
        Rect {
            x: 5.,
            y: 5.,
            w: 30.,
            h: 30.,
        },
        |s| {
            s.drop_shadow(
                Rect {
                    x: 10.,
                    y: 10.,
                    w: 20.,
                    h: 20.,
                },
                5.,
                Shadow {
                    x: -2.,
                    y: 3.,
                    spread: 8.,
                    color: Color::rgba(0, 0, 0, 160),
                },
            )
        },
    );
    assert_eq!(s.get(4, 18), 0xffffffff);
    assert_ne!(s.get(8, 18), 0xffffffff);
    assert_ne!(s.get(20, 32), 0xffffffff);
    assert_eq!(s.get(20, 20), 0xffffffff);
}
#[test]
fn low_alpha_shadow_does_not_multiply_opacity_by_spread() {
    let mut pixels = vec![0xffffffff; 40 * 40];
    Surface::new(&mut pixels, 40, 40).drop_shadow(
        Rect {
            x: 15.,
            y: 15.,
            w: 10.,
            h: 10.,
        },
        0.,
        Shadow {
            x: 0.,
            y: 0.,
            spread: 12.,
            color: Color::rgba(0, 0, 0, 1),
        },
    );
    assert!(pixels.iter().all(|p| p & 255 >= 254));
}
