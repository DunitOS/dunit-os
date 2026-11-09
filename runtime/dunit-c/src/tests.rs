use super::*;
use core::ptr;
fn node(kind: u32, parent: u32, key: &[u8], text: &[u8], message: u64) -> DunitUiNodeV1 {
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
fn nodes(label: &[u8]) -> Vec<DunitUiNodeV1> {
    vec![
        node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, b"root", b"", 0),
        node(DUNIT_UI_TEXT, 0, b"title", label, 0),
        node(DUNIT_UI_BUTTON, 0, b"increment", b"Increment", 10),
        node(DUNIT_UI_BUTTON, 0, b"quit", b"Quit", 20),
    ]
}
#[test]
fn descriptions_copy_text_and_keep_order() {
    let mut label = b"Count 0".to_vec();
    let view = unsafe { tree::convert(&nodes(&label)) }.unwrap();
    label.fill(b'x');
    assert_eq!(view.children()[0].content(), Some("Count 0"));
    assert_eq!(view.children()[1].key(), "increment");
    assert_eq!(view.children()[2].key(), "quit");
}
#[test]
fn invalid_shapes_and_shared_sdk_key_rules_are_rejected() {
    let original = nodes(b"Count 0");
    for change in 0..6 {
        let mut bad = original.clone();
        match change {
            0 => bad[0].parent = 0,
            1 => bad[1].parent = 2,
            2 => bad[2].parent = 1,
            3 => bad[2].kind = 99,
            4 => {
                bad[2].key = bad[1].key;
                bad[2].key_len = bad[1].key_len;
            }
            _ => bad[0].key_len = 0,
        }
        assert!(unsafe { tree::convert(&bad) }.is_err());
    }
    assert!(unsafe { tree::convert(&[]) }.is_err());
}
#[test]
fn foreign_strings_are_length_checked_and_utf8_checked() {
    for (pointer, length) in [
        (ptr::null(), 1),
        (b"x".as_ptr(), DUNIT_UI_MAX_TEXT_BYTES as usize + 1),
        (b"\xff".as_ptr(), 1),
    ] {
        let mut bad = nodes(b"Count 0");
        bad[1].text = pointer;
        bad[1].text_len = length;
        assert!(unsafe { tree::convert(&bad) }.is_err());
    }
}
#[test]
fn foreign_depth_uses_the_actual_runtime_limit() {
    let mut deep = Vec::new();
    for i in 0..=MAX_VIEW_DEPTH {
        deep.push(node(
            DUNIT_UI_COLUMN,
            if i == 0 { DUNIT_UI_ROOT } else { i as u32 - 1 },
            b"key",
            b"",
            0,
        ));
    }
    // Unique keys distinguish the depth check from duplicate-key rejection.
    let keys: Vec<_> = (0..deep.len()).map(|i| alloc::format!("key{i}")).collect();
    for (n, k) in deep.iter_mut().zip(&keys) {
        n.key = k.as_ptr();
        n.key_len = k.len();
    }
    assert!(unsafe { tree::convert(&deep[..MAX_VIEW_DEPTH]) }.is_ok());
    assert!(unsafe { tree::convert(&deep) }.is_err());
}
struct State {
    label: Vec<u8>,
    fail: bool,
    builds: usize,
}
unsafe extern "C" fn build_test(user: *mut c_void, out: *mut DunitUiNodeV1, capacity: u32) -> i32 {
    let state = &mut *(user as *mut State);
    state.builds += 1;
    if state.fail {
        return -5;
    }
    let input = nodes(&state.label);
    assert!(capacity >= input.len() as u32);
    ptr::copy_nonoverlapping(input.as_ptr(), out, input.len());
    input.len() as i32
}
unsafe extern "C" fn update_test(user: *mut c_void, message: u64) -> i32 {
    let state = &mut *(user as *mut State);
    match message {
        10 => {
            state.label = if state.label == b"Count 0" {
                b"Count 1".to_vec()
            } else {
                b"Count 2".to_vec()
            };
            0
        }
        20 => 1,
        _ => 2,
    }
}
fn callbacks(state: &mut State) -> DunitUiCallbacksV1 {
    DunitUiCallbacksV1 {
        abi_version: DUNIT_UI_ABI_VERSION,
        struct_size: core::mem::size_of::<DunitUiCallbacksV1>() as u32,
        user: state as *mut _ as *mut c_void,
        build: Some(build_test),
        update: Some(update_test),
    }
}
#[test]
fn callbacks_rebuild_the_model_and_retain_the_prior_tree_on_error() {
    let mut state = State {
        label: b"Count 0".to_vec(),
        fail: false,
        builds: 0,
    };
    let cb = callbacks(&mut state);
    let mut model = unsafe { Model::new(cb) }.unwrap();
    let buffer = model.nodes.as_ptr();
    model.update(10.into());
    assert_eq!(model.nodes.as_ptr(), buffer);
    assert_eq!(model.tree.children()[0].content(), Some("Count 1"));
    model.update(99.into());
    assert_eq!(model.error.unwrap().0, dunit::errno::EINVAL);
    model.error = None;
    state.fail = true;
    model.update(10.into());
    assert_eq!(model.error.unwrap().0, -5);
    assert_eq!(model.nodes.as_ptr(), buffer);
    assert_eq!(model.tree.children()[0].content(), Some("Count 1"));
    let before = state.builds;
    model.update(20.into());
    assert!(model.stop);
    assert_eq!(state.builds, before);
}
unsafe extern "C" fn oversized(_: *mut c_void, _: *mut DunitUiNodeV1, capacity: u32) -> i32 {
    capacity as i32 + 1
}
#[test]
fn callback_cannot_claim_more_nodes_than_its_buffer() {
    let mut state = State {
        label: vec![],
        fail: false,
        builds: 0,
    };
    let mut cb = callbacks(&mut state);
    cb.build = Some(oversized);
    assert!(unsafe { build(&cb, &mut node_buffer()) }.is_err());
}
#[test]
fn invalid_config_is_rejected_without_entering_native_transport() {
    let mut state = State {
        label: vec![],
        fail: false,
        builds: 0,
    };
    let mut cb = callbacks(&mut state);
    cb.abi_version += 1;
    let env = [ptr::null()];
    assert_eq!(
        unsafe { dunit_ui_run_v1(0, ptr::null(), env.as_ptr(), 400, 180, &cb) },
        dunit::errno::EINVAL as i32
    );
    assert_eq!(state.builds, 0);
}
struct Reference(u32);
impl Component for Reference {
    type Message = u64;
    fn update(&mut self, message: u64) {
        assert_eq!(message, 10);
        self.0 += 1;
    }
    fn view(&self) -> View<u64> {
        View::column(
            "root",
            vec![
                View::text("title", alloc::format!("Count {}", self.0)),
                View::button("increment", "Increment", 10),
                View::button("quit", "Quit", 20),
            ],
        )
    }
}
#[test]
fn adapted_widgets_match_direct_rust_pixels_before_and_after_update() {
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
    let renderer = || {
        let mut r = Renderer::new(font.clone(), theme.cascade.clone(), theme.background);
        r.set_motion_enabled(false);
        r
    };
    let mut reference = Application::new(Reference(0)).unwrap();
    let mut state = State {
        label: b"Count 0".to_vec(),
        fail: false,
        builds: 0,
    };
    let cb = callbacks(&mut state);
    let mut adapter = Application::new(unsafe { Model::new(cb) }.unwrap()).unwrap();
    for (name, count) in [("initial", 0), ("updated", 2)] {
        if count > 0 {
            reference.key(Key::plain('\t')).unwrap();
            adapter.key(Key::plain('\t')).unwrap();
            reference.update(10).unwrap();
            reference.update(10).unwrap();
            // The C callback owns the model; two activations can select any label.
            adapter.update(10.into()).unwrap();
            adapter.update(10.into()).unwrap();
        }
        let mut direct = renderer();
        let mut translated = renderer();
        direct.render_application(&reference, 400, 180).unwrap();
        translated.render_application(&adapter, 400, 180).unwrap();
        assert_eq!(direct.pixels(), translated.pixels());
        if let Some(output) = std::env::var_os("DUNIT_C_GOLDEN_DIR") {
            let pixels: Vec<u8> = direct
                .pixels()
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect();
            std::fs::write(
                std::path::Path::new(&output).join(alloc::format!("c-ui-{name}.rgb")),
                pixels,
            )
            .unwrap();
        }
    }
}
