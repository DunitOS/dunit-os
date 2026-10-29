//! Pixel evidence for viewport containment and document-order scroll painting.
use dunit_render::{paint_clipped_with_widgets, ScrollArea, Surface};
use dunit_style::{cascade::NodeStyle, parse, Cascade};
use dunit_text::Font;
use dunit_ui::{layout, parse as dui, Rect};

fn render(source: &str, area: ScrollArea, damage: Rect) -> Vec<u32> {
    let tree = dui(source).unwrap();
    let layout = layout(&tree, 100., 80.);
    let mut cascade = Cascade::new();
    cascade.push(parse("Scroll { background: #112233; scrollbar-track: #445566; scrollbar-thumb: #778899; scrollbar-radius: 0; } Text { background: #ee0000; } Text#overlay { background: #0000ee; }").unwrap());
    let style_of = |id| {
        cascade.resolve(&NodeStyle {
            element: tree.node(id).kind.tag(),
            id: tree.node(id).name.as_deref(),
            classes: &[],
            states: &[],
        })
    };
    let f = Font::parse(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/DejaVuSans.ttf"
        ))
        .unwrap(),
    )
    .unwrap();
    let mut pixels = vec![0; 100 * 80];
    paint_clipped_with_widgets(
        &tree,
        &layout,
        &style_of,
        &f,
        &mut Surface::new(&mut pixels, 100, 80),
        damage,
        &|_| None,
        &|id| (tree.node(id).kind.tag() == "Scroll").then_some(&area),
    )
    .unwrap();
    pixels
}
fn area() -> ScrollArea {
    ScrollArea {
        viewport: Rect {
            x: 5.,
            y: 4.,
            w: 40.,
            h: 30.,
        },
        track: None,
        thumb: None,
    }
}
fn full() -> Rect {
    Rect {
        x: 0.,
        y: 0.,
        w: 100.,
        h: 80.,
    }
}

#[test]
fn scroll_children_are_clipped_to_viewport_while_parent_background_fills_box() {
    let p = render(
        "Stack { Scroll width=60 height=40 { Text width=90 height=90 } }",
        area(),
        full(),
    );
    for y in 0..80 {
        for x in 0..100 {
            let expected = if (5..45).contains(&x) && (4..34).contains(&y) {
                0xffee0000
            } else if x < 60 && y < 40 {
                0xff112233
            } else {
                0
            };
            assert_eq!(p[y * 100 + x], expected, "pixel {x},{y}");
        }
    }
}
#[test]
fn scrollbar_paints_after_children_and_thumb_after_track() {
    let mut a = area();
    a.track = Some(Rect {
        x: 35.,
        y: 4.,
        w: 10.,
        h: 30.,
    });
    a.thumb = Some(Rect {
        x: 37.,
        y: 9.,
        w: 6.,
        h: 12.,
    });
    let p = render(
        "Stack { Scroll width=60 height=40 { Text width=90 height=90 } }",
        a,
        full(),
    );
    assert_eq!(p[10 * 100 + 20], 0xffee0000);
    assert_eq!(p[10 * 100 + 35], 0xff445566);
    assert_eq!(p[10 * 100 + 38], 0xff778899);
    assert_eq!(p[25 * 100 + 38], 0xff445566);
}
#[test]
fn later_overlapping_sibling_covers_scroll_children_and_scrollbar() {
    let mut a = area();
    a.track = Some(Rect {
        x: 5.,
        y: 4.,
        w: 10.,
        h: 30.,
    });
    a.thumb = Some(Rect {
        x: 5.,
        y: 4.,
        w: 10.,
        h: 10.,
    });
    let p = render("Stack { Scroll width=60 height=40 { Text width=90 height=90 } Text#overlay width=20 height=20 }", a, full());
    for y in 0..20 {
        for x in 0..20 {
            assert_eq!(p[y * 100 + x], 0xff0000ee);
        }
    }
    assert_eq!(p[25 * 100 + 8], 0xff445566);
    assert_eq!(p[25 * 100 + 22], 0xffee0000);
}
#[test]
fn damage_clip_contains_background_children_and_scrollbar() {
    let mut a = area();
    a.track = Some(Rect {
        x: 35.,
        y: 4.,
        w: 10.,
        h: 30.,
    });
    a.thumb = Some(Rect {
        x: 37.,
        y: 9.,
        w: 6.,
        h: 12.,
    });
    let doc = "Stack { Scroll width=60 height=40 { Text width=90 height=90 } }";
    let reference = render(doc, a, full());
    let clipped = render(
        doc,
        a,
        Rect {
            x: 10.,
            y: 10.,
            w: 30.,
            h: 20.,
        },
    );
    for y in 0..80 {
        for x in 0..100 {
            assert_eq!(
                clipped[y * 100 + x],
                if (10..40).contains(&x) && (10..30).contains(&y) {
                    reference[y * 100 + x]
                } else {
                    0
                }
            );
        }
    }
}
