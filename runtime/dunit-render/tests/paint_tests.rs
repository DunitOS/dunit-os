//! End-to-end painting: DUI tree + DSS cascade + font -> pixels.

use dunit_render::{pack_argb, paint, Surface};
use dunit_style::cascade::{Cascade, NodeStyle};
use dunit_style::parse as parse_dss;
use dunit_style::value::Color;
use dunit_text::Font;
use dunit_ui::{layout, parse as parse_dui};

fn load_font() -> Font {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/fonts/DejaVuSans.ttf"
    );
    let bytes = std::fs::read(path).expect("bundled DejaVuSans.ttf must be readable");
    Font::parse(bytes).expect("DejaVuSans.ttf must parse")
}

#[test]
fn paints_background_border_and_text() {
    let font = load_font();
    let tree = parse_dui(r#"Column#root { Button#ok width=120 height=40 "OK" }"#).expect("dui");

    let sheet = parse_dss(
        r#"
        Column { background: #202020; }
        Button {
            background: #3050ff;
            color: #ffffff;
            border-width: 2;
            border-color: #000000;
            font-size: 16;
            padding: 4;
        }
        "#,
    )
    .expect("dss");
    let mut cas = Cascade::new();
    cas.push(sheet);

    let lay = layout(&tree, 200.0, 60.0);

    let mut buf = vec![0u32; 200 * 60];
    let mut surface = Surface::new(&mut buf, 200, 60);
    let style_of = |id| cas.resolve(&NodeStyle::element(tree.node(id).kind.tag()));
    paint(&tree, &lay, &style_of, &font, &mut surface).unwrap();

    // Column fills the whole viewport behind the button.
    assert_eq!(
        surface.get(160, 50),
        pack_argb(Color::rgb(0x20, 0x20, 0x20))
    );
    // Button interior is its blue background (clear of border and glyphs).
    assert_eq!(surface.get(80, 30), pack_argb(Color::rgb(0x30, 0x50, 0xff)));
    // Left edge of the button is the black border.
    assert_eq!(surface.get(0, 20), pack_argb(Color::rgb(0, 0, 0)));

    // The "OK" label inked white pixels somewhere in the content box.
    let mut ink = 0;
    for y in 4..36 {
        for x in 4..120 {
            let p = surface.get(x, y);
            let (r, g, b) = ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff);
            if r > 200 && g > 200 && b > 200 {
                ink += 1;
            }
        }
    }
    assert!(ink > 0, "expected white text pixels, found none");
}

#[test]
fn empty_tree_paints_nothing() {
    let font = load_font();
    let tree = parse_dui("Column").expect("dui");
    let lay = layout(&tree, 10.0, 10.0);
    let mut buf = vec![0u32; 100];
    let mut surface = Surface::new(&mut buf, 10, 10);
    // No background declared -> surface stays zeroed.
    let style_of = |id| Cascade::new().resolve(&NodeStyle::element(tree.node(id).kind.tag()));
    paint(&tree, &lay, &style_of, &font, &mut surface).unwrap();
    assert!(buf.iter().all(|&p| p == 0));
}

#[test]
fn clipped_label_matches_the_buffered_raster_with_overhang() {
    use dunit_render::paint_clipped;
    use dunit_ui::Rect;

    let font = load_font();
    let px = 64.;
    let text = "fJ Åp e\u{301} Ж 0123456789";
    let tree = parse_dui(&format!(r#"Column {{ Text width=180 height=100 "{text}" }}"#)).unwrap();
    let layout = layout(&tree, 180., 100.);
    let mut cascade = Cascade::new();
    cascade.push(parse_dss("Text { font-size: 64; color: #ffffff; }").unwrap());
    let style = |id| cascade.resolve(&NodeStyle::element(tree.node(id).kind.tag()));
    let baseline = (font.line_metrics(px).ascent + 0.5) as i32;
    for (x, y, w, h) in [(0., 0., 11., 100.), (18., 21., 18., 8.), (93., 55., 39., 35.)] {
        let clip = Rect { x, y, w, h };
        let mut expected = vec![0; 180 * 100];
        Surface::new(&mut expected, 180, 100).with_clip(clip, |surface| {
            for g in font.layout_line(text, px).0 {
                if let Some(bitmap) = font.rasterize(g.glyph, px) {
                    surface.blit_glyph(
                        &bitmap,
                        (g.x + 0.5) as i32 + bitmap.left,
                        baseline - bitmap.top,
                        Color::rgb(255, 255, 255),
                    );
                }
            }
        });
        assert!(expected.iter().any(|&pixel| pixel != 0), "empty test crop {clip:?}");
        let mut actual = vec![0; expected.len()];
        paint_clipped(
            &tree,
            &layout,
            &style,
            &font,
            &mut Surface::new(&mut actual, 180, 100),
            clip,
        )
        .unwrap();
        assert!(actual == expected, "clipped label differs for {clip:?}");
    }
}

#[test]
fn explicit_rect_clip_contains_descendants_in_the_raw_painter() {
    let font = load_font();
    let tree =
        parse_dui(r#"Column { Column width=20 height=20 { Button width=40 height=40 "" } }"#)
            .unwrap();
    let lay = layout(&tree, 50., 50.);
    let mut cascade = Cascade::new();
    cascade.push(parse_dss("Column {clip:rect;} Button {background:#0000ff;}").unwrap());
    let style_of = |id| cascade.resolve(&NodeStyle::element(tree.node(id).kind.tag()));
    let mut pixels = vec![0; 50 * 50];
    let mut surface = Surface::new(&mut pixels, 50, 50);
    paint(&tree, &lay, &style_of, &font, &mut surface).unwrap();
    assert_eq!(surface.get(10, 10), 0xff0000ff);
    assert_eq!(surface.get(25, 10), 0);
}
