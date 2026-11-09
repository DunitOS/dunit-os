use super::*;
use core::ptr;
const LABELS: [&str; 4] = ["Storage", "Display", "Network", "Theme"];
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
#[derive(Default)]
struct State {
    wide: bool,
    warm: bool,
    selected: usize,
    summary: String,
    classes: String,
    failure: i32,
    calls: usize,
}
fn nodes(s: &mut State) -> Vec<DunitUiNodeV1> {
    s.summary = alloc::format!("Selected: {}", LABELS[s.selected]);
    let mut nodes = vec![
        node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0),
        node(DUNIT_UI_TEXT, 0, "title", "C SDK layout lab", 0),
        node(DUNIT_UI_STACK, 0, "hero", "", 0),
        node(DUNIT_UI_COLUMN, 2, "backdrop", "", 0),
        node(DUNIT_UI_TEXT, 2, "summary", &s.summary, 0),
        node(DUNIT_UI_ROW, 0, "controls", "", 0),
        node(DUNIT_UI_BUTTON, 5, "layout", "Layout", 1),
        node(DUNIT_UI_BUTTON, 5, "palette", "Palette", 2),
        node(DUNIT_UI_BUTTON, 5, "quit", "Quit", 8),
        node(DUNIT_UI_GRID, 0, "cards", "", 0),
    ];
    for (i, label) in LABELS.iter().enumerate() {
        nodes.push(node(DUNIT_UI_BUTTON, 9, label, label, 10 + i as u64));
    }
    nodes.push(node(
        DUNIT_UI_TEXT,
        0,
        "hint",
        "Tab / Space: layout, palette, cards",
        0,
    ));
    nodes
}
unsafe extern "C" fn build(user: *mut c_void, out: *mut DunitUiNodeV1, capacity: u32) -> i32 {
    let nodes = nodes(&mut *(user as *mut State));
    assert!(capacity >= nodes.len() as u32);
    ptr::copy_nonoverlapping(nodes.as_ptr(), out, nodes.len());
    nodes.len() as i32
}
unsafe extern "C" fn decorate(user: *mut c_void, index: u32, out: *mut DunitUiAppearanceV1) -> i32 {
    let s = &mut *(user as *mut State);
    s.calls += 1;
    if s.failure != 0 {
        return s.failure;
    }
    s.classes = String::from(match index {
        0 => {
            if s.warm {
                "page warm"
            } else {
                "page"
            }
        }
        1 => "heading",
        2 => "hero",
        3 => "backdrop",
        4 => "summary",
        5 => "controls",
        6..=8 => "control",
        9 => {
            if s.wide {
                "cards wide"
            } else {
                "cards"
            }
        }
        10..=13 => {
            if index as usize - 10 == s.selected {
                if s.warm {
                    "card selected warm"
                } else {
                    "card selected"
                }
            } else {
                "card"
            }
        }
        _ => "hint",
    });
    (*out).classes = s.classes.as_ptr();
    (*out).classes_len = s.classes.len();
    0
}
fn apply(s: &mut State, message: u64) {
    match message {
        1 => s.wide = !s.wide,
        2 => s.warm = !s.warm,
        10..=13 => s.selected = message as usize - 10,
        _ => panic!("unexpected activation"),
    }
}
unsafe extern "C" fn update(user: *mut c_void, message: u64) -> i32 {
    apply(&mut *(user as *mut State), message);
    0
}
fn callbacks(s: &mut State) -> DunitUiCallbacksV1 {
    DunitUiCallbacksV1 {
        abi_version: DUNIT_UI_ABI_VERSION,
        struct_size: core::mem::size_of::<DunitUiCallbacksV1>() as u32,
        user: s as *mut _ as *mut c_void,
        build: Some(build),
        update: Some(update),
    }
}
#[test]
fn classes_are_copied_per_node_before_the_next_foreign_callback() {
    let mut s = State::default();
    let cb = callbacks(&mut s);
    let model = unsafe { Model::new_decorated(cb, Some(decorate)) }.unwrap();
    s.classes.clear();
    s.summary.clear();
    assert_eq!(s.calls, 15);
    assert_eq!(model.tree.classes(), &[String::from("page")]);
    assert_eq!(
        model.tree.children()[1].children()[1].content(),
        Some("Selected: Storage")
    );
    assert_eq!(
        model.tree.children()[3].children()[0].classes(),
        &[String::from("card"), String::from("selected")]
    );
}
#[test]
fn decoration_errors_keep_the_previous_valid_tree() {
    let mut s = State::default();
    let cb = callbacks(&mut s);
    let mut m = unsafe { Model::new_decorated(cb, Some(decorate)) }.unwrap();
    for error in [-5, 1] {
        s.failure = error;
        m.update(1);
        assert_eq!(
            m.error.unwrap().0,
            if error < 0 { -5 } else { dunit::errno::EINVAL }
        );
        assert_eq!(m.tree.children()[3].classes(), &[String::from("cards")]);
        m.error = None;
    }
}
struct Bad {
    pointer: *const u8,
    length: usize,
}
unsafe extern "C" fn bad(user: *mut c_void, _: u32, out: *mut DunitUiAppearanceV1) -> i32 {
    let b = &*(user as *const Bad);
    (*out).classes = b.pointer;
    (*out).classes_len = b.length;
    0
}
#[test]
fn class_spans_and_identifiers_use_the_existing_bounds_and_sdk_grammar() {
    for (pointer, length) in [
        (ptr::null(), 1),
        (b"\xff".as_ptr(), 1),
        (b"bad!".as_ptr(), 4),
        (b"x".as_ptr(), DUNIT_UI_MAX_TEXT_BYTES as usize + 1),
    ] {
        let mut data = Bad { pointer, length };
        let nodes = [node(DUNIT_UI_BUTTON, DUNIT_UI_ROOT, "root", "Test", 1)];
        assert!(unsafe {
            tree::convert_decorated(&nodes, &mut data as *mut _ as *mut c_void, Some(bad))
        }
        .is_err());
    }
}
#[test]
fn null_empty_class_spans_are_allowed_and_appearance_output_starts_empty() {
    unsafe extern "C" fn empty(_: *mut c_void, _: u32, out: *mut DunitUiAppearanceV1) -> i32 {
        assert!((*out).classes.is_null());
        assert_eq!((*out).classes_len, 0);
        0
    }
    let nodes = [node(DUNIT_UI_BUTTON, DUNIT_UI_ROOT, "root", "Test", 1)];
    assert!(
        unsafe { tree::convert_decorated(&nodes, ptr::null_mut(), Some(empty)) }
            .unwrap()
            .classes()
            .is_empty()
    );
}
#[test]
fn style_configuration_and_spans_fail_before_native_transport() {
    let mut s = State::default();
    let cb = callbacks(&mut s);
    let env = [ptr::null()];
    let mut style = DunitUiStyleV1 {
        abi_version: DUNIT_UI_ABI_VERSION,
        struct_size: core::mem::size_of::<DunitUiStyleV1>() as u32,
        stylesheet: ptr::null(),
        stylesheet_len: 0,
        decorate: None,
    };
    for error in 0..5 {
        let candidate = match error {
            0 => ptr::null(),
            _ => {
                style.abi_version = if error == 1 { 99 } else { DUNIT_UI_ABI_VERSION };
                style.struct_size = if error == 2 { 0 } else { 32 };
                style.stylesheet_len = if error >= 3 { 1 } else { 0 };
                style.stylesheet = if error == 4 {
                    b"\xff".as_ptr()
                } else {
                    ptr::null()
                };
                &style
            }
        };
        assert_eq!(
            unsafe {
                dunit_ui_run_styled_v1(0, ptr::null(), env.as_ptr(), 560, 360, &cb, candidate)
            },
            dunit::errno::EINVAL as i32
        );
    }
    assert_eq!(s.calls, 0);
}
#[test]
fn stack_and_grid_keep_shared_depth_parent_and_disabled_rules() {
    for kind in [DUNIT_UI_STACK, DUNIT_UI_GRID] {
        let nodes = [
            node(kind, DUNIT_UI_ROOT, "root", "", 0),
            node(DUNIT_UI_TEXT, 0, "child", "Text", 0),
        ];
        assert!(unsafe { tree::convert(&nodes) }.is_ok());
        let mut invalid = nodes;
        invalid[0].kind |= DUNIT_UI_DISABLED;
        assert!(unsafe { tree::convert(&invalid) }.is_err());
        let mut invalid = nodes;
        invalid[1].parent = 1;
        assert!(unsafe { tree::convert(&invalid) }.is_err());
    }
}
#[test]
fn application_dss_validation_remains_the_sdk_theme_contract() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let profile = std::fs::read_to_string(root.join("assets/dwm/sdk/dark.dss")).unwrap();
    let defaults = std::fs::read_to_string(root.join("assets/dwm/sdk/shell.dss")).unwrap();
    let base = std::fs::read_to_string(root.join("assets/dwm/sdk/base.dss")).unwrap();
    for invalid in [
        "Grid { grid-columns: 0; }",
        "Grid { width: -2; }",
        "Button { color: $undefined; }",
        "Button { color: #xyz; }",
    ] {
        assert!(Theme::parse_shell(&profile, &defaults, &base, invalid).is_err());
    }
}
struct Reference(State);
impl Component for Reference {
    type Message = u64;
    fn update(&mut self, message: u64) {
        apply(&mut self.0, message);
    }
    fn view(&self) -> View<u64> {
        let s = &self.0;
        let mut cards = Vec::new();
        for (i, label) in LABELS.iter().enumerate() {
            let mut card = View::button(*label, *label, 10 + i as u64).class("card");
            if i == s.selected {
                card = card.class("selected");
                if s.warm {
                    card = card.class("warm");
                }
            }
            cards.push(card);
        }
        let mut grid = View::grid("cards", cards).class("cards");
        if s.wide {
            grid = grid.class("wide");
        }
        let mut root = View::column(
            "root",
            vec![
                View::text("title", "C SDK layout lab").class("heading"),
                View::stack(
                    "hero",
                    vec![
                        View::column("backdrop", vec![]).class("backdrop"),
                        View::text(
                            "summary",
                            alloc::format!("Selected: {}", LABELS[s.selected]),
                        )
                        .class("summary"),
                    ],
                )
                .class("hero"),
                View::row(
                    "controls",
                    vec![
                        View::button("layout", "Layout", 1).class("control"),
                        View::button("palette", "Palette", 2).class("control"),
                        View::button("quit", "Quit", 8).class("control"),
                    ],
                )
                .class("controls"),
                grid,
                View::text("hint", "Tab / Space: layout, palette, cards").class("hint"),
            ],
        )
        .class("page");
        if s.warm {
            root = root.class("warm");
        }
        root
    }
}
fn sequence<C: Component<Message = u64>>(app: &mut Application<C>, phase: usize) {
    app.key(Key::plain('\t')).unwrap();
    app.key(Key::plain(' ')).unwrap();
    if phase == 2 {
        for _ in 0..3 {
            app.key(Key::plain('\t')).unwrap();
        }
        app.key(Key::plain(' ')).unwrap();
    }
}
#[test]
fn application_classes_grid_reflow_and_stack_layers_match_direct_rust_pixels() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sheet = std::fs::read_to_string(root.join("assets/tcc/layout.dss")).unwrap();
    let mut state = State::default();
    let cb = callbacks(&mut state);
    let mut adapted =
        Application::new(unsafe { Model::new_decorated(cb, Some(decorate)) }.unwrap()).unwrap();
    let mut direct = Application::new(Reference(State::default())).unwrap();
    let mut frames = Vec::new();
    for (phase, name) in ["initial", "wide", "styled"].iter().enumerate() {
        if phase > 0 {
            sequence(&mut adapted, phase);
            sequence(&mut direct, phase);
        }
        let mut expected = super::checkable_tests::renderer_with_style(&sheet);
        let mut actual = super::checkable_tests::renderer_with_style(&sheet);
        expected.render_application(&direct, 560, 360).unwrap();
        actual.render_application(&adapted, 560, 360).unwrap();
        assert_eq!(expected.pixels(), actual.pixels());
        let position = |key: &str| {
            (0..360)
                .step_by(4)
                .find_map(|y| {
                    (0..560)
                        .step_by(4)
                        .find(|x| actual.hit_key(*x as f32, y as f32) == Some(key))
                        .map(|x| (x, y))
                })
                .expect("card must have a real input rectangle")
        };
        let storage = position("Storage");
        let network = position("Network");
        if phase == 0 {
            assert!(network.1 > storage.1);
        } else {
            assert_eq!(network.1, storage.1);
            assert!(network.0 > storage.0);
        }
        frames.push(expected.pixels().to_vec());
        if let Some(output) = std::env::var_os("DUNIT_C_GOLDEN_DIR") {
            let pixels: Vec<u8> = expected
                .pixels()
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect();
            std::fs::write(
                std::path::Path::new(&output).join(alloc::format!("c-ui-style-{name}.rgb")),
                pixels,
            )
            .unwrap();
        }
    }
    assert_ne!(frames[0], frames[1]);
    assert_ne!(frames[1], frames[2]);
    assert!(state.wide && state.warm);
    assert_eq!(state.selected, 1);
}
