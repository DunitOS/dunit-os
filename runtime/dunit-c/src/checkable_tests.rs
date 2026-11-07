use super::*;
use core::ptr;

const KEYS: [&str; 3] = ["checkbox", "switch", "toggle"];
const LABELS: [&str; 3] = ["Checkbox", "Switch", "Toggle button"];
const KINDS: [(u32, u32); 3] = [
    (DUNIT_UI_CHECKBOX, DUNIT_UI_CHECKBOX_CHECKED),
    (DUNIT_UI_SWITCH, DUNIT_UI_SWITCH_CHECKED),
    (DUNIT_UI_TOGGLE_BUTTON, DUNIT_UI_TOGGLE_BUTTON_CHECKED),
];
const MESSAGES: [u64; 3] = [1, 2, u64::MAX];
fn node(kind: u32, key: &str, label: &str, message: u64) -> DunitUiNodeV1 {
    DunitUiNodeV1 {
        kind,
        parent: 0,
        key: key.as_ptr(),
        key_len: key.len(),
        text: label.as_ptr(),
        text_len: label.len(),
        message,
    }
}
fn nodes(bits: u32) -> Vec<DunitUiNodeV1> {
    let mut nodes = vec![
        node(DUNIT_UI_COLUMN, "root", "", 0),
        node(DUNIT_UI_TEXT, "title", "C controlled widgets", 0),
    ];
    nodes[0].parent = DUNIT_UI_ROOT;
    for i in 0..3 {
        nodes.push(node(
            if bits & (1 << i) == 0 {
                KINDS[i].0
            } else {
                KINDS[i].1
            },
            KEYS[i],
            LABELS[i],
            MESSAGES[i],
        ));
    }
    nodes.push(node(DUNIT_UI_BUTTON, "quit", "Quit", 8));
    nodes
}
struct State {
    bits: u32,
    changes: usize,
    accept: bool,
}
unsafe extern "C" fn build(user: *mut c_void, out: *mut DunitUiNodeV1, capacity: u32) -> i32 {
    let state = &*(user as *const State);
    let nodes = nodes(state.bits);
    assert!(capacity >= nodes.len() as u32);
    ptr::copy_nonoverlapping(nodes.as_ptr(), out, nodes.len());
    nodes.len() as i32
}
unsafe extern "C" fn update(user: *mut c_void, message: u64) -> i32 {
    let state = &mut *(user as *mut State);
    let i = MESSAGES.iter().position(|m| *m == message).unwrap();
    state.changes += 1;
    if state.accept {
        state.bits ^= 1 << i;
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
#[test]
fn pointer_activation_preserves_full_messages_and_c_controls_state() {
    let mut state = State {
        bits: 0,
        changes: 0,
        accept: false,
    };
    let mut app = Application::new(model(&mut state)).unwrap();
    for accept in [false, true] {
        state.accept = accept;
        for (i, key) in KEYS.iter().enumerate() {
            app.pointer(PointerEvent::Down, Some(key)).unwrap();
            assert_eq!(state.bits & (1 << i), 0);
            app.pointer(PointerEvent::Up, Some(key)).unwrap();
            assert_eq!(state.bits & (1 << i) != 0, accept);
            let child = &app.view().children()[i + 1];
            assert_eq!(child.interaction().checked, accept);
            assert_eq!(child.accessibility_label(), Some(LABELS[i]));
            if i == 2 {
                assert_eq!(child.accessibility_role(), Some("button"));
                assert_eq!(child.accessibility_pressed(), Some(accept));
            } else {
                assert_eq!(child.accessibility_role(), Some(KEYS[i]));
                assert_eq!(child.accessibility_checked(), Some(accept));
            }
            app.pointer(PointerEvent::Up, Some(key)).unwrap();
        }
    }
    assert_eq!(state.changes, 6);
}
#[test]
fn checkable_nodes_remain_utf8_checked_leaves() {
    for (off, on) in KINDS {
        for kind in [off, on] {
            let mut nodes = nodes(0);
            nodes[2].kind = kind;
            nodes[3].parent = 2;
            assert!(unsafe { tree::convert(&nodes) }.is_err());
            nodes[3].parent = 0;
            nodes[2].text = b"\xff".as_ptr();
            nodes[2].text_len = 1;
            assert!(unsafe { tree::convert(&nodes) }.is_err());
        }
    }
}
struct Reference(u32);
impl Component for Reference {
    type Message = u64;
    fn update(&mut self, message: u64) {
        let i = MESSAGES.iter().position(|m| *m == message).unwrap();
        self.0 ^= 1 << i;
    }
    fn view(&self) -> View<u64> {
        View::column(
            "root",
            vec![
                View::text("title", "C controlled widgets"),
                View::checkbox("checkbox", "Checkbox", self.0 & 1 != 0, MESSAGES[0]),
                View::switch("switch", "Switch", self.0 & 2 != 0, MESSAGES[1]),
                View::toggle_button("toggle", "Toggle button", self.0 & 4 != 0, MESSAGES[2]),
                View::button("quit", "Quit", 8),
            ],
        )
    }
}
fn renderer() -> Renderer {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let theme = Theme::parse_shell(
        &std::fs::read_to_string(root.join("assets/dwm/sdk/dark.dss")).unwrap(),
        &std::fs::read_to_string(root.join("assets/dwm/sdk/shell.dss")).unwrap(),
        &std::fs::read_to_string(root.join("assets/dwm/sdk/base.dss")).unwrap(),
        "",
    )
    .unwrap();
    let font =
        Font::parse(std::fs::read(root.join("assets/fonts/DejaVuSans.ttf")).unwrap()).unwrap();
    let mut renderer = Renderer::new(font, theme.cascade, theme.background);
    renderer.set_motion_enabled(false);
    renderer
}
fn activate_all<C: Component<Message = u64>>(app: &mut Application<C>, step: usize) {
    if step == 2 {
        app.key(Key::plain('\t')).unwrap();
    }
    for i in 0..3 {
        app.key(Key::plain('\t')).unwrap();
        app.key(Key::plain(if (i + step) % 2 == 1 { ' ' } else { '\r' }))
            .unwrap();
    }
}
#[test]
fn keyboard_transitions_match_direct_rust_pixels_in_both_directions() {
    let mut state = State {
        bits: 0,
        changes: 0,
        accept: true,
    };
    let mut adapted = Application::new(model(&mut state)).unwrap();
    let mut direct = Application::new(Reference(0)).unwrap();
    for (step, name) in ["initial", "checked", "updated"].iter().enumerate() {
        if step > 0 {
            activate_all(&mut adapted, step);
            activate_all(&mut direct, step);
            assert_eq!(state.bits, if step == 1 { 7 } else { 0 });
        }
        let mut expected = renderer();
        let mut actual = renderer();
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
                std::path::Path::new(&output).join(alloc::format!("c-ui-checkables-{name}.rgb")),
                pixels,
            )
            .unwrap();
        }
    }
    assert_eq!(state.changes, 6);
}
