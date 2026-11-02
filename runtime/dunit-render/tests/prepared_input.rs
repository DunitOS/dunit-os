use dunit_render::{
    paint_clipped_with_inputs, paint_clipped_with_prepared_inputs, CaretAffinity, Surface,
    TextInput,
};
use dunit_style::{cascade::NodeStyle, parse, Cascade};
use dunit_text::Font;
use dunit_ui::{layout, parse as dui, Rect};
use dunit_widgets::text_input::EditCommand;
fn input(value: &str, multiline: bool, password: bool) -> TextInput {
    TextInput {
        spinbox: None,
        value: value.into(),
        placeholder: "Hint e\u{301} alpha beta gamma".into(),
        password,
        multiline,
        soft_wrap: multiline,
        affinity: CaretAffinity::Downstream,
        cursor: value.len(),
        anchor: 0,
        focused: true,
        caret_visible: true,
        scroll_x: 0.,
        scroll_y: 0.,
    }
}
#[test]
fn prepared_and_standalone_geometry_and_pixels_match_on_resize_mask_and_placeholder() {
    let font =
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap();
    let mut cascade = Cascade::new();
    cascade.push(parse("Stack {background:#123456;} Input,Textarea {color:#ffffff;placeholder-color:#00ff00;selection-color:#334455;caret-color:#ff0000;caret-width:2;font-size:16;padding:3;border-width:1;border-color:#224466;scrollbar-width:8;}").unwrap());
    for (value, multiline, password) in [
        ("alpha beta gamma\n\ne\u{301}👩‍💻 tail", true, false),
        ("e\u{301}👩‍💻 wide password", false, true),
        ("", false, true),
        ("", true, false),
    ] {
        let tag = if multiline { "Textarea" } else { "Input" };
        let tree = dui(&format!("Stack {{ {tag} #field width=grow height=grow }}")).unwrap();
        let id = tree.by_name("field").unwrap();
        let style_of = |id| cascade.resolve(&NodeStyle::element(tree.node(id).kind.tag()));
        for (width, height) in [(180, 100), (65, 45), (180, 100)] {
            let laid = layout(&tree, width as f32, height as f32);
            let rect = laid.rect(id);
            let style = style_of(id);
            for affinity in [CaretAffinity::Upstream, CaretAffinity::Downstream] {
                let mut ordinary = input(value, multiline, password);
                let first = ordinary.prepare(&font, &style, rect);
                if multiline && !value.is_empty() {
                    ordinary.cursor = first.lines().lines[0].end;
                }
                ordinary.affinity = affinity;
                let mut prepared_input = ordinary.clone();
                let prepared = prepared_input.prepare(&font, &style, rect);
                ordinary.fit_scroll(&font, &style, rect);
                prepared_input.fit_scroll_with_lines(&font, &style, rect, prepared.lines());
                assert_eq!(
                    (ordinary.scroll_x, ordinary.scroll_y),
                    (prepared_input.scroll_x, prepared_input.scroll_y)
                );
                assert_eq!(
                    ordinary.scroll_axes(&font, &style, rect),
                    prepared_input.scroll_axes_with_lines(&font, &style, rect, prepared.lines())
                );
                for (x, y) in [
                    (-10., -10.),
                    (20., 20.),
                    (width as f32 - 1., height as f32 - 1.),
                    (10000., 10000.),
                ] {
                    assert_eq!(
                        ordinary.hit_position_at(&font, &style, rect, x, y),
                        prepared_input.hit_position_with_lines(
                            &font,
                            &style,
                            rect,
                            x,
                            y,
                            prepared.lines()
                        )
                    );
                }
                for command in [
                    EditCommand::Home { extend: false },
                    EditCommand::End { extend: true },
                    EditCommand::Up { extend: true },
                    EditCommand::Down { extend: false },
                ] {
                    assert_eq!(
                        ordinary.visual_navigation(&font, &style, rect, command, Some(13.)),
                        prepared_input.visual_navigation_with_lines(
                            command,
                            Some(13.),
                            prepared.lines()
                        )
                    );
                }
                for clip in [
                    Rect {
                        x: 0.,
                        y: 0.,
                        w: width as f32,
                        h: height as f32,
                    },
                    Rect {
                        x: 11.,
                        y: 9.,
                        w: 29.,
                        h: 19.,
                    },
                ] {
                    let mut old = vec![0; width * height];
                    let mut shared = old.clone();
                    paint_clipped_with_inputs(
                        &tree,
                        &laid,
                        &style_of,
                        &font,
                        &mut Surface::new(&mut old, width, height),
                        clip,
                        &|n| (n == id).then_some(&ordinary),
                    )
                    .unwrap();
                    paint_clipped_with_prepared_inputs(
                        &tree,
                        &laid,
                        &style_of,
                        &font,
                        &mut Surface::new(&mut shared, width, height),
                        clip,
                        &|n| (n == id).then_some(&prepared_input),
                        &|_| None,
                        &|_| None,
                        &|_| None,
                        &|_| None,
                        &|n| (n == id).then_some(&prepared),
                    )
                    .unwrap();
                    assert_eq!(
                        old, shared,
                        "{tag} {width}x{height} {affinity:?} password={password} value={value:?}"
                    );
                }
            }
        }
    }
}
