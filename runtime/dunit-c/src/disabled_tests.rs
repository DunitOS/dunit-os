use super::*;
use core::ptr;
fn node(kind: u32, parent: u32, key: &str, text: &str, message: u64) -> DunitUiNodeV1 {
    DunitUiNodeV1 {
        kind,
        parent,
        key: key.as_ptr(),
        key_len: key.len(),
        text: text.as_ptr(),
        text_len: text.len(),
        message,
    }
}
struct Fixed {
    view: View<u64>,
    calls: Vec<u64>,
}
impl Component for Fixed {
    type Message = u64;
    fn update(&mut self, m: u64) {
        self.calls.push(m);
    }
    fn view(&self) -> View<u64> {
        self.view.clone()
    }
}
#[test]
fn every_disabled_activation_kind_suppresses_pointer_and_is_skipped_by_tab() {
    for kind in [
        DUNIT_UI_BUTTON,
        DUNIT_UI_CHECKBOX,
        DUNIT_UI_CHECKBOX_CHECKED,
        DUNIT_UI_SWITCH,
        DUNIT_UI_SWITCH_CHECKED,
        DUNIT_UI_TOGGLE_BUTTON,
        DUNIT_UI_TOGGLE_BUTTON_CHECKED,
        DUNIT_UI_RADIO,
        DUNIT_UI_RADIO_CHECKED,
    ] {
        let radio = matches!(kind, DUNIT_UI_RADIO | DUNIT_UI_RADIO_CHECKED);
        let nodes = [
            node(
                if radio {
                    DUNIT_UI_RADIO_GROUP_COLUMN
                } else {
                    DUNIT_UI_COLUMN
                },
                DUNIT_UI_ROOT,
                "root",
                "",
                0,
            ),
            node(kind | DUNIT_UI_DISABLED, 0, "target", "Target", u64::MAX),
            node(
                if radio {
                    DUNIT_UI_RADIO
                } else {
                    DUNIT_UI_BUTTON
                },
                0,
                "peer",
                "Peer",
                7,
            ),
        ];
        let view = unsafe { tree::convert(&nodes) }.unwrap();
        let mut app = Application::new(Fixed {
            view,
            calls: vec![],
        })
        .unwrap();
        app.pointer(PointerEvent::Down, Some("target")).unwrap();
        app.pointer(PointerEvent::Up, Some("target")).unwrap();
        assert!(app.component().calls.is_empty());
        assert!(app.view().children()[0].interaction().disabled);
        assert_eq!(
            app.view().children()[0].interaction().checked,
            matches!(
                kind,
                DUNIT_UI_CHECKBOX_CHECKED
                    | DUNIT_UI_SWITCH_CHECKED
                    | DUNIT_UI_TOGGLE_BUTTON_CHECKED
                    | DUNIT_UI_RADIO_CHECKED
            )
        );
        app.key(Key::plain('\t')).unwrap();
        assert_eq!(app.focused_key(), Some("peer"));
    }
}
#[test]
fn unsupported_flags_do_not_disable_containers_or_bypass_kind_validation() {
    for kind in [
        DUNIT_UI_TEXT,
        DUNIT_UI_COLUMN,
        DUNIT_UI_ROW,
        DUNIT_UI_RADIO_GROUP_COLUMN,
        DUNIT_UI_RADIO_GROUP_ROW,
        99,
        0x40000002,
    ] {
        assert!(unsafe {
            tree::convert(&[node(kind | DUNIT_UI_DISABLED, DUNIT_UI_ROOT, "root", "", 0)])
        }
        .is_err());
    }
}
#[derive(Default)]
struct State {
    enabled: bool,
    count: usize,
    checked: bool,
}
impl State {
    fn apply(&mut self, message: u64) {
        match message {
            1 => {
                assert!(self.enabled);
                self.count += 1;
            }
            2 => {
                assert!(self.enabled);
                self.checked = !self.checked;
            }
            3 => self.enabled = !self.enabled,
            _ => panic!("unexpected activation"),
        }
    }
}
unsafe extern "C" fn build(user: *mut c_void, out: *mut DunitUiNodeV1, capacity: u32) -> i32 {
    let s = &*(user as *const State);
    let flag = if s.enabled { 0 } else { DUNIT_UI_DISABLED };
    let nodes = [
        node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0),
        node(DUNIT_UI_TEXT, 0, "title", "C disabled controls", 0),
        node(DUNIT_UI_BUTTON | flag, 0, "action", "Action", 1),
        node(
            (if s.checked {
                DUNIT_UI_CHECKBOX_CHECKED
            } else {
                DUNIT_UI_CHECKBOX
            }) | flag,
            0,
            "check",
            "Checkbox",
            2,
        ),
        node(
            if s.enabled {
                DUNIT_UI_TOGGLE_BUTTON_CHECKED
            } else {
                DUNIT_UI_TOGGLE_BUTTON
            },
            0,
            "enable",
            "Enable controls",
            3,
        ),
        node(DUNIT_UI_BUTTON, 0, "quit", "Quit", 8),
    ];
    assert!(capacity >= nodes.len() as u32);
    ptr::copy_nonoverlapping(nodes.as_ptr(), out, nodes.len());
    nodes.len() as i32
}
unsafe extern "C" fn update(user: *mut c_void, message: u64) -> i32 {
    (&mut *(user as *mut State)).apply(message);
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
#[test]
fn disabling_during_a_press_clears_focus_and_cancels_release_activation() {
    let mut state = State::default();
    let mut app = Application::new(model(&mut state)).unwrap();
    app.update(3).unwrap();
    app.pointer(PointerEvent::Down, Some("action")).unwrap();
    assert_eq!(app.focused_key(), Some("action"));
    app.update(3).unwrap();
    app.pointer(PointerEvent::Up, Some("action")).unwrap();
    assert_eq!(app.focused_key(), None);
    assert_eq!(state.count, 0);
    app.update(3).unwrap();
    app.pointer(PointerEvent::Down, Some("action")).unwrap();
    app.pointer(PointerEvent::Up, Some("action")).unwrap();
    assert_eq!(state.count, 1);
}
struct Reference(State);
impl Component for Reference {
    type Message = u64;
    fn update(&mut self, message: u64) {
        self.0.apply(message);
    }
    fn view(&self) -> View<u64> {
        View::column(
            "root",
            vec![
                View::text("title", "C disabled controls"),
                View::button("action", "Action", 1).disabled(!self.0.enabled),
                View::checkbox("check", "Checkbox", self.0.checked, 2).disabled(!self.0.enabled),
                View::toggle_button("enable", "Enable controls", self.0.enabled, 3),
                View::button("quit", "Quit", 8),
            ],
        )
    }
}
fn sequence<C: Component<Message = u64>>(app: &mut Application<C>, phase: usize) {
    if phase == 1 {
        app.key(Key::plain('\t')).unwrap();
        assert_eq!(app.focused_key(), Some("enable"));
        app.key(Key::plain(' ')).unwrap();
    } else {
        app.key(Key::plain('\t')).unwrap();
        app.key(Key::plain('\t')).unwrap();
        for key in ["action", "check", "enable"] {
            assert_eq!(app.focused_key(), Some(key));
            app.key(Key::plain(' ')).unwrap();
            if key != "enable" {
                app.key(Key::plain('\t')).unwrap();
            }
        }
    }
}
#[test]
fn disabled_enabled_and_disabled_checked_states_match_rust_pixels() {
    let mut state = State::default();
    let mut adapted = Application::new(model(&mut state)).unwrap();
    let mut direct = Application::new(Reference(State::default())).unwrap();
    for (phase, name) in ["initial", "enabled", "updated"].iter().enumerate() {
        if phase > 0 {
            sequence(&mut adapted, phase);
            sequence(&mut direct, phase);
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
                std::path::Path::new(&output).join(alloc::format!("c-ui-disabled-{name}.rgb")),
                pixels,
            )
            .unwrap();
        }
    }
    assert!(!state.enabled);
    assert!(state.checked);
    assert_eq!(state.count, 1);
}
