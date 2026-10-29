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
        multiline: false,
        cursor: 0,
        anchor: 0,
        focused: false,
        caret_visible: false,
        scroll_x: 0.,
        scroll_y: 0.,
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
        for name in ["font-size", "caret-width", "scroll-step"] {
            assert!(base.number(name).unwrap() > 0., "{name}");
        }
        assert!(base.get("caret-blink").is_some());
        let area = c.resolve(&NodeStyle::element("Textarea"));
        assert_eq!(base, area);
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

#[test]
fn multiline_unicode_hits_follow_rows_blank_lines_and_scrolled_axes() {
    let f = font();
    let s = style();
    let r = Rect { h: 65., ..rect() };
    let h = f.line_metrics(16.).line_height();
    let mut i = input("aЖ\n\n🦀z\n");
    i.multiline = true;
    let c = input_content_rect(&s, r);
    assert_eq!(i.hit_index_at(&f, &s, r, c.x + 100., c.y + h * 0.5), 3);
    assert_eq!(i.hit_index_at(&f, &s, r, c.x, c.y + h * 1.5), 4);
    assert_eq!(i.hit_index_at(&f, &s, r, c.x, c.y + h * 2.5), 5);
    assert_eq!(
        i.hit_index_at(&f, &s, r, c.x + 100., c.y + h * 100.),
        i.value.len()
    );
    i.scroll_y = h * 2.;
    i.scroll_x = f.layout_line("🦀", 16.).1;
    assert_eq!(i.hit_index_at(&f, &s, r, c.x, c.y + h * 0.5), 9);
}
#[test]
fn multiline_scroll_fits_both_axes_including_trailing_blank_caret() {
    let f = font();
    let s = style();
    let r = rect();
    let h = f.line_metrics(16.).line_height();
    let mut i = input("a long long long first line\nsecond\n\n");
    i.multiline = true;
    i.cursor = "a long long long first line".len();
    i.fit_scroll(&f, &s, r);
    assert!(i.scroll_x > 0.);
    assert_eq!(i.scroll_y, 0.);
    i.cursor = i.value.len();
    i.fit_scroll(&f, &s, r);
    assert_eq!(i.scroll_x, 0.);
    assert!((i.scroll_y - (4. * h - input_content_rect(&s, r).h)).abs() < 0.001);
    let end_scroll = i.scroll_y;
    assert!(i.scroll_lines(&f, &s, r, -1.));
    assert!((i.scroll_y - end_scroll + h).abs() < 0.001);
    assert!(i.scroll_lines(&f, &s, r, -100.));
    assert_eq!(i.scroll_y, 0.);
    assert!(!i.scroll_lines(&f, &s, r, -1.));
    // Manual viewport motion is independent of the insertion point.
    assert_eq!(i.cursor, i.value.len());
    assert!(!i.scroll_lines(&f, &s, r, f32::NAN));
}
#[test]
fn multiline_lf_selection_and_trailing_caret_paint_on_their_lines() {
    let f = font();
    let h = f.line_metrics(16.).line_height();
    let mut i = input("a\n\n");
    i.multiline = true;
    i.focused = true;
    i.anchor = 1;
    i.cursor = 2;
    let p = render(
        &i,
        "Column { Input width=80 height=80 }",
        Rect {
            x: 0.,
            y: 0.,
            w: 100.,
            h: 40.,
        },
    );
    assert_eq!(
        p[5 * 100 + 70],
        0xff334455,
        "selected LF extends to content edge"
    );
    assert_eq!(
        p[(4. + h + 1.) as usize * 100 + 70],
        0,
        "unselected blank line"
    );
    i.anchor = i.value.len();
    i.cursor = i.value.len();
    i.caret_visible = true;
    i.scroll_y = h;
    let p = draw(&i);
    assert_eq!(
        p[(4. + h + 1.) as usize * 100 + 4],
        0xffff0000,
        "caret follows final LF"
    );
}
#[test]
fn multiline_paint_ignores_offscreen_lines_and_long_line_suffixes() {
    let mut short = input("visible first row\nvisible second row\nthird");
    short.multiline = true;
    let mut long = short.clone();
    long.value.push_str(&"\nINVISIBLE".repeat(10000));
    assert_eq!(draw(&short), draw(&long));
    short.value = "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWW".into();
    long.value = format!("{}{}", short.value, "z".repeat(100000));
    assert_eq!(draw(&short), draw(&long));
    short.value = "top\nvisible".into();
    short.scroll_y = font().line_metrics(16.).line_height();
    long.value = "bottom\nvisible".into();
    long.scroll_y = short.scroll_y;
    assert_eq!(draw(&short), draw(&long));
}
#[test]
fn multiline_selection_respects_damage_clip_and_later_sibling() {
    let mut i = input("first\nsecond\nthird");
    i.multiline = true;
    i.focused = true;
    i.cursor = i.value.len();
    i.caret_visible = true;
    let clip = Rect {
        x: 11.,
        y: 8.,
        w: 26.,
        h: 20.,
    };
    let p = render(&i, "Column { Input width=80 height=70 }", clip);
    assert!(p.contains(&0xff334455));
    for y in 0..40 {
        for x in 0..100 {
            if !(11..37).contains(&x) || !(8..28).contains(&y) {
                assert_eq!(p[y * 100 + x], 0);
            }
        }
    }
    let p = render(
        &i,
        "Stack { Input width=80 height=70 Text width=80 height=70 }",
        Rect {
            x: 0.,
            y: 0.,
            w: 100.,
            h: 40.,
        },
    );
    for y in 0..40 {
        for x in 0..80 {
            assert_eq!(p[y * 100 + x], 0xff123456);
        }
    }
}

