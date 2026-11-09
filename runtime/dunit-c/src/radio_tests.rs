use super::*;
use core::ptr;

const MESSAGES: [u64; 4] = [17, u64::MAX, 29, 31];
fn node(kind: u32, parent: u32, key: &str, label: &str, message: u64) -> DunitUiNodeV1 {
    DunitUiNodeV1 {
        kind,
        parent,
        key: key.as_ptr(),
        key_len: key.len(),
        text: label.as_ptr(),
        text_len: label.len(),
        message,
    }
}
fn nodes(selected: [usize; 2], invalid: bool) -> Vec<DunitUiNodeV1> {
    let radio = |parent, key, label, i: usize| {
        node(
            if selected[i / 2] == i % 2 || (invalid && i < 2) {
                DUNIT_UI_RADIO_CHECKED
            } else {
                DUNIT_UI_RADIO
            },
            parent,
            key,
            label,
            MESSAGES[i],
        )
    };
    vec![
        node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0),
        node(DUNIT_UI_TEXT, 0, "title", "C radio groups", 0),
        node(DUNIT_UI_RADIO_GROUP_COLUMN, 0, "main", "", 0),
        radio(2, "alpha", "Alpha", 0),
        radio(2, "beta", "Beta", 1),
        node(DUNIT_UI_RADIO_GROUP_ROW, 0, "mode", "", 0),
        radio(5, "light", "Light", 2),
        radio(5, "dark", "Dark", 3),
        node(DUNIT_UI_BUTTON, 0, "quit", "Quit", 8),
    ]
}
struct State {
    selected: [usize; 2],
    changes: usize,
    accept: bool,
    invalid: bool,
}
unsafe extern "C" fn build(user: *mut c_void, out: *mut DunitUiNodeV1, capacity: u32) -> i32 {
    let state = &*(user as *const State);
    let nodes = nodes(state.selected, state.invalid);
    assert!(capacity >= nodes.len() as u32);
    ptr::copy_nonoverlapping(nodes.as_ptr(), out, nodes.len());
    nodes.len() as i32
}
unsafe extern "C" fn update(user: *mut c_void, message: u64) -> i32 {
    let state = &mut *(user as *mut State);
    if message == 0 {
        state.invalid = true;
        return 0;
    }
    let i = MESSAGES.iter().position(|m| *m == message).unwrap();
    state.changes += 1;
    if state.accept {
        state.selected[i / 2] = i % 2;
    }
    0
}
fn model(state: &mut State) -> Model {
    unsafe {
        Model::new(DunitUiCallbacksV1 {
            abi_version: DUNIT_UI_ABI_VERSION,
            struct_size: core::mem::size_of::<DunitUiCallbacksV1>() as u32,
            user: state as *mut _ as *mut c_void,
            build: Some(build),
            update: Some(update),
        })
    }
    .unwrap()
}
fn state() -> State {
    State {
        selected: [0, 0],
        changes: 0,
        accept: true,
        invalid: false,
    }
}
#[test]
fn malformed_groups_and_foreign_group_names_are_rejected() {
    assert!(unsafe { tree::convert(&nodes([0, 0], true)) }.is_err());
    for parent in [0, DUNIT_UI_ROOT, 3] {
        let mut bad = nodes([0, 0], false);
        bad[3].parent = parent;
        assert!(unsafe { tree::convert(&bad) }.is_err());
    }
    for (key, len) in [
        (ptr::null(), 1),
        (b"\xff".as_ptr(), 1),
        (b" ".as_ptr(), 1),
        (b"x".as_ptr(), DUNIT_UI_MAX_TEXT_BYTES as usize + 1),
    ] {
        let mut bad = nodes([0, 0], false);
        bad[2].key = key;
        bad[2].key_len = len;
        assert!(unsafe { tree::convert(&bad) }.is_err());
    }
}
#[test]
fn radio_labels_and_group_identity_are_owned_after_conversion() {
    let mut group = b"main".to_vec();
    let mut label = b"Alpha".to_vec();
    let mut input = nodes([0, 0], false);
    input[2].key = group.as_ptr();
    input[3].text = label.as_ptr();
    let view = unsafe { tree::convert(&input) }.unwrap();
    group.fill(b'x');
    label.fill(b'y');
    struct Fixed(View<u64>);
    impl Component for Fixed {
        type Message = u64;
        fn update(&mut self, _: u64) {}
        fn view(&self) -> View<u64> {
            self.0.clone()
        }
    }
    let mut app = Application::new(Fixed(view)).unwrap();
    assert_eq!(app.view().children()[1].key(), "main");
    let radio = &app.view().children()[1].children()[0];
    assert_eq!(radio.accessibility_label(), Some("Alpha"));
    assert_eq!(radio.accessibility_role(), Some("radio"));
    assert_eq!(radio.accessibility_checked(), Some(true));
    app.key(Key::plain('\t')).unwrap();
    app.navigate(Navigation::Right).unwrap();
    assert_eq!(app.focused_key(), Some("beta"));
}
#[test]
fn pointer_release_is_controlled_and_selected_radio_does_not_dispatch_twice() {
    let mut state = state();
    let mut app = Application::new(model(&mut state)).unwrap();
    for (key, expected) in [("beta", [1, 0]), ("dark", [1, 1])] {
        app.pointer(PointerEvent::Down, Some(key)).unwrap();
        assert_ne!(state.selected, expected);
        app.pointer(PointerEvent::Up, Some(key)).unwrap();
        assert_eq!(state.selected, expected);
        let before = state.changes;
        app.pointer(PointerEvent::Down, Some(key)).unwrap();
        app.pointer(PointerEvent::Up, Some(key)).unwrap();
        assert_eq!(state.changes, before);
    }
    state.accept = false;
    app.pointer(PointerEvent::Down, Some("alpha")).unwrap();
    app.pointer(PointerEvent::Up, Some("alpha")).unwrap();
    assert_eq!(state.selected, [1, 1]);
    assert!(!app.view().children()[1].children()[0].interaction().checked);
    assert_eq!(state.changes, 3);
}
#[test]
fn invalid_rebuilt_selection_keeps_last_valid_tree() {
    let mut state = state();
    let mut model = model(&mut state);
    model.update(0.into());
    assert_eq!(model.error.unwrap().0, dunit::errno::EINVAL);
    assert!(model.tree.children()[1].children()[0].interaction().checked);
    assert!(!model.tree.children()[1].children()[1].interaction().checked);
}
struct Reference([usize; 2]);
impl Component for Reference {
    type Message = u64;
    fn update(&mut self, message: u64) {
        let i = MESSAGES.iter().position(|m| *m == message).unwrap();
        self.0[i / 2] = i % 2;
    }
    fn view(&self) -> View<u64> {
        View::column(
            "root",
            vec![
                View::text("title", "C radio groups"),
                View::column(
                    "main",
                    vec![
                        View::radio("alpha", "Alpha", "main", self.0[0] == 0, MESSAGES[0]),
                        View::radio("beta", "Beta", "main", self.0[0] == 1, MESSAGES[1]),
                    ],
                ),
                View::row(
                    "mode",
                    vec![
                        View::radio("light", "Light", "mode", self.0[1] == 0, MESSAGES[2]),
                        View::radio("dark", "Dark", "mode", self.0[1] == 1, MESSAGES[3]),
                    ],
                ),
                View::button("quit", "Quit", 8),
            ],
        )
    }
}
fn sequence<C: Component>(app: &mut Application<C>, phase: usize)
where
    C::Message: Clone,
{
    if phase == 1 {
        app.key(Key::plain('\t')).unwrap();
        assert_eq!(app.focused_key(), Some("alpha"));
        app.key(Key::plain(' ')).unwrap();
        app.navigate(Navigation::Right).unwrap();
        app.key(Key::plain('\r')).unwrap();
        app.key(Key::plain('\t')).unwrap();
        assert_eq!(app.focused_key(), Some("light"));
        app.navigate(Navigation::Right).unwrap();
    } else {
        app.navigate(Navigation::Right).unwrap();
        app.navigate(Navigation::Left).unwrap();
        app.key(Key::plain('\t')).unwrap();
        assert_eq!(app.focused_key(), Some("quit"));
        app.key(Key::plain('\t')).unwrap();
        assert_eq!(app.focused_key(), Some("beta"));
        app.navigate(Navigation::Left).unwrap();
        app.navigate(Navigation::Left).unwrap();
    }
}
#[test]
fn radio_tab_stops_arrows_and_wrap_match_direct_rust_pixels() {
    let mut state = state();
    let mut adapted = Application::new(model(&mut state)).unwrap();
    let mut direct = Application::new(Reference([0, 0])).unwrap();
    for (phase, name) in ["initial", "selected", "updated"].iter().enumerate() {
        if phase > 0 {
            sequence(&mut adapted, phase);
            sequence(&mut direct, phase);
            assert_eq!(state.selected, [1, 1]);
            assert_eq!(state.changes, if phase == 1 { 2 } else { 6 });
        }
        let mut expected = super::checkable_tests::renderer();
        let mut actual = super::checkable_tests::renderer();
        expected.render_application(&direct, 400, 240).unwrap();
        actual.render_application(&adapted, 400, 240).unwrap();
        assert_eq!(expected.pixels(), actual.pixels());
        if let Some(output) = std::env::var_os("DUNIT_C_GOLDEN_DIR") {
            let pixels: Vec<u8> = expected
                .pixels()
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect();
            std::fs::write(
                std::path::Path::new(&output).join(alloc::format!("c-ui-radio-{name}.rgb")),
                pixels,
            )
            .unwrap();
        }
    }
}
