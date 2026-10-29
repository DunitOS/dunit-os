use dunit_component::{
    Application, Attrs, Component, EditCommand as E, Font, InputOptions, Key, Renderer, Sizing,
    Theme, UiError, View,
};
#[derive(Clone)]
enum Msg {
    Text(String),
    Noop,
}
struct Model(String);
impl Component for Model {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        if let Msg::Text(s) = m {
            self.0 = s;
        }
    }
    fn view(&self) -> View<Msg> {
        View::text_area(
            "editor",
            "Document",
            self.0.clone(),
            Msg::Text,
            InputOptions::default(),
        )
        .layout(Attrs {
            width: Sizing::Px(200.),
            height: Sizing::Px(90.),
            ..Attrs::default()
        })
    }
}
fn app(s: &str) -> Application<Model> {
    let mut a = Application::new(Model(s.into())).unwrap();
    a.key(Key::plain('\t')).unwrap();
    a
}
fn renderer() -> Renderer {
    let base = include_str!("../../../assets/dwm/sdk/base.dss");
    let profile = include_str!("../../../assets/dwm/sdk/dark.dss");
    let theme = Theme::parse(profile, base, "").unwrap();
    let font = Font::parse(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/DejaVuSans.ttf"
        ))
        .unwrap(),
    )
    .unwrap();
    Renderer::new(font, theme.cascade, theme.background)
}
#[test]
fn multiline_enter_history_and_line_navigation_are_controlled() {
    let mut a = app("é界");
    a.key(Key::plain('\n')).unwrap();
    a.key(Key::plain('Z')).unwrap();
    assert_eq!(a.component().0, "é界\nZ");
    a.edit(E::Home { extend: false }).unwrap();
    assert_eq!(a.view().text_editor().unwrap().cursor(), "é界\n".len());
    a.edit(E::Backspace { word: false }).unwrap();
    assert_eq!(a.component().0, "é界Z");
    a.edit(E::Undo).unwrap();
    assert_eq!(a.component().0, "é界\nZ");
    a.edit(E::Redo).unwrap();
    assert_eq!(a.component().0, "é界Z");
    a.edit(E::DocumentHome { extend: false }).unwrap();
    assert_eq!(a.view().text_editor().unwrap().cursor(), 0);
    a.edit(E::DocumentEnd { extend: true }).unwrap();
    assert_eq!(a.view().text_editor().unwrap().selection(), 0.."é界Z".len());
}
#[test]
fn multiline_validation_does_not_loosen_single_line_or_password_contract() {
    assert!(
        View::text_area("e", "Editor", "a\nb", Msg::Text, InputOptions::default())
            .validate()
            .is_ok()
    );
    assert_eq!(
        View::input("i", "Input", "a\nb", Msg::Text, InputOptions::default()).validate(),
        Err(UiError::InvalidInput)
    );
    for (value, options) in [
        ("a\rb", InputOptions::default()),
        (
            "a",
            InputOptions {
                password: true,
                ..InputOptions::default()
            },
        ),
    ] {
        assert_eq!(
            View::text_area("e", "Editor", value, Msg::Text, options).validate(),
            Err(UiError::InvalidInput)
        );
    }
}
#[test]
fn vertical_preferred_column_survives_unrelated_model_rebuild() {
    let mut a = app("abcdef\nx\nuvwxyz");
    a.edit(E::DocumentHome { extend: false }).unwrap();
    for _ in 0..4 {
        a.edit(E::Right {
            extend: false,
            word: false,
        })
        .unwrap();
    }
    a.edit(E::Down { extend: false }).unwrap();
    assert_eq!(a.view().text_editor().unwrap().cursor(), 8);
    a.update(Msg::Noop).unwrap();
    a.edit(E::Down { extend: true }).unwrap();
    assert_eq!(a.view().text_editor().unwrap().cursor(), 13);
    assert_eq!(a.view().text_editor().unwrap().selection(), 8..13);
}
#[test]
fn manual_scroll_changes_pixels_and_survives_noop_and_caret_blink() {
    let text = (0..50)
        .map(|i| format!("row {i:02} é界\n"))
        .collect::<String>();
    let mut a = app(&text);
    let mut r = renderer();
    r.render(a.view(), 200, 90).unwrap();
    let end = r.input_scroll_y("editor").unwrap();
    assert!(end > 0.);
    let before = r.pixels().to_vec();
    assert!(r.scroll_input("editor", -5.));
    assert!(r.render(a.view(), 200, 90).unwrap().damage.is_some());
    let scrolled = r.input_scroll_y("editor").unwrap();
    assert!(scrolled < end);
    assert_ne!(r.pixels(), before);
    assert_eq!(r.render(a.view(), 200, 90).unwrap().damaged_pixels, 0);
    a.blink_caret();
    r.render(a.view(), 200, 90).unwrap();
    assert_eq!(r.input_scroll_y("editor"), Some(scrolled));
    a.update(Msg::Noop).unwrap();
    r.render(a.view(), 200, 90).unwrap();
    assert_eq!(r.input_scroll_y("editor"), Some(scrolled));
    a.edit(E::DocumentHome { extend: false }).unwrap();
    r.render(a.view(), 200, 90).unwrap();
    assert_eq!(r.input_scroll_y("editor"), Some(0.));
    assert!(r.scroll_input_wheel("editor", 1));
    r.render(a.view(), 200, 90).unwrap();
    assert!(r.input_scroll_y("editor").unwrap() > 0.);
    // Home is meaningful even if the insertion index is already zero.
    a.edit(E::DocumentHome { extend: false }).unwrap();
    r.render(a.view(), 200, 90).unwrap();
    assert_eq!(r.input_scroll_y("editor"), Some(0.));
}
#[test]
fn multiline_hit_testing_and_incremental_repaint_match_fresh_canvas() {
    let mut a = app("é界\nsecond\nthird");
    a.edit(E::DocumentHome { extend: false }).unwrap();
    let mut r = renderer();
    r.render(a.view(), 200, 90).unwrap();
    let first = r.input_index_at("editor", 6., 6.).unwrap();
    let second = r.input_index_at("editor", 6., 25.).unwrap();
    assert_eq!(first, 0);
    assert_eq!(second, "é界\n".len());
    for c in [
        E::Down { extend: false },
        E::End { extend: true },
        E::Insert('✓'),
        E::Insert('\n'),
        E::Undo,
    ] {
        a.edit(c).unwrap();
        r.render(a.view(), 200, 90).unwrap();
        let mut full = renderer();
        full.render(a.view(), 200, 90).unwrap();
        assert_eq!(r.pixels(), full.pixels());
    }
}

