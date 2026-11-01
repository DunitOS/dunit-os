use dunit_component::{
    Application, Component, DialogAction, InputOptions, Key, PointerEvent as P, SpinboxOptions,
    View,
};
#[derive(Clone)]
enum Msg {
    Act,
    Text(String),
    Open(i64),
    Close,
    Show,
}
struct Model {
    calls: usize,
    modal: bool,
    value: i64,
}
impl Component for Model {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Act => self.calls += 1,
            Msg::Open(v) => {
                self.value = v;
                self.modal = true
            }
            Msg::Close => self.modal = false,
            Msg::Show => self.modal = true,
            Msg::Text(s) => {
                let _ = s;
            }
        }
    }
    fn view(&self) -> View<Msg> {
        View::modal(
            "host",
            View::column(
                "base",
                vec![
                    View::button("normal", "Normal", Msg::Act),
                    View::button("skip", "Skip", Msg::Act).tab_index(-1),
                    View::button("second", "Second", Msg::Act).tab_index(2),
                    View::button("first", "First", Msg::Act)
                        .tab_index(1)
                        .mnemonic('f'),
                    View::button("disabled", "Disabled", Msg::Act)
                        .tab_index(1)
                        .mnemonic('d')
                        .disabled(true),
                    View::input("input", "Input", "", Msg::Text, InputOptions::default())
                        .mnemonic('i'),
                    View::spinbox(
                        "spin",
                        "Value",
                        self.value,
                        SpinboxOptions::new(0, 100, 1),
                        Msg::Open,
                    ),
                ],
            ),
            self.modal.then(|| {
                View::dialog(
                    "dialog",
                    "Confirm",
                    View::button("inside", "Inside", Msg::Act).mnemonic('f'),
                    vec![DialogAction::new("close", "Close", Msg::Close)],
                    None,
                    Some(Msg::Close),
                )
            }),
        )
    }
}
fn app() -> Application<Model> {
    Application::new(Model {
        calls: 0,
        modal: false,
        value: 0,
    })
    .unwrap()
}
#[test]
fn tab_order_is_stable_positive_then_document_and_negative_is_pointer_only() {
    let mut a = app();
    for expected in ["first", "second", "normal", "input", "spin", "first"] {
        a.key(Key::plain('\t')).unwrap();
        assert_eq!(a.focused_key(), Some(expected));
    }
    a.key(Key {
        character: '\t',
        modifiers: 1,
    })
    .unwrap();
    assert_eq!(a.focused_key(), Some("spin"));
    a.pointer(P::Down, Some("skip")).unwrap();
    assert_eq!(a.focused_key(), Some("skip"));
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("first"));
}
#[test]
fn mnemonic_focuses_input_activates_button_and_skips_disabled() {
    let mut a = app();
    a.key(Key {
        character: 'f',
        modifiers: 4,
    })
    .unwrap();
    assert_eq!(a.component().calls, 1);
    assert_eq!(a.focused_key(), Some("first"));
    a.key(Key {
        character: 'i',
        modifiers: 4,
    })
    .unwrap();
    assert_eq!(a.focused_key(), Some("input"));
    assert!(!a
        .key(Key {
            character: 'd',
            modifiers: 4
        })
        .unwrap());
    assert_eq!(a.component().calls, 1);
}
#[test]
fn mnemonic_is_scoped_to_modal_and_escape_restores_opener() {
    let mut a = app();
    a.pointer(P::Down, Some("normal")).unwrap();
    a.update(Msg::Show).unwrap();
    a.key(Key {
        character: 'f',
        modifiers: 4,
    })
    .unwrap();
    assert_eq!(a.focused_key(), Some("inside"));
    assert_eq!(a.component().calls, 1);
    a.key(Key::plain('\x1b')).unwrap();
    assert_eq!(a.focused_key(), Some("normal"));
}
#[test]
fn committing_spinbox_cannot_route_the_same_press_behind_new_dialog() {
    let mut a = app();
    a.pointer(P::Down, Some("spin")).unwrap();
    a.key(Key {
        character: 'a',
        modifiers: 2,
    })
    .unwrap();
    a.key(Key::plain('7')).unwrap();
    a.pointer(P::Down, Some("normal")).unwrap();
    assert!(a.component().modal);
    assert_ne!(a.focused_key(), Some("normal"));
    a.pointer(P::Up, Some("normal")).unwrap();
    assert_eq!(a.component().calls, 0);
}

#[test]
fn negative_roving_member_does_not_remove_the_entire_group_from_tab_order() {
    struct Group;
    impl Component for Group {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self) -> View<()> {
            View::column(
                "root",
                vec![
                    View::radio("skipped", "Skipped", "g", true, ()).tab_index(-1),
                    View::radio("member", "Member", "g", false, ()),
                    View::toolbar(
                        "tools",
                        "Tools",
                        vec![
                            View::button("skip-tool", "Skip", ()).tab_index(-1),
                            View::button("tool", "Tool", ()),
                        ],
                    ),
                ],
            )
        }
    }
    let mut app = Application::new(Group).unwrap();
    app.key(Key::plain('\t')).unwrap();
    assert_eq!(app.focused_key(), Some("member"));
    app.key(Key::plain('\t')).unwrap();
    assert_eq!(app.focused_key(), Some("tool"));
    app.key(Key::plain('\t')).unwrap();
    assert_eq!(app.focused_key(), Some("member"));
}
