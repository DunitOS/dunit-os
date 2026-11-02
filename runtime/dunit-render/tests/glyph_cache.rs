use dunit_render::{paint_clipped_with_inputs, Surface, TextInput};
use dunit_style::{cascade::NodeStyle, parse, Cascade};
use dunit_text::{Font, GlyphCache};
use dunit_ui::{layout, parse as dui, Rect};
fn font(mono: bool) -> Font {
    Font::parse(if mono {
        include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf").to_vec()
    } else {
        include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()
    })
    .unwrap()
}
fn frame(
    font: &Font,
    cache: Option<&mut GlyphCache>,
    px: f32,
    color: &str,
    clip: Rect,
) -> Vec<u32> {
    let tree = dui(
        "Column { Button width=250 height=35 \"Repeat é Ж Repeat\" Textarea width=250 height=85 }",
    )
    .unwrap();
    let lay = layout(&tree, 280., 140.);
    let mut c = Cascade::new();
    c.push(parse(&format!("Column {{ background:#203040; }} Button,Textarea {{ font-size:{px};color:{color};padding:2;border-width:1;border-color:#abcdef;caret-width:1;caret-color:#ffffff;selection-color:#456789; }}")).unwrap());
    let input = TextInput {
        value: "Repeat é Ж\nRepeat Repeat\nlast".into(),
        placeholder: "Hint".into(),
        soft_wrap: true,
        password: false,
        multiline: true,
        affinity: Default::default(),
        spinbox: None,
        cursor: 12,
        anchor: 0,
        focused: true,
        caret_visible: true,
        scroll_x: 0.,
        scroll_y: 3.,
    };
    let mut pixels = vec![0x11223344; 280 * 140];
    let mut s = match cache {
        Some(cache) => Surface::with_glyph_cache(&mut pixels, 280, 140, cache),
        None => Surface::new(&mut pixels, 280, 140),
    };
    paint_clipped_with_inputs(
        &tree,
        &lay,
        &|id| c.resolve(&NodeStyle::element(tree.node(id).kind.tag())),
        font,
        &mut s,
        clip,
        &|id| (tree.node(id).kind.tag() == "Textarea").then_some(&input),
    )
    .unwrap();
    pixels
}
#[test]
fn cached_and_uncached_button_and_input_pixels_match_through_style_font_and_clip_changes() {
    let f = font(false);
    let mono = font(true);
    let mut cache = GlyphCache::new(24, 4096);
    for (font, px, color, clip) in [
        (
            &f,
            16.,
            "#ffffff",
            Rect {
                x: 0.,
                y: 0.,
                w: 280.,
                h: 140.,
            },
        ),
        (
            &f,
            16.,
            "#88ff99",
            Rect {
                x: 9.,
                y: 5.,
                w: 140.,
                h: 90.,
            },
        ),
        (
            &f,
            18.5,
            "#ffffff",
            Rect {
                x: 0.,
                y: 0.,
                w: 280.,
                h: 140.,
            },
        ),
        (
            &mono,
            16.,
            "#ffffff",
            Rect {
                x: 0.,
                y: 0.,
                w: 280.,
                h: 140.,
            },
        ),
        (
            &f,
            16.,
            "#ffffff",
            Rect {
                x: 0.,
                y: 0.,
                w: 280.,
                h: 140.,
            },
        ),
    ] {
        let expected = frame(font, None, px, color, clip);
        for _ in 0..2 {
            assert_eq!(frame(font, Some(&mut cache), px, color, clip), expected);
            assert!(cache.len() <= 24);
            assert!(cache.coverage_bytes() <= 4096);
        }
    }
}
