use super::*;
use core::ptr;
const IDS: [u64; 3] = [101, 102, u64::MAX];
const LABELS: [&str; 5] = ["Name", "Password", "Notes", "Read only", "Disabled"];
struct State {
    values: [String; 3],
    status: String,
    accepted: usize,
    declined: usize,
    submits: usize,
    resets: usize,
    changed_result: i32,
}
impl Default for State {
    fn default() -> Self {
        Self {
            values: ["guest".into(), "secret".into(), "hello".into()],
            status: String::new(),
            accepted: 0,
            declined: 0,
            submits: 0,
            resets: 0,
            changed_result: 0,
        }
    }
}
impl State {
    fn edit(&mut self, id: u64, value: &str) -> i32 {
        if self.changed_result != 0 {
            return self.changed_result;
        }
        let i = IDS
            .iter()
            .position(|n| *n == id)
            .expect("read-only/disabled fields must not dispatch");
        if i == 0 && value.contains('!') {
            self.declined += 1;
            return 0;
        }
        self.values[i] = value.into();
        self.accepted += 1;
        0
    }
    fn action(&mut self, id: u64) -> i32 {
        match id {
            8 => 1,
            201 => {
                self.submits += 1;
                0
            }
            10 => {
                self.values = ["guest".into(), "secret".into(), "hello".into()];
                self.resets += 1;
                0
            }
            _ => panic!("unexpected action"),
        }
    }
}
fn node(kind: u32, parent: u32, key: &str, value: &str, id: u64) -> DunitUiNodeV1 {
    DunitUiNodeV1 {
        kind,
        parent,
        key: key.as_ptr(),
        key_len: key.len(),
        text: value.as_ptr(),
        text_len: value.len(),
        message: id,
    }
}
unsafe extern "C" fn build(user: *mut c_void, out: *mut DunitUiNodeV1, capacity: u32) -> i32 {
    let s = &mut *(user as *mut State);
    s.status = alloc::format!(
        "Accepted {} / declined {} / submits {}",
        s.accepted,
        s.declined,
        s.submits
    );
    let nodes = [
        node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0),
        node(DUNIT_UI_TEXT, 0, "title", "C SDK text editor", 0),
        node(DUNIT_UI_INPUT, 0, "name", &s.values[0], IDS[0]),
        node(DUNIT_UI_INPUT, 0, "password", &s.values[1], IDS[1]),
        node(DUNIT_UI_TEXTAREA, 0, "notes", &s.values[2], IDS[2]),
        node(DUNIT_UI_INPUT, 0, "readonly", "read only", 104),
        node(
            DUNIT_UI_INPUT | DUNIT_UI_DISABLED,
            0,
            "disabled",
            "unavailable",
            105,
        ),
        node(DUNIT_UI_ROW, 0, "actions", "", 0),
        node(DUNIT_UI_BUTTON, 7, "reset", "Reset", 10),
        node(DUNIT_UI_BUTTON, 7, "quit", "Quit", 8),
        node(DUNIT_UI_TEXT, 0, "status", &s.status, 0),
    ];
    assert!(capacity >= nodes.len() as u32);
    ptr::copy_nonoverlapping(nodes.as_ptr(), out, nodes.len());
    nodes.len() as i32
}
unsafe extern "C" fn configure(_: *mut c_void, index: u32, out: *mut DunitUiInputV1) -> i32 {
    let label = LABELS[index as usize - 2];
    (*out).label = label.as_ptr();
    (*out).label_len = label.len();
    (*out).placeholder = label.as_ptr();
    (*out).placeholder_len = label.len();
    (*out).max_chars = if index == 2 { 8 } else { 128 };
    (*out).flags = match index {
        2 => {
            (*out).submit = 201;
            DUNIT_UI_INPUT_SUBMIT
        }
        3 => DUNIT_UI_INPUT_PASSWORD,
        4 => DUNIT_UI_INPUT_SOFT_WRAP,
        5 => DUNIT_UI_INPUT_READ_ONLY,
        _ => 0,
    };
    0
}
unsafe extern "C" fn changed(user: *mut c_void, id: u64, value: *const u8, len: usize) -> i32 {
    let value = core::str::from_utf8(core::slice::from_raw_parts(value, len)).unwrap();
    (&mut *(user as *mut State)).edit(id, value)
}
unsafe extern "C" fn update(user: *mut c_void, id: u64) -> i32 {
    (&mut *(user as *mut State)).action(id)
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
fn config() -> DunitUiTextV1 {
    DunitUiTextV1 {
        abi_version: DUNIT_UI_ABI_VERSION,
        struct_size: core::mem::size_of::<DunitUiTextV1>() as u32,
        configure: Some(configure),
        changed: Some(changed),
    }
}
fn model(s: &mut State) -> Model {
    let cb = callbacks(s);
    unsafe { Model::new_configured(cb, None, Some(config())) }.unwrap()
}
#[test]
fn utf8_limits_selection_and_undo_redo_use_the_controlled_sdk_editor() {
    let mut s = State::default();
    let mut app = Application::new(model(&mut s)).unwrap();
    app.key(Key::plain('\t')).unwrap();
    app.edit(EditCommand::SelectAll).unwrap();
    app.insert_text("é界").unwrap();
    assert_eq!(s.values[0], "é界");
    app.edit(EditCommand::Undo).unwrap();
    assert_eq!(s.values[0], "guest");
    app.edit(EditCommand::Redo).unwrap();
    assert_eq!(s.values[0], "é界");
    app.insert_text("abcdef").unwrap();
    assert_eq!(s.values[0].chars().count(), 8);
    let count = s.accepted;
    app.key(Key::plain('✓')).unwrap();
    assert_eq!(s.accepted, count);
    assert_eq!(
        app.view().children()[1].content(),
        Some(s.values[0].as_str())
    );
}
#[test]
fn declined_proposals_restore_value_selection_and_undo_history() {
    let mut s = State::default();
    let mut app = Application::new(model(&mut s)).unwrap();
    app.key(Key::plain('\t')).unwrap();
    app.edit(EditCommand::SelectAll).unwrap();
    let before = app.input_context().unwrap().editor().clone();
    app.insert_text("!").unwrap();
    assert_eq!(s.values[0], "guest");
    assert_eq!(s.declined, 1);
    assert_eq!(app.input_context().unwrap().editor(), &before);
    app.insert_text("ok").unwrap();
    app.edit(EditCommand::Undo).unwrap();
    assert_eq!(s.values[0], "guest");
}
#[test]
fn independent_fields_full_width_ids_readonly_disabled_and_password_export() {
    let mut s = State::default();
    let mut app = Application::new(model(&mut s)).unwrap();
    app.key(Key::plain('\t')).unwrap();
    app.key(Key::plain('\r')).unwrap();
    assert_eq!(s.submits, 1);
    app.key(Key::plain('\t')).unwrap();
    app.edit(EditCommand::SelectAll).unwrap();
    assert_eq!(app.input_context().unwrap().export_text(), None);
    app.insert_text("new").unwrap();
    assert_eq!(s.values[1], "new");
    app.key(Key::plain('\t')).unwrap();
    app.edit(EditCommand::SelectAll).unwrap();
    app.insert_text("é\n界").unwrap();
    assert_eq!(s.values[2], "é\n界");
    app.key(Key::plain('\t')).unwrap();
    assert!(!app.input_context().unwrap().writable());
    let calls = s.accepted;
    app.insert_text("no").unwrap();
    assert_eq!(s.accepted, calls);
    assert_eq!(app.input_context().unwrap().editor().text(), "read only");
    app.key(Key::plain('\t')).unwrap();
    assert_eq!(app.focused_key(), Some("reset"));
    app.pointer(PointerEvent::Down, Some("disabled")).unwrap();
    app.pointer(PointerEvent::Up, Some("disabled")).unwrap();
    assert_eq!(s.accepted, calls);
}
#[test]
fn callback_errors_and_close_do_not_publish_a_partial_text_value() {
    for result in [-5, 2, 1] {
        let mut s = State::default();
        let mut m = model(&mut s);
        s.changed_result = result;
        m.update(Message::Edit(101, "new".into()));
        assert_eq!(m.tree.children()[1].content(), Some("guest"));
        if result == 1 {
            assert!(m.stop);
        } else {
            assert_eq!(
                m.error.unwrap().0,
                if result < 0 { -5 } else { dunit::errno::EINVAL }
            );
        }
    }
}
#[test]
fn invalid_text_callback_configuration_fails_before_build_or_transport() {
    let mut s = State::default();
    let cb = callbacks(&mut s);
    let env = [ptr::null()];
    let mut text = config();
    for case in 0..4 {
        let candidate = match case {
            0 => ptr::null(),
            _ => {
                text.abi_version = if case == 1 { 99 } else { 1 };
                text.struct_size = if case == 2 { 0 } else { 24 };
                text.changed = if case == 3 { None } else { Some(changed) };
                &text
            }
        };
        assert_eq!(
            unsafe {
                dunit_ui_run_text_v1(
                    0,
                    ptr::null(),
                    env.as_ptr(),
                    560,
                    460,
                    &cb,
                    ptr::null(),
                    candidate,
                )
            },
            dunit::errno::EINVAL as i32
        );
    }
    assert!(s.status.is_empty());
}
struct Raw {
    input: DunitUiInputV1,
    result: i32,
}
unsafe extern "C" fn raw(user: *mut c_void, _: u32, out: *mut DunitUiInputV1) -> i32 {
    let r = &*(user as *const Raw);
    *out = r.input;
    r.result
}
fn raw_default() -> Raw {
    Raw {
        input: DunitUiInputV1 {
            label: b"Label".as_ptr(),
            label_len: 5,
            placeholder: ptr::null(),
            placeholder_len: 0,
            max_chars: 128,
            flags: 0,
            reserved: 0,
            submit: 0,
        },
        result: 0,
    }
}
fn raw_view(r: &mut Raw, kind: u32, value: &str) -> dunit::Result<View<Message>> {
    let n = [node(kind, DUNIT_UI_ROOT, "root", value, 101)];
    let cfg = DunitUiTextV1 {
        configure: Some(raw),
        ..config()
    };
    unsafe { tree::convert_runtime(&n, r as *mut _ as *mut c_void, None, Some(&cfg)) }
}
#[test]
fn input_configuration_and_values_share_sdk_validation_and_ffi_bounds() {
    for case in 0..9 {
        let mut r = raw_default();
        let mut kind = DUNIT_UI_INPUT;
        match case {
            0 => r.input.flags = 0x80000000,
            1 => r.input.reserved = 1,
            2 => r.input.label_len = DUNIT_UI_MAX_TEXT_BYTES as usize + 1,
            3 => {
                r.input.placeholder = ptr::null();
                r.input.placeholder_len = 1;
            }
            4 => {
                r.input.label = b"\xff".as_ptr();
                r.input.label_len = 1;
            }
            5 => r.input.max_chars = 0,
            6 => {
                kind = DUNIT_UI_TEXTAREA;
                r.input.flags = DUNIT_UI_INPUT_PASSWORD;
            }
            7 => {
                kind = DUNIT_UI_TEXTAREA;
                r.input.flags = DUNIT_UI_INPUT_SUBMIT;
            }
            _ => r.input.flags = DUNIT_UI_INPUT_SOFT_WRAP,
        }
        assert!(raw_view(&mut r, kind, "text").is_err());
    }
    assert!(raw_view(&mut raw_default(), DUNIT_UI_INPUT, "a\nb").is_err());
    assert!(raw_view(&mut raw_default(), DUNIT_UI_TEXTAREA, "a\nb").is_ok());
    for kind in [DUNIT_UI_INPUT, DUNIT_UI_TEXTAREA] {
        assert!(
            unsafe { tree::convert(&[node(kind, DUNIT_UI_ROOT, "root", "text", 101)]) }.is_err()
        );
    }
}
#[test]
fn configure_errors_propagate_and_omitted_configuration_has_owned_defaults() {
    for result in [-5, 1] {
        let mut r = raw_default();
        r.result = result;
        assert_eq!(
            raw_view(&mut r, DUNIT_UI_INPUT, "value").err().unwrap().0,
            if result < 0 { -5 } else { dunit::errno::EINVAL }
        );
    }
    let n = [node(DUNIT_UI_INPUT, DUNIT_UI_ROOT, "key", "value", 101)];
    let cfg = DunitUiTextV1 {
        configure: None,
        ..config()
    };
    let view = unsafe { tree::convert_runtime(&n, ptr::null_mut(), None, Some(&cfg)) }.unwrap();
    assert_eq!(view.accessibility_label(), Some("key"));
    assert_eq!(view.content(), Some("value"));
}
#[test]
fn input_labels_and_placeholders_are_copied_before_foreign_storage_changes() {
    let mut r = raw_default();
    let mut label = b"Label".to_vec();
    let mut placeholder = b"Hint".to_vec();
    r.input.label = label.as_ptr();
    r.input.placeholder = placeholder.as_ptr();
    r.input.placeholder_len = placeholder.len();
    let view = raw_view(&mut r, DUNIT_UI_INPUT, "").unwrap();
    label.fill(b'x');
    placeholder.fill(b'y');
    assert_eq!(view.accessibility_label(), Some("Label"));
    let mut expected = super::checkable_tests::renderer();
    let mut actual = super::checkable_tests::renderer();
    let direct = View::input(
        "root",
        "Label",
        "",
        |s| Message::Edit(101, s),
        InputOptions {
            placeholder: "Hint".into(),
            max_chars: 128,
            ..InputOptions::default()
        },
    );
    expected.render(&direct, 240, 48).unwrap();
    actual.render(&view, 240, 48).unwrap();
    assert_eq!(expected.pixels(), actual.pixels());
}
struct Reference(State);
fn name(value: String) -> Message {
    Message::Edit(101, value)
}
fn password(value: String) -> Message {
    Message::Edit(102, value)
}
fn notes(value: String) -> Message {
    Message::Edit(u64::MAX, value)
}
impl Component for Reference {
    type Message = Message;
    fn update(&mut self, message: Message) {
        match message {
            Message::Action(id) => {
                self.0.action(id);
            }
            Message::Edit(id, value) => {
                self.0.edit(id, &value);
            }
        }
    }
    fn view(&self) -> View<Message> {
        let s = &self.0;
        View::column(
            "root",
            vec![
                View::text("title", "C SDK text editor"),
                View::input(
                    "name",
                    "Name",
                    s.values[0].clone(),
                    name,
                    InputOptions {
                        placeholder: "Name".into(),
                        max_chars: 8,
                        ..InputOptions::default()
                    },
                )
                .on_submit(Message::Action(201)),
                View::input(
                    "password",
                    "Password",
                    s.values[1].clone(),
                    password,
                    InputOptions {
                        placeholder: "Password".into(),
                        password: true,
                        max_chars: 128,
                        ..InputOptions::default()
                    },
                ),
                View::text_area(
                    "notes",
                    "Notes",
                    s.values[2].clone(),
                    notes,
                    InputOptions {
                        placeholder: "Notes".into(),
                        soft_wrap: true,
                        max_chars: 128,
                        ..InputOptions::default()
                    },
                ),
                View::input(
                    "readonly",
                    "Read only",
                    "read only",
                    name,
                    InputOptions {
                        placeholder: "Read only".into(),
                        read_only: true,
                        max_chars: 128,
                        ..InputOptions::default()
                    },
                ),
                View::input(
                    "disabled",
                    "Disabled",
                    "unavailable",
                    name,
                    InputOptions {
                        placeholder: "Disabled".into(),
                        max_chars: 128,
                        ..InputOptions::default()
                    },
                )
                .disabled(true),
                View::row(
                    "actions",
                    vec![
                        View::button("reset", "Reset", Message::Action(10)),
                        View::button("quit", "Quit", Message::Action(8)),
                    ],
                ),
                View::text(
                    "status",
                    alloc::format!(
                        "Accepted {} / declined {} / submits {}",
                        s.accepted,
                        s.declined,
                        s.submits
                    ),
                ),
            ],
        )
    }
}
fn replace<C: Component>(app: &mut Application<C>, value: &str)
where
    C::Message: Clone,
{
    app.edit(EditCommand::SelectAll).unwrap();
    for c in value.chars() {
        app.key(Key::plain(c)).unwrap();
    }
}
fn sequence<C: Component>(app: &mut Application<C>, phase: usize)
where
    C::Message: Clone,
{
    app.key(Key::plain('\t')).unwrap();
    if phase == 1 {
        replace(app, "alexander");
        app.key(Key::plain('\r')).unwrap();
        app.key(Key::plain('\t')).unwrap();
        replace(app, "xyz");
        app.key(Key::plain('\t')).unwrap();
        replace(app, "line1");
        app.key(Key::plain('\r')).unwrap();
        for c in "line2".chars() {
            app.key(Key::plain(c)).unwrap();
        }
        app.key(Key::plain('\t')).unwrap();
        app.key(Key::plain('z')).unwrap();
        app.key(Key::plain('\t')).unwrap();
        assert_eq!(app.focused_key(), Some("reset"));
        app.key(Key::plain('\t')).unwrap();
    } else {
        replace(app, "!");
        for _ in 0..4 {
            app.key(Key::plain('\t')).unwrap();
        }
        assert_eq!(app.focused_key(), Some("reset"));
        app.key(Key::plain(' ')).unwrap();
    }
}
#[test]
fn controlled_form_edits_rejection_and_reset_match_direct_rust_pixels() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sheet = std::fs::read_to_string(root.join("assets/tcc/text.dss")).unwrap();
    let mut s = State::default();
    let mut adapted = Application::new(model(&mut s)).unwrap();
    let mut direct = Application::new(Reference(State::default())).unwrap();
    for (phase, state) in ["initial", "edited", "reset"].iter().enumerate() {
        if phase > 0 {
            sequence(&mut adapted, phase);
            sequence(&mut direct, phase);
        }
        let mut expected = super::checkable_tests::renderer_with_style(&sheet);
        let mut actual = super::checkable_tests::renderer_with_style(&sheet);
        expected.render_application(&direct, 560, 460).unwrap();
        actual.render_application(&adapted, 560, 460).unwrap();
        assert_eq!(expected.pixels(), actual.pixels());
        if let Some(output) = std::env::var_os("DUNIT_C_GOLDEN_DIR") {
            let pixels: Vec<u8> = expected
                .pixels()
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect();
            std::fs::write(
                std::path::Path::new(&output).join(alloc::format!("c-ui-text-{state}.rgb")),
                pixels,
            )
            .unwrap();
        }
    }
    assert_eq!(s.accepted, 22);
    assert_eq!(s.declined, 1);
    assert_eq!(s.submits, 1);
    assert_eq!(s.resets, 1);
}