#[test]
fn rejected_change_cannot_restore_an_editor_of_the_previous_mode() {
    struct Switching {
        multiline: bool,
    }
    impl Component for Switching {
        type Message = String;
        fn update(&mut self, _: String) {
            self.multiline = !self.multiline;
        }
        fn view(&self) -> View<String> {
            if self.multiline {
                View::text_area(
                    "field",
                    "Field",
                    "A",
                    core::convert::identity,
                    InputOptions::default(),
                )
            } else {
                View::input(
                    "field",
                    "Field",
                    "A",
                    core::convert::identity,
                    InputOptions::default(),
                )
            }
        }
    }
    for initial in [false, true] {
        let mut a = Application::new(Switching { multiline: initial }).unwrap();
        a.key(Key::plain('\t')).unwrap();
        a.key(Key::plain('x')).unwrap();
        assert_eq!(a.view().text_editor().unwrap().is_multiline(), !initial);
        assert_eq!(a.focused_input_key(), None);
        assert_eq!(a.view().text_editor().unwrap().text(), "A");
        assert!(a.view().validate().is_ok());
    }
}

#[test]
fn same_index_pointer_activity_requests_redraw_after_manual_scroll() {
    let mut a = app(&(0..50).map(|i| format!("row {i}\n")).collect::<String>());
    let mut r = renderer();
    a.edit(E::DocumentHome { extend: false }).unwrap();
    r.render(a.view(), 200, 90).unwrap();
    assert!(r.scroll_input("editor", 5.));
    r.render(a.view(), 200, 90).unwrap();
    assert!(r.input_scroll_y("editor").unwrap() > 0.);
    // Establish hover before the revision-only click.
    a.pointer(dunit_component::PointerEvent::Move, Some("editor"))
        .unwrap();
    assert!(a
        .pointer_at(
            dunit_component::PointerEvent::Down,
            Some("editor"),
            Some(("editor", 0))
        )
        .unwrap());
    r.render(a.view(), 200, 90).unwrap();
    assert_eq!(r.input_scroll_y("editor"), Some(0.));
    assert!(a
        .pointer_at(
            dunit_component::PointerEvent::Up,
            Some("editor"),
            Some(("editor", 0))
        )
        .unwrap());
}

#[test]
fn theme_scroll_step_controls_wheel_distance_and_can_disable_it() {
    let profile = include_str!("../../../assets/dwm/sdk/dark.dss");
    let base = include_str!("../../../assets/dwm/sdk/base.dss");
    for invalid in ["-1", "1025", "red"] {
        assert!(Theme::parse(
            profile,
            base,
            &format!("Textarea {{ scroll-step: {invalid}; }}")
        )
        .is_err());
    }
    let mut a = app(&(0..50).map(|i| format!("row {i}\n")).collect::<String>());
    a.edit(E::DocumentHome { extend: false }).unwrap();
    let mut distances = Vec::new();
    for step in [0, 1, 2] {
        let theme = Theme::parse(
            profile,
            base,
            &format!("Textarea {{ scroll-step: {step}; }}"),
        )
        .unwrap();
        let mut r = renderer();
        r.set_theme(theme, None);
        r.render(a.view(), 200, 90).unwrap();
        assert_eq!(r.scroll_input_wheel("editor", 1), step != 0);
        r.render(a.view(), 200, 90).unwrap();
        distances.push(r.input_scroll_y("editor").unwrap());
        r.scroll_input_wheel("editor", -1);
        r.render(a.view(), 200, 90).unwrap();
        assert_eq!(r.input_scroll_y("editor"), Some(0.));
    }
    assert_eq!(distances[0], 0.);
    assert!(distances[1] > 0.);
    assert_eq!(distances[2], distances[1] * 2.);
}
