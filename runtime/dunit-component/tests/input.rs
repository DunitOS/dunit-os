use dunit_component::{
    Application, Attrs, Component, EditCommand as E, InputOptions, Key, PointerEvent as P, Sizing,
    UiError, View,
};
#[derive(Clone)]
enum Msg {
    Text(String),
    External(String),
    Reject,
    Disable,
    Reverse,
}
struct Model {
    value: String,
    reject: bool,
    disabled: bool,
    reverse: bool,
    calls: usize,
}
impl Component for Model {
    type Message = Msg;
    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Text(value) => {
                self.calls += 1;
                if !self.reject {
                    self.value = value;
                }
            }
            Msg::External(v) => self.value = v,
            Msg::Reject => self.reject = true,
            Msg::Disable => self.disabled = true,
            Msg::Reverse => self.reverse = !self.reverse,
        }
    }
    fn view(&self) -> View<Msg> {
        let mut children = vec![
            View::input(
                "input",
                "Name",
                self.value.clone(),
                Msg::Text,
                InputOptions {
                    placeholder: "Type a name".into(),
                    ..InputOptions::default()
                },
            )
            .disabled(self.disabled),
            View::button("button", "Action", Msg::Reject),
        ];
        if self.reverse {
            children.reverse();
        }
        View::column("root", children)
    }
}
fn app() -> Application<Model> {
    Application::new(Model {
        value: "Aé界".into(),
        reject: false,
        disabled: false,
        reverse: false,
        calls: 0,
    })
    .unwrap()
}
fn editor(app: &Application<Model>) -> &dunit_component::TextEditor {
    app.view()
        .children()
        .iter()
        .find(|v| v.key() == "input")
        .unwrap()
        .text_editor()
        .unwrap()
}
#[test]
fn typing_utf8_selection_undo_redo_and_model_messages() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_input_key(), Some("input"));
    a.edit(E::Left {
        extend: true,
        word: false,
    })
    .unwrap();
    a.key(Key::plain('✓')).unwrap();
    assert_eq!(a.component().value, "Aé✓");
    a.key(Key {
        character: '\u{1a}',
        modifiers: 2,
    })
    .unwrap();
    assert_eq!(a.component().value, "Aé界");
    assert_eq!(editor(&a).selection(), 3..6);
    a.key(Key {
        character: 'z',
        modifiers: 3,
    })
    .unwrap();
    assert_eq!(a.component().value, "Aé✓");
    a.key(Key {
        character: '\u{1}',
        modifiers: 2,
    })
    .unwrap();
    a.key(Key::plain('X')).unwrap();
    assert_eq!(a.component().value, "X");
    assert_eq!(a.component().calls, 4);
}
#[test]
fn rejection_external_updates_and_disabled_fields_do_not_hide_state_changes() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.update(Msg::Reject).unwrap();
    a.key(Key::plain('x')).unwrap();
    assert_eq!(a.component().value, "Aé界");
    assert_eq!(editor(&a).text(), "Aé界");
    assert!(!editor(&a).can_undo());
    a.update(Msg::External("external".into())).unwrap();
    assert_eq!(editor(&a).text(), "external");
    a.update(Msg::Disable).unwrap();
    assert_eq!(a.focused_input_key(), None);
    a.pointer_at(P::Down, Some("input"), Some(("input", 0)))
        .unwrap();
    assert_eq!(a.focused_input_key(), None);
}
#[test]
fn cursor_and_history_survive_reorder_and_pointer_drag_selects_scalar_ranges() {
    let mut a = app();
    a.pointer_at(P::Down, Some("input"), Some(("input", 1)))
        .unwrap();
    a.pointer_at(P::Move, Some("button"), Some(("input", 6)))
        .unwrap();
    a.pointer_at(P::Up, Some("button"), Some(("input", 6)))
        .unwrap();
    assert_eq!(editor(&a).selection(), 1..6);
    a.key(Key::plain('!')).unwrap();
    assert_eq!(a.component().value, "A!");
    a.update(Msg::Reverse).unwrap();
    assert_eq!(a.focused_key(), Some("input"));
    a.edit(E::Undo).unwrap();
    assert_eq!(a.component().value, "Aé界");
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("button"));
}
#[test]
fn constraints_read_only_and_accessibility_are_explicit() {
    struct Field {
        value: String,
        options: InputOptions,
    }
    impl Component for Field {
        type Message = String;
        fn update(&mut self, v: String) {
            self.value = v;
        }
        fn view(&self) -> View<String> {
            View::input(
                "i",
                "Label",
                self.value.clone(),
                core::convert::identity,
                self.options.clone(),
            )
        }
    }
    let make = |v: &str, o: InputOptions| {
        Application::new(Field {
            value: v.into(),
            options: o,
        })
    };
    assert!(matches!(
        make("bad\n", InputOptions::default()),
        Err(UiError::InvalidInput)
    ));
    assert!(matches!(
        make(
            "four",
            InputOptions {
                max_chars: 3,
                ..InputOptions::default()
            }
        ),
        Err(UiError::InvalidInput)
    ));
    let mut a = make(
        "é",
        InputOptions {
            max_chars: 1,
            ..InputOptions::default()
        },
    )
    .unwrap();
    a.key(Key::plain('\t')).unwrap();
    a.key(Key::plain('界')).unwrap();
    assert_eq!(a.component().value, "é");
    let mut a = make(
        "fixed",
        InputOptions {
            read_only: true,
            password: true,
            ..InputOptions::default()
        },
    )
    .unwrap();
    a.key(Key::plain('\t')).unwrap();
    a.edit(E::SelectAll).unwrap();
    a.key(Key::plain('x')).unwrap();
    assert_eq!(a.component().value, "fixed");
    assert_eq!(a.view().accessibility_role(), Some("textbox"));
    assert_eq!(a.view().accessibility_label(), Some("Label"));
}
#[test]
fn cursor_scroll_blink_and_incremental_input_pixels_match_full_render() {
    use dunit_component::{Font, Renderer, Theme};
    let base = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/dwm/sdk/base.dss"
    ))
    .unwrap();
    let profile = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/dwm/sdk/dark.dss"
    ))
    .unwrap();
    let make = || {
        let theme = Theme::parse(&profile, &base, "").unwrap();
        let font = Font::parse(
            std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/fonts/DejaVuSans.ttf"
            ))
            .unwrap(),
        )
        .unwrap();
        Renderer::new(font, theme.cascade, theme.background)
    };
    struct Field(String);
    impl Component for Field {
        type Message = String;
        fn update(&mut self, v: String) {
            self.0 = v;
        }
        fn view(&self) -> View<String> {
            View::input(
                "i",
                "Label",
                self.0.clone(),
                core::convert::identity,
                InputOptions::default(),
            )
            .layout(Attrs {
                width: Sizing::Px(180.),
                height: Sizing::Px(40.),
                ..Attrs::default()
            })
        }
    }
    let mut a = Application::new(Field("long Unicode 界 ".repeat(12))).unwrap();
    a.key(Key::plain('\t')).unwrap();
    let mut r = make();
    r.render(a.view(), 180, 40).unwrap();
    assert!(r.input_scroll_x("i").unwrap() > 0.);
    assert_eq!(r.caret_blink_ms(), Some(500));
    let hit = r.input_index("i", -100.).unwrap();
    assert!(a.view().content().unwrap().is_char_boundary(hit));

    for command in [
        E::Home { extend: false },
        E::Right {
            extend: true,
            word: true,
        },
        E::Insert('✓'),
        E::Undo,
    ] {
        a.edit(command).unwrap();
        r.render(a.view(), 180, 40).unwrap();
        // A fresh render starts its viewport at zero; compare at Home, where
        // both viewport histories settle at the beginning of the same line.
        a.edit(E::Home { extend: false }).unwrap();
        r.render(a.view(), 180, 40).unwrap();
        let mut full = make();
        full.render(a.view(), 180, 40).unwrap();
        assert_eq!(r.pixels(), full.pixels());
    }
    let before = r.pixels().to_vec();
    assert!(a.blink_caret());
    assert!(r.render(a.view(), 180, 40).unwrap().damage.is_some());
    assert_ne!(r.pixels(), before);
    let mut full = make();
    full.render(a.view(), 180, 40).unwrap();
    assert_eq!(r.pixels(), full.pixels());
    let steady = Theme::parse(&profile, &base, "Input {caret-blink:0ms;}").unwrap();
    r.set_theme(steady, None);
    r.render(a.view(), 180, 40).unwrap();
    assert_eq!(r.caret_blink_ms(), None);
    assert_eq!(r.pixels(), before);
    for style in [
        "Input {caret-blink:wrong;}",
        "Input {caret-width:-1;}",
        "Input {selection-color:12;}",
        "Input {selection-scroll-interval:32;}",
        "Input {selection-scroll-interval:-1ms;}",
        "Input {selection-scroll-interval:60001ms;}",
        "Input {selection-scroll-step:-1;}",
        "Input {selection-scroll-step:1025;}",
    ] {
        assert!(Theme::parse(&profile, &base, style).is_err());
    }
    assert!(Theme::parse(&profile, &base, "Input {selection-scroll-interval:0ms; selection-scroll-step:0;}").is_ok());
}

#[test]
fn rejected_proposal_keeps_prior_history_and_selection() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.key(Key::plain('a')).unwrap();
    a.edit(E::SelectAll).unwrap();
    let before = editor(&a).clone();
    a.update(Msg::Reject).unwrap();
    a.key(Key::plain('x')).unwrap();
    assert_eq!(editor(&a), &before);
    assert!(editor(&a).can_undo());
    a.blink_caret();
    assert!(a
        .pointer_at(P::Move, Some("input"), Some(("input", editor(&a).cursor())))
        .unwrap());
}