#[test]
fn scrolled_ink_overhang_matches_unculled_clip_reference() {
    let f = font();
    let px = 128.;
    let sheet = "Input { font-size: 128; color: #ffffff; padding: 4; }";
    let tree = dui("Column { Input width=80 height=170 }").unwrap();
    let layout = layout(&tree, 100., 180.);
    let mut cascade = Cascade::new();
    cascade.push(parse(sheet).unwrap());
    let style_of = |id| cascade.resolve(&NodeStyle::element(tree.node(id).kind.tag()));
    let style = cascade.resolve(&NodeStyle::element("Input"));
    let content = input_content_rect(
        &style,
        Rect {
            x: 0.,
            y: 0.,
            w: 80.,
            h: 170.,
        },
    );
    for (text, scroll) in [
        // f's advance ends just before the left edge, while its ink extends in.
        ("f", f.layout_line("f", px).1 + 0.1),
        // J's origin lies on the right edge, while its negative bearing extends in.
        ("MMMMJ", f.layout_line("MMMM", px).1 - content.w),
    ] {
        let mut i = input(text);
        i.multiline = true;
        i.scroll_x = scroll;
        let mut actual = vec![0; 100 * 180];
        paint_clipped_with_inputs(
            &tree,
            &layout,
            &style_of,
            &f,
            &mut Surface::new(&mut actual, 100, 180),
            Rect {
                x: 0.,
                y: 0.,
                w: 100.,
                h: 180.,
            },
            &|id| (tree.node(id).kind.tag() == "Input").then_some(&i),
        )
        .unwrap();
        let mut expected = vec![0; 100 * 180];
        let mut reference = Surface::new(&mut expected, 100, 180);
        // Independent reference rasterizes every glyph and only clips pixels.
        reference.with_clip(content, |surface| {
            let (glyphs, _) = f.layout_line(text, px);
            let baseline = content.y + f.line_metrics(px).ascent;
            let round = |v: f32| {
                if v < 0. {
                    (v - 0.5) as i32
                } else {
                    (v + 0.5) as i32
                }
            };
            for g in glyphs {
                if let Some(bitmap) = f.rasterize(g.glyph, px) {
                    surface.blit_glyph(
                        &bitmap,
                        round(content.x - scroll + g.x) + bitmap.left,
                        round(baseline) - bitmap.top,
                        dunit_style::value::Color::rgb(255, 255, 255),
                    );
                }
            }
        });
        let edge = if text == "f" { 4 } else { 75 };
        assert!(
            (0..180).any(|y| expected[y * 100 + edge] != 0),
            "reference has ink at {text} viewport edge"
        );
        assert_eq!(actual, expected, "culled raster differs for {text}");
    }
}

#[test]
fn vertical_ink_overhang_matches_full_raster_clipped_reference() {
    let f = font();
    let px = 64.;
    let h = f.line_metrics(px).line_height();
    let sheet = "Input { font-size: 64; color: #ffffff; padding: 0; }";
    let tree = dui(&format!("Column {{ Input width=100 height={h} }}")).unwrap();
    let layout = layout(&tree, 100., 100.);
    let mut cascade = Cascade::new();
    cascade.push(parse(sheet).unwrap());
    let style_of = |id| cascade.resolve(&NodeStyle::element(tree.node(id).kind.tag()));
    let content = Rect {
        x: 0.,
        y: 0.,
        w: 100.,
        h,
    };
    for (text, scroll, edge_band) in [
        // The second row starts at the viewport bottom, yet its accent enters it.
        ("\n\u{1eb2}", 0., 64usize..75),
        // The first row has scrolled out logically, yet its descender remains.
        ("\u{06b8}\n", h, 0usize..11),
    ] {
        let mut i = input(text);
        i.multiline = true;
        i.scroll_y = scroll;
        let mut actual = vec![0; 100 * 100];
        paint_clipped_with_inputs(
            &tree,
            &layout,
            &style_of,
            &f,
            &mut Surface::new(&mut actual, 100, 100),
            Rect {
                x: 0.,
                y: 0.,
                w: 100.,
                h: 100.,
            },
            &|id| (tree.node(id).kind.tag() == "Input").then_some(&i),
        )
        .unwrap();
        let mut expected = vec![0; 100 * 100];
        let mut reference = Surface::new(&mut expected, 100, 100);
        // No row culling: rasterize every logical line, then clip its pixels.
        reference.with_clip(content, |surface| {
            let round = |v: f32| {
                if v < 0. {
                    (v - 0.5) as i32
                } else {
                    (v + 0.5) as i32
                }
            };
            for (row, line) in text.split('\n').enumerate() {
                let baseline = row as f32 * h - scroll + f.line_metrics(px).ascent;
                for glyph in f.layout_line(line, px).0 {
                    if let Some(bitmap) = f.rasterize(glyph.glyph, px) {
                        surface.blit_glyph(
                            &bitmap,
                            round(glyph.x) + bitmap.left,
                            round(baseline) - bitmap.top,
                            dunit_style::value::Color::rgb(255, 255, 255),
                        );
                    }
                }
            }
        });
        assert!(
            edge_band
                .clone()
                .any(|y| (0..100).any(|x| expected[y * 100 + x] != 0)),
            "reference has crossing ink in rows {edge_band:?}"
        );
        assert_eq!(
            actual, expected,
            "row culling loses vertical overhang for {text:?}"
        );
    }
}
