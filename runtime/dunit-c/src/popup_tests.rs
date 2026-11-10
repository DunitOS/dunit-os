use super::*;
use core::ptr;
use PopupCommand as P;
fn node(kind: u32, parent: u32, key: &str, text: &str, id: u64) -> DunitUiNodeV1 {
    DunitUiNodeV1 {
        kind,
        parent,
        key: key.as_ptr(),
        key_len: key.len(),
        text: text.as_ptr(),
        text_len: text.len(),
        message: id,
    }
}
#[derive(Default)]
struct State {
    actions: usize,
    selected: bool,
    choices: usize,
    declined: usize,
    resets: usize,
    status: String,
}
fn nodes(s: &mut State) -> Vec<DunitUiNodeV1> {
    s.status = alloc::format!(
        "Actions {} / color {}",
        s.actions,
        if s.selected { "blue" } else { "red" }
    );
    vec![
        node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0),
        node(DUNIT_UI_TEXT, 0, "title", "C SDK menus and popups", 0),
        node(DUNIT_UI_MENU_BAR, 0, "bar", "", 0),
        node(DUNIT_UI_MENU, 2, "file", "File", 0),
        node(DUNIT_UI_MNEMONIC, 3, "file-key", "f", 0),
        node(DUNIT_UI_MENU_ITEM, 3, "new", "New", 10),
        node(
            DUNIT_UI_MENU_ITEM | DUNIT_UI_DISABLED,
            3,
            "unavailable",
            "Unavailable",
            999,
        ),
        node(DUNIT_UI_MENU_SEPARATOR, 3, "separator", "", 0),
        node(DUNIT_UI_SUBMENU, 3, "recent", "Recent", 0),
        node(DUNIT_UI_MNEMONIC, 8, "recent-key", "r", 0),
        node(DUNIT_UI_MENU_ITEM, 8, "project", "Project", u64::MAX),
        node(DUNIT_UI_MENU, 2, "tools", "Tools", 0),
        node(DUNIT_UI_MNEMONIC, 11, "tools-key", "t", 0),
        node(DUNIT_UI_MENU_ITEM, 11, "tools-reset", "Reset", 20),
        node(DUNIT_UI_SELECT, 0, "color", "Choose color", 0),
        node(
            if !s.selected {
                DUNIT_UI_OPTION_SELECTED
            } else {
                DUNIT_UI_OPTION
            },
            14,
            "red",
            "Red",
            30,
        ),
        node(
            DUNIT_UI_OPTION | DUNIT_UI_DISABLED,
            14,
            "green",
            "Green unavailable",
            999,
        ),
        node(
            if s.selected {
                DUNIT_UI_OPTION_SELECTED
            } else {
                DUNIT_UI_OPTION
            },
            14,
            "blue",
            "Blue",
            31,
        ),
        node(
            DUNIT_UI_BUTTON,
            0,
            "surface",
            "Context actions (Shift+F10)",
            51,
        ),
        node(DUNIT_UI_CONTEXT_MENU, 18, "context-binding", "", 0),
        node(DUNIT_UI_MENU_ITEM, 19, "inspect", "Inspect", 40),
        node(
            DUNIT_UI_MENU_ITEM | DUNIT_UI_DISABLED,
            19,
            "context-disabled",
            "Unavailable",
            999,
        ),
        node(DUNIT_UI_BUTTON, 0, "help", "Hover for SDK tooltip", 50),
        node(
            DUNIT_UI_TOOLTIP,
            22,
            "help-tooltip",
            "Tooltips use the shared SDK timer",
            0,
        ),
        node(DUNIT_UI_ROW, 0, "actions", "", 0),
        node(DUNIT_UI_BUTTON, 24, "reset", "Reset", 20),
        node(DUNIT_UI_BUTTON, 24, "quit", "Quit", 8),
        node(DUNIT_UI_TEXT, 0, "status", &s.status, 0),
    ]
}
impl State {
    fn action(&mut self, id: u64) -> i32 {
        match id {
            8 => 1,
            10 | u64::MAX | 40 => {
                self.actions += 1;
                0
            }
            31 => {
                self.selected = true;
                self.choices += 1;
                0
            }
            30 if self.selected => {
                self.declined += 1;
                0
            }
            20 => {
                self.actions = 0;
                self.selected = false;
                self.resets += 1;
                0
            }
            _ => panic!("disabled or unexpected action"),
        }
    }
}
unsafe extern "C" fn build(user: *mut c_void, out: *mut DunitUiNodeV1, capacity: u32) -> i32 {
    let nodes = nodes(&mut *(user as *mut State));
    assert!(capacity >= nodes.len() as u32);
    ptr::copy_nonoverlapping(nodes.as_ptr(), out, nodes.len());
    nodes.len() as i32
}
unsafe extern "C" fn update(user: *mut c_void, id: u64) -> i32 {
    (&mut *(user as *mut State)).action(id)
}
fn model(s: &mut State) -> Model {
    unsafe {
        Model::new(DunitUiCallbacksV1 {
            abi_version: 1,
            struct_size: 32,
            user: s as *mut _ as *mut c_void,
            build: Some(build),
            update: Some(update),
        })
    }
    .unwrap()
}
fn alt(ch: char) -> Key {
    Key {
        character: ch,
        modifiers: 4,
    }
}
#[test]
fn nested_menu_order_mnemonics_disabled_separators_and_full_ids_are_sdk_owned() {
    let mut s = State::default();
    let mut app = Application::new(model(&mut s)).unwrap();
    app.key(alt('f')).unwrap();
    assert_eq!(
        app.popup_items(&[])
            .unwrap()
            .iter()
            .map(|p| p.key.as_str())
            .collect::<Vec<_>>(),
        vec!["new", "unavailable", "separator", "recent"]
    );
    app.popup_command(P::Down).unwrap();
    assert_eq!(app.popup_state().unwrap().path, vec![3]);
    app.popup_command(P::Right).unwrap();
    assert_eq!(app.popup_state().unwrap().path, vec![3, 0]);
    app.popup_command(P::Activate).unwrap();
    assert_eq!(s.actions, 1);
    assert_eq!(app.focused_key(), Some("file"));
    app.key(alt('f')).unwrap();
    app.key(Key::plain('r')).unwrap();
    assert_eq!(app.popup_state().unwrap().path, vec![3, 0]);
    app.popup_command(P::Escape).unwrap();
    assert_eq!(app.popup_state().unwrap().path, vec![3]);
    app.popup_command(P::Escape).unwrap();
    assert!(app.popup_state().is_none());
}
#[test]
fn select_is_controlled_and_a_declined_choice_preserves_model_selection() {
    let mut s = State::default();
    let mut app = Application::new(model(&mut s)).unwrap();
    app.popup_command(P::MenuBar).unwrap();
    app.key(Key::plain('\t')).unwrap();
    assert_eq!(app.focused_key(), Some("color"));
    app.key(Key::plain(' ')).unwrap();
    app.popup_command(P::Down).unwrap();
    assert_eq!(app.popup_state().unwrap().path, vec![2]);
    app.popup_command(P::Activate).unwrap();
    assert!(s.selected);
    assert_eq!(s.choices, 1);
    app.key(Key::plain(' ')).unwrap();
    app.popup_command(P::Home).unwrap();
    app.popup_command(P::Activate).unwrap();
    assert!(s.selected);
    assert_eq!(s.declined, 1);
    assert_eq!(app.view().children()[2].selected_option(), Some("blue"));
}
#[test]
fn malformed_descriptor_parents_duplicates_and_selected_flags_are_rejected() {
    for case in 0..7 {
        let mut s = State::default();
        let mut n = nodes(&mut s);
        match case {
            0 => n[5].parent = 0,
            1 => n[15].parent = 3,
            2 => n[17].kind = DUNIT_UI_OPTION_SELECTED,
            3 => {
                n[20].key = n[5].key;
                n[20].key_len = n[5].key_len;
            }
            4 => n[4].text_len = 0,
            5 => n[7].kind |= DUNIT_UI_DISABLED,
            _ => n[23].parent = 5,
        };
        assert!(unsafe { tree::convert(&n) }.is_err());
    }
    for kind in [
        DUNIT_UI_MENU_ITEM,
        DUNIT_UI_MENU_SEPARATOR,
        DUNIT_UI_OPTION,
        DUNIT_UI_CONTEXT_MENU,
        DUNIT_UI_TOOLTIP,
        DUNIT_UI_MNEMONIC,
    ] {
        assert!(unsafe { tree::convert(&[node(kind, DUNIT_UI_ROOT, "root", "", 0)]) }.is_err());
    }
}
#[test]
fn metadata_is_single_bound_and_menu_bar_children_use_the_sdk_validator() {
    for kind in [DUNIT_UI_CONTEXT_MENU, DUNIT_UI_TOOLTIP, DUNIT_UI_MNEMONIC] {
        let host = if kind == DUNIT_UI_MNEMONIC {
            DUNIT_UI_MENU
        } else {
            DUNIT_UI_BUTTON
        };
        let n = [
            node(host, DUNIT_UI_ROOT, "root", "Host", 0),
            node(kind, 0, "one", "x", 0),
            node(kind, 0, "two", "y", 0),
        ];
        assert!(unsafe { tree::convert(&n) }.is_err());
    }
    let n = [
        node(DUNIT_UI_MENU_BAR, DUNIT_UI_ROOT, "root", "", 0),
        node(DUNIT_UI_BUTTON, 0, "wrong", "Button", 1),
    ];
    assert!(unsafe { tree::convert(&n) }.is_err());
    let mut n = vec![node(DUNIT_UI_MENU, DUNIT_UI_ROOT, "root", "Menu", 0)];
    let keys: Vec<_> = (0..17).map(|i| alloc::format!("sub{i}")).collect();
    for (i, key) in keys.iter().enumerate() {
        n.push(node(DUNIT_UI_SUBMENU, i as u32, key, "Submenu", 0));
    }
    assert!(unsafe { tree::convert(&n) }.is_err());
}
#[test]
fn foreign_item_labels_tooltip_and_mnemonics_are_owned_and_validated() {
    let mut label = b"New".to_vec();
    let mut tooltip = b"Tooltips use the shared SDK timer".to_vec();
    let mut s = State::default();
    let mut n = nodes(&mut s);
    n[5].text = label.as_ptr();
    n[23].text = tooltip.as_ptr();
    let view = unsafe { tree::convert(&n) }.unwrap();
    label.fill(b'x');
    tooltip.fill(b'y');
    assert_eq!(
        view.children()[4].accessibility_description(),
        Some("Tooltips use the shared SDK timer")
    );
    struct Fixed(View<u64>);
    impl Component for Fixed {
        type Message = u64;
        fn update(&mut self, _: u64) {}
        fn view(&self) -> View<u64> {
            self.0.clone()
        }
    }
    let mut app = Application::new(Fixed(view)).unwrap();
    app.key(alt('f')).unwrap();
    assert_eq!(app.popup_items(&[]).unwrap()[0].label, "New");
    for idx in [4, 5, 15, 23] {
        let mut s = State::default();
        let mut bad = nodes(&mut s);
        bad[idx].text = b"\xff".as_ptr();
        bad[idx].text_len = 1;
        assert!(unsafe { tree::convert(&bad) }.is_err());
    }
}
#[test]
fn disabled_triggers_and_context_tooltip_owners_use_normal_sdk_rules() {
    let mut s = State::default();
    let mut n = nodes(&mut s);
    n[3].kind |= DUNIT_UI_DISABLED;
    n[14].kind |= DUNIT_UI_DISABLED;
    n[18].kind |= DUNIT_UI_DISABLED;
    n[22].kind |= DUNIT_UI_DISABLED;
    struct Fixed(View<u64>);
    impl Component for Fixed {
        type Message = u64;
        fn update(&mut self, _: u64) {}
        fn view(&self) -> View<u64> {
            self.0.clone()
        }
    }
    let mut app = Application::new(Fixed(unsafe { tree::convert(&n) }.unwrap())).unwrap();
    assert!(!app.open_context_menu("surface", None));
    app.tooltip_pointer(Some("help"), 0);
    assert!(!app.sample_tooltip(1000, 100));
    app.popup_command(P::MenuBar).unwrap();
    assert_eq!(app.focused_key(), Some("tools"));
}
#[test]
fn context_action_restores_focus_and_tooltip_obeys_the_shared_deadline() {
    let mut s = State::default();
    let mut app = Application::new(model(&mut s)).unwrap();
    app.pointer(PointerEvent::Down, Some("surface")).unwrap();
    app.pointer(PointerEvent::Leave, None).unwrap();
    assert!(app.open_context_menu("surface", None));
    app.popup_command(P::Activate).unwrap();
    assert_eq!(s.actions, 1);
    assert_eq!(app.focused_key(), Some("surface"));
    app.tooltip_pointer(Some("help"), 100);
    assert!(!app.sample_tooltip(199, 100));
    assert!(app.sample_tooltip(200, 100));
    assert_eq!(app.visible_tooltip(), Some("help"));
    app.popup_command(P::MenuBar).unwrap();
    assert!(app.visible_tooltip().is_none());
}
struct Reference(State);
fn color(key: String) -> Message {
    Message::Action(if key == "red" { 30 } else { 31 })
}
impl Component for Reference {
    type Message = Message;
    fn update(&mut self, m: Message) {
        match m {
            Message::Action(id) => {
                self.0.action(id);
            }
            _ => panic!("unexpected edit"),
        }
    }
    fn view(&self) -> View<Message> {
        let s = &self.0;
        View::column(
            "root",
            vec![
                View::text("title", "C SDK menus and popups"),
                View::menu_bar(
                    "bar",
                    vec![
                        View::menu(
                            "file",
                            "File",
                            vec![
                                MenuItem::action("new", "New", Message::Action(10)),
                                MenuItem::action(
                                    "unavailable",
                                    "Unavailable",
                                    Message::Action(999),
                                )
                                .disabled(true),
                                MenuItem::separator("separator"),
                                MenuItem::submenu(
                                    "recent",
                                    "Recent",
                                    vec![MenuItem::action(
                                        "project",
                                        "Project",
                                        Message::Action(u64::MAX),
                                    )],
                                )
                                .mnemonic('r'),
                            ],
                        )
                        .menu_mnemonic('f'),
                        View::menu(
                            "tools",
                            "Tools",
                            vec![MenuItem::action(
                                "tools-reset",
                                "Reset",
                                Message::Action(20),
                            )],
                        )
                        .menu_mnemonic('t'),
                    ],
                ),
                View::select(
                    "color",
                    "Choose color",
                    Some(if s.selected { "blue" } else { "red" }),
                    vec![
                        SelectOption::new("red", "Red"),
                        SelectOption::new("green", "Green unavailable").disabled(true),
                        SelectOption::new("blue", "Blue"),
                    ],
                    color,
                ),
                View::button(
                    "surface",
                    "Context actions (Shift+F10)",
                    Message::Action(51),
                )
                .context_menu(vec![
                    MenuItem::action("inspect", "Inspect", Message::Action(40)),
                    MenuItem::action("context-disabled", "Unavailable", Message::Action(999))
                        .disabled(true),
                ]),
                View::button("help", "Hover for SDK tooltip", Message::Action(50))
                    .tooltip("Tooltips use the shared SDK timer"),
                View::row(
                    "actions",
                    vec![
                        View::button("reset", "Reset", Message::Action(20)),
                        View::button("quit", "Quit", Message::Action(8)),
                    ],
                ),
                View::text(
                    "status",
                    alloc::format!(
                        "Actions {} / color {}",
                        s.actions,
                        if s.selected { "blue" } else { "red" }
                    ),
                ),
            ],
        )
    }
}
fn renderer() -> Renderer {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    super::checkable_tests::renderer_with_style(
        &std::fs::read_to_string(root.join("assets/tcc/popups.dss")).unwrap(),
    )
}
fn sequence<C: Component>(a: &mut Application<C>, phase: usize)
where
    C::Message: Clone,
{
    match phase {
        1 => {
            a.popup_command(P::MenuBar).unwrap();
            a.key(Key::plain('\r')).unwrap();
            a.popup_command(P::Activate).unwrap();
            a.key(alt('f')).unwrap();
            a.popup_command(P::Down).unwrap();
            a.popup_command(P::Right).unwrap();
        }
        2 => {
            a.popup_command(P::Activate).unwrap();
            a.key(Key::plain('\t')).unwrap();
            a.key(Key::plain(' ')).unwrap();
            a.popup_command(P::Down).unwrap();
        }
        3 => {
            a.popup_command(P::Activate).unwrap();
            a.key(Key::plain(' ')).unwrap();
            a.popup_command(P::Home).unwrap();
            a.popup_command(P::Activate).unwrap();
            a.key(Key::plain('\t')).unwrap();
            let mut r = renderer();
            r.render_application(a, 560, 420).unwrap();
            let rect = r.widget_rect("surface").unwrap();
            assert!(a.open_context_menu("surface", Some((rect.x, rect.y + rect.h))));
        }
        4 => {
            a.popup_command(P::Activate).unwrap();
            a.key(Key::plain('\t')).unwrap();
            a.pointer(PointerEvent::Move, Some("help")).unwrap();
        }
        _ => unreachable!(),
    }
}
#[test]
fn menus_choices_context_and_delayed_tooltip_match_direct_rust_pixels() {
    let mut state = State::default();
    let mut adapted = Application::new(model(&mut state)).unwrap();
    let mut direct = Application::new(Reference(State::default())).unwrap();
    let mut help = None;
    let mut roi = None;
    for (phase, name) in ["initial", "menu", "select", "context", "tooltip"]
        .iter()
        .enumerate()
    {
        if phase > 0 {
            sequence(&mut adapted, phase);
            sequence(&mut direct, phase);
        }
        let mut expected = renderer();
        let mut actual = renderer();
        let mut before = None;
        if phase == 4 {
            expected.render_application(&direct, 560, 420).unwrap();
            before = Some(expected.pixels().to_vec());
            adapted.tooltip_pointer(Some("help"), 0);
            assert!(adapted.sample_tooltip(100, 100));
            direct.tooltip_pointer(Some("help"), 0);
            assert!(direct.sample_tooltip(100, 100));
        }
        expected.render_application(&direct, 560, 420).unwrap();
        actual.render_application(&adapted, 560, 420).unwrap();
        assert_eq!(expected.pixels(), actual.pixels());
        if phase == 0 {
            let rect = expected.widget_rect("help").unwrap();
            help = Some((rect.x + 8., rect.y + 8.));
        }
        if let Some(before) = before {
            let points: Vec<_> = before
                .iter()
                .zip(expected.pixels())
                .enumerate()
                .filter_map(|(i, (a, b))| (a != b).then_some((i % 560, i / 560)))
                .collect();
            assert!(!points.is_empty());
            let x = points.iter().map(|p| p.0).min().unwrap();
            let y = points.iter().map(|p| p.1).min().unwrap();
            let w = points.iter().map(|p| p.0).max().unwrap() + 1 - x;
            let h = points.iter().map(|p| p.1).max().unwrap() + 1 - y;
            let help = expected.widget_rect("help").unwrap();
            assert!(y as f32 >= help.y + help.h || (y + h) as f32 <= help.y);
            roi = Some((x, y, w, h));
        }
        if let Some(output) = std::env::var_os("DUNIT_C_GOLDEN_DIR") {
            let pixels: Vec<u8> = expected
                .pixels()
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect();
            std::fs::write(
                std::path::Path::new(&output).join(alloc::format!("c-ui-popup-{name}.rgb")),
                pixels,
            )
            .unwrap();
        }
    }
    assert_eq!(state.actions, 3);
    assert_eq!(state.choices, 1);
    assert_eq!(state.declined, 1);
    if let Some(output) = std::env::var_os("DUNIT_C_GOLDEN_DIR") {
        let (x, y) = help.unwrap();
        let (rx, ry, w, h) = roi.unwrap();
        std::fs::write(
            std::path::Path::new(&output).join("c-ui-popup-geometry.json"),
            alloc::format!("{{\"help\":[{x},{y}],\"tooltip\":[{rx},{ry},{w},{h}]}}"),
        )
        .unwrap();
    }
}
