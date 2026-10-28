//! Real-font evidence for input hit testing, masking and document-order paint.
use dunit_render::{input_content_rect, paint_clipped_with_inputs, Surface, TextInput};
use dunit_style::{cascade::NodeStyle, parse, Cascade, Style};
use dunit_text::Font;
use dunit_ui::{layout, parse as dui, Rect};

const SHEET: &str = "Input { color: #ffffff; placeholder-color: #00ff00; selection-color: #334455; caret-color: #ff0000; caret-width: 2; font-size: 16; padding: 3; border-width: 1; border-color: #00000000; } Text { background: #123456; }";
fn font() -> Font {
    Font::parse(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/DejaVuSans.ttf"
        ))
        .unwrap(),
    )
    .unwrap()
}
fn input(value: &str) -> TextInput {
    TextInput {
        value: value.into(),
        placeholder: "Hint".into(),
        password: false,
        cursor: 0,
        anchor: 0,
        focused: false,
        caret_visible: false,
        scroll_x: 0.,
    }
}
fn style() -> Style {
    let mut c = Cascade::new();
    c.push(parse(SHEET).unwrap());
    c.resolve(&NodeStyle::element("Input"))
}
fn rect() -> Rect {
    Rect {
        x: 0.,
        y: 0.,
        w: 80.,
        h: 30.,
    }
}
fn render(input: &TextInput, source: &str, clip: Rect) -> Vec<u32> {
    let tree = dui(source).unwrap();
    let layout = layout(&tree, 100., 40.);
    let mut c = Cascade::new();
    c.push(parse(SHEET).unwrap());
    let style_of = |id| c.resolve(&NodeStyle::element(tree.node(id).kind.tag()));
    let mut pixels = vec![0; 100 * 40];
    paint_clipped_with_inputs(
        &tree,
        &layout,
        &style_of,
        &font(),
        &mut Surface::new(&mut pixels, 100, 40),
        clip,
        &|id| (tree.node(id).kind.tag() == "Input").then_some(input),
    )
    .unwrap();
    pixels
}
fn draw(input: &TextInput) -> Vec<u32> {
    render(
        input,
        "Column { Input width=80 height=30 }",
        Rect {
            x: 0.,
            y: 0.,
            w: 100.,
            h: 40.,
        },
    )
}
#[test]
fn utf8_hits_choose_nearest_scalar_boundary_including_mask_and_scroll() {
    let f = font();
    let s = style();
    for password in [false, true] {
        let mut i = input("aЖ🦀z");
        i.password = password;
        i.scroll_x = 7.;
        let mut x = input_content_rect(&s, rect()).x - i.scroll_x;
        for (offset, ch) in i.value.char_indices() {
            let width = f
                .layout_line(
                    &if password {
                        "•".into()
                    } else {
                        ch.to_string()
                    },
                    16.,
                )
                .1;
            assert_eq!(i.hit_index(&f, &s, rect(), x + width * 0.49), offset);
            assert_eq!(
                i.hit_index(&f, &s, rect(), x + width * 0.51),
                offset + ch.len_utf8()
            );
            x += width;
        }
        assert_eq!(i.hit_index(&f, &s, rect(), -100.), 0);
        assert_eq!(i.hit_index(&f, &s, rect(), 1000.), i.value.len());
    }
}
#[test]
fn password_pixels_reveal_only_scalar_count_and_never_change_value() {
    let mut a = input("SecretЖ");
    a.password = true;
    let mut b = input("七個字元abc");
    b.password = true;
    assert_eq!(a.value.chars().count(), b.value.chars().count());
    assert_eq!(draw(&a), draw(&b));
    assert_eq!(a.value, "SecretЖ");
    a.password = false;
    assert_ne!(draw(&a), draw(&b));
}
#[test]
fn placeholder_has_its_own_color_and_is_never_password_masked() {
    let mut i = input("");
    let normal = draw(&i);
    i.password = true;
    assert_eq!(draw(&i), normal);
    assert!(normal.contains(&0xff00ff00));
    i.value = "Value".into();
    let value = draw(&i);
    assert!(!value.contains(&0xff00ff00));
    assert!(value.contains(&0xffffffff));
}
#[test]
fn focused_selection_and_blink_caret_use_dss_colors() {
    let mut i = input("abcdefgh");
    i.cursor = 4;
    i.focused = true;
    i.caret_visible = true;
    let on = draw(&i);
    assert!(on.contains(&0xff334455));
    assert!(on.contains(&0xffff0000));
    i.caret_visible = false;
    let off = draw(&i);
    assert!(!off.contains(&0xffff0000));
    assert!(off.contains(&0xff334455));
    i.focused = false;
    let inactive = draw(&i);
    assert!(!inactive.contains(&0xff334455));
    assert!(!inactive.contains(&0xffff0000));
}
#[test]
fn scroll_follows_caret_preserves_viewport_and_clamps_after_resize() {
    let f = font();
    let s = style();
    let mut i = input("This long text extends well past the input");
    i.cursor = i.value.len();
    i.fit_scroll(&f, &s, rect());
    let first = i.scroll_x;
    assert!(first > 0.);
    let width = input_content_rect(&s, rect()).w - 2.;
    assert!((f.layout_line(&i.value, 16.).1 - first - width).abs() < 0.001);
    i.cursor -= 1;
    i.fit_scroll(&f, &s, rect());
    assert_eq!(i.scroll_x, first);
    i.cursor = 0;
    i.fit_scroll(&f, &s, rect());
    assert_eq!(i.scroll_x, 0.);
    i.scroll_x = 100.;
    i.fit_scroll(&f, &s, Rect { w: 2000., ..rect() });
    assert_eq!(i.scroll_x, 0.);
}
#[test]
fn text_selection_caret_stay_inside_content_and_damage_clip() {
    let mut i = input("Oversized line spilling into its neighbours");
    i.cursor = i.value.len();
    i.focused = true;
    i.caret_visible = true;
    i.fit_scroll(&font(), &style(), rect());
    let clip = Rect {
        x: 12.,
        y: 10.,
        w: 35.,
        h: 12.,
    };
    let pixels = render(&i, "Column { Input width=80 height=30 }", clip);
    assert!(pixels.iter().any(|p| *p != 0));
    for y in 0..40 {
        for x in 0..100 {
            if !(12..47).contains(&x) || !(10..22).contains(&y) {
                assert_eq!(pixels[y * 100 + x], 0);
            }
        }
    }
    let pixels = draw(&i);
    for y in 0..40 {
        for x in 0..100 {
            if !(4..76).contains(&x) || !(4..26).contains(&y) {
                assert_eq!(pixels[y * 100 + x], 0);
            }
        }
    }
}
#[test]
fn later_sibling_covers_input_at_document_position() {
    let mut i = input("visible input");
    i.cursor = 3;
    i.focused = true;
    i.caret_visible = true;
    let pixels = render(
        &i,
        "Stack { Input width=80 height=30 Text width=80 height=30 }",
        Rect {
            x: 0.,
            y: 0.,
            w: 100.,
            h: 40.,
        },
    );
    for y in 0..30 {
        for x in 0..80 {
            assert_eq!(pixels[y * 100 + x], 0xff123456);
        }
    }
}
#[test]
fn sdk_profiles_supply_input_style_and_state_tokens() {
    for profile in [
        include_str!("../../../assets/dwm/sdk/dark.dss"),
        include_str!("../../../assets/dwm/sdk/light.dss"),
    ] {
        let mut c = Cascade::new();
        c.push(
            parse(&format!(
                "{}\n{}",
                profile,
                include_str!("../../../assets/dwm/sdk/base.dss")
            ))
            .unwrap(),
        );
        let resolve = |states| {
            c.resolve(&NodeStyle {
                element: "Input",
                id: None,
                classes: &[],
                states,
            })
        };
        let base = resolve(&[]);
        for name in [
            "color",
            "placeholder-color",
            "selection-color",
            "caret-color",
        ] {
            assert!(base.color(name).is_some(), "{name}");
        }
        for name in ["font-size", "caret-width"] {
            assert!(base.number(name).unwrap() > 0., "{name}");
        }
        assert!(base.get("caret-blink").is_some());
        assert_ne!(
            base.color("border-color"),
            resolve(&["focus"]).color("border-color")
        );
        assert_ne!(
            base.color("background"),
            resolve(&["read-only"]).color("background")
        );
        assert_ne!(base.color("color"), resolve(&["disabled"]).color("color"));
    }
}
