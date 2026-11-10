use super::*;
use core::ptr;
use SliderCommand as S;
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
#[derive(Default)]
struct State {
    h: i64,
    v: i64,
    spin: i64,
    accepted: usize,
    declined: usize,
    edits: usize,
    toggles: usize,
    resets: usize,
    running: bool,
    note: String,
    status: String,
    result: i32,
}
fn state() -> State {
    State {
        note: "mixed".into(),
        ..State::default()
    }
}
impl State {
    fn number(&mut self, id: u64, value: i64) -> i32 {
        if self.result != 0 {
            return self.result;
        }
        if id == 101 && value == 45 {
            self.declined += 1;
            return 0;
        }
        match id {
            101 => self.h = value,
            u64::MAX => self.v = value,
            102 => self.spin = value,
            _ => panic!("readonly/disabled numeric dispatch"),
        };
        self.accepted += 1;
        0
    }
    fn action(&mut self, id: u64) -> i32 {
        match id {
            8 => 1,
            1 => {
                self.running = !self.running;
                self.toggles += 1;
                0
            }
            2 => {
                self.h = 0;
                self.v = 0;
                self.spin = 0;
                self.running = false;
                self.note = "mixed".into();
                self.resets += 1;
                0
            }
            _ => panic!("unknown action"),
        }
    }
    fn edit(&mut self, id: u64, value: &str) -> i32 {
        assert_eq!(id, 201);
        self.note = value.into();
        self.edits += 1;
        0
    }
}
unsafe extern "C" fn build(user: *mut c_void, out: *mut DunitUiNodeV1, capacity: u32) -> i32 {
    let s = &mut *(user as *mut State);
    s.status = alloc::format!("H {} / V {} / Spin {}", s.h, s.v, s.spin);
    let nodes = [
        node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0),
        node(DUNIT_UI_TEXT, 0, "title", "C SDK numeric controls", 0),
        node(DUNIT_UI_SLIDER, 0, "horizontal", "", 101),
        node(DUNIT_UI_ROW, 0, "body", "", 0),
        node(DUNIT_UI_VERTICAL_SLIDER, 3, "vertical", "", u64::MAX),
        node(DUNIT_UI_COLUMN, 3, "values", "", 0),
        node(DUNIT_UI_SPINBOX, 5, "spin", "", 102),
        node(DUNIT_UI_PROGRESS, 5, "progress", "", 0),
        node(DUNIT_UI_BUSY, 5, "busy", "", 0),
        node(DUNIT_UI_SPINBOX, 5, "readonly", "", 103),
        node(DUNIT_UI_SLIDER | DUNIT_UI_DISABLED, 0, "disabled", "", 104),
        node(DUNIT_UI_INPUT, 0, "note", &s.note, 201),
        node(DUNIT_UI_ROW, 0, "actions", "", 0),
        node(DUNIT_UI_BUTTON, 12, "toggle", "Busy", 1),
        node(DUNIT_UI_BUTTON, 12, "reset", "Reset", 2),
        node(DUNIT_UI_BUTTON, 12, "quit", "Quit", 8),
        node(DUNIT_UI_TEXT, 0, "status", &s.status, 0),
    ];
    assert!(capacity >= nodes.len() as u32);
    ptr::copy_nonoverlapping(nodes.as_ptr(), out, nodes.len());
    nodes.len() as i32
}
unsafe extern "C" fn configure(user: *mut c_void, index: u32, out: *mut DunitUiNumberV1) -> i32 {
    let s = &*(user as *const State);
    let c = &mut *out;
    c.min = -50;
    c.max = 50;
    c.step = 5;
    c.page_step = 10;
    match index {
        2 | 7 => c.value = s.h,
        4 => c.value = s.v,
        6 => c.value = s.spin,
        9 => c.flags = DUNIT_UI_NUMBER_READ_ONLY,
        8 => {
            c.flags = if s.running {
                DUNIT_UI_NUMBER_RUNNING
            } else {
                0
            }
        }
        10 => {}
        _ => panic!("not a numeric node"),
    };
    0
}
unsafe extern "C" fn changed(user: *mut c_void, id: u64, value: i64) -> i32 {
    (&mut *(user as *mut State)).number(id, value)
}
unsafe extern "C" fn input(_: *mut c_void, index: u32, out: *mut DunitUiInputV1) -> i32 {
    assert_eq!(index, 11);
    (*out).max_chars = 16;
    0
}
unsafe extern "C" fn edited(user: *mut c_void, id: u64, value: *const u8, length: usize) -> i32 {
    (&mut *(user as *mut State)).edit(
        id,
        core::str::from_utf8(core::slice::from_raw_parts(value, length)).unwrap(),
    )
}
unsafe extern "C" fn update(user: *mut c_void, id: u64) -> i32 {
    (&mut *(user as *mut State)).action(id)
}
fn callbacks(s: &mut State) -> DunitUiCallbacksV1 {
    DunitUiCallbacksV1 {
        abi_version: 1,
        struct_size: 32,
        user: s as *mut _ as *mut c_void,
        build: Some(build),
        update: Some(update),
    }
}
fn text() -> DunitUiTextV1 {
    DunitUiTextV1 {
        abi_version: 1,
        struct_size: 24,
        configure: Some(input),
        changed: Some(edited),
    }
}
fn numbers() -> DunitUiNumbersV1 {
    DunitUiNumbersV1 {
        abi_version: 1,
        struct_size: 24,
        configure: Some(configure),
        changed: Some(changed),
    }
}
fn model(s: &mut State) -> Model {
    let cb = callbacks(s);
    unsafe { Model::new_app(cb, None, Some(text()), Some(numbers())) }.unwrap()
}
#[test]
fn keyboard_ranges_decline_and_passive_tab_stops_are_sdk_controlled() {
    let mut s = state();
    let mut a = Application::new(model(&mut s)).unwrap();
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("horizontal"));
    a.slider_command(S::End).unwrap();
    assert_eq!(s.h, 50);
    a.slider_command(S::Decrease).unwrap();
    assert_eq!(s.h, 50);
    assert_eq!(s.declined, 1);
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("vertical"));
    a.slider_command(S::PageUp).unwrap();
    assert_eq!(s.v, 10);
    assert_eq!(
        a.view().children()[2].children()[0]
            .slider_range()
            .unwrap()
            .step,
        5
    );
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("spin"));
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("readonly"));
    let calls = s.accepted;
    a.spinbox_step(S::Increase).unwrap();
    assert_eq!(s.accepted, calls);
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("note"));
}
#[test]
fn signed_spinbox_draft_commits_through_the_numeric_callback() {
    let mut s = state();
    let mut a = Application::new(model(&mut s)).unwrap();
    for _ in 0..3 {
        a.key(Key::plain('\t')).unwrap();
    }
    a.edit(EditCommand::SelectAll).unwrap();
    for ch in "-15".chars() {
        a.key(Key::plain(ch)).unwrap();
    }
    assert_eq!(s.accepted, 0);
    a.key(Key::plain('\r')).unwrap();
    assert_eq!(s.spin, -15);
    assert_eq!(s.accepted, 1);
}
#[test]
fn errors_close_and_invalid_rebuild_keep_the_prior_numeric_view() {
    for ret in [-5, 2, 1] {
        let mut s = state();
        let mut m = model(&mut s);
        s.result = ret;
        m.update(Message::Number(101, 10));
        assert_eq!(m.tree.children()[1].slider_range().unwrap().value, 0);
        if ret == 1 {
            assert!(m.stop);
        } else {
            assert_eq!(
                m.error.unwrap().0,
                if ret < 0 { -5 } else { dunit::errno::EINVAL }
            );
        }
    }
    let mut s = state();
    let mut m = model(&mut s);
    m.update(Message::Number(101, 51));
    assert_eq!(m.error.unwrap().0, dunit::errno::EINVAL);
    assert_eq!(m.tree.children()[1].slider_range().unwrap().value, 0);
}
fn raw_config() -> DunitUiNumberV1 {
    DunitUiNumberV1 {
        label: b"Range".as_ptr(),
        label_len: 5,
        min: 0,
        max: 100,
        step: 5,
        page_step: 10,
        value: 0,
        flags: 0,
        reserved: 0,
    }
}
struct Raw {
    config: DunitUiNumberV1,
    result: i32,
    seen: Option<(u64, i64)>,
}
unsafe extern "C" fn raw(user: *mut c_void, _: u32, out: *mut DunitUiNumberV1) -> i32 {
    let r = &*(user as *const Raw);
    *out = r.config;
    r.result
}
fn raw_view(r: &mut Raw, kind: u32, with_changed: bool) -> dunit::Result<View<Message>> {
    let nodes = [node(kind, DUNIT_UI_ROOT, "root", "", u64::MAX)];
    let cfg = DunitUiNumbersV1 {
        configure: Some(raw),
        changed: if with_changed { Some(changed) } else { None },
        ..numbers()
    };
    unsafe { tree::convert_app(&nodes, r as *mut _ as *mut c_void, None, None, Some(&cfg)) }
}
#[test]
fn numeric_flags_ranges_labels_and_callback_errors_are_validated() {
    for case in 0..9 {
        let mut r = Raw {
            config: raw_config(),
            result: 0,
            seen: None,
        };
        match case {
            0 => r.config.min = 101,
            1 => r.config.step = 0,
            2 => r.config.page_step = 7,
            3 => r.config.value = 3,
            4 => r.config.flags = 0x80000000,
            5 => r.config.reserved = 1,
            6 => {
                r.config.label = ptr::null();
                r.config.label_len = 1;
            }
            7 => {
                r.config.label = b"\xff".as_ptr();
                r.config.label_len = 1;
            }
            _ => r.config.label_len = DUNIT_UI_MAX_TEXT_BYTES as usize + 1,
        }
        assert!(raw_view(&mut r, DUNIT_UI_SLIDER, true).is_err());
    }
    for result in [-5, 1] {
        let mut r = Raw {
            config: raw_config(),
            result,
            seen: None,
        };
        assert_eq!(
            raw_view(&mut r, DUNIT_UI_SLIDER, true).err().unwrap().0,
            if result < 0 { -5 } else { dunit::errno::EINVAL }
        );
    }
}
#[test]
fn progress_and_busy_need_no_change_callback_but_interactive_controls_do() {
    let mut r = Raw {
        config: raw_config(),
        result: 0,
        seen: None,
    };
    assert!(raw_view(&mut r, DUNIT_UI_PROGRESS, false).is_ok());
    r.config.flags = DUNIT_UI_NUMBER_RUNNING;
    let v = raw_view(&mut r, DUNIT_UI_BUSY, false).unwrap();
    assert_eq!(v.accessibility_busy(), Some(true));
    r.config.flags = 0;
    for kind in [DUNIT_UI_SLIDER, DUNIT_UI_VERTICAL_SLIDER, DUNIT_UI_SPINBOX] {
        assert!(raw_view(&mut r, kind, false).is_err());
        assert!(unsafe { tree::convert(&[node(kind, DUNIT_UI_ROOT, "root", "", 1)]) }.is_err());
    }
    r.config.value = 101;
    assert!(raw_view(&mut r, DUNIT_UI_PROGRESS, false).is_err());
    r.config = raw_config();
    r.config.flags = DUNIT_UI_NUMBER_READ_ONLY;
    assert!(raw_view(&mut r, DUNIT_UI_PROGRESS, false).is_err());
    assert!(
        raw_view(&mut r, DUNIT_UI_SPINBOX, true)
            .unwrap()
            .spinbox_range()
            .unwrap()
            .read_only
    );
}
#[test]
fn full_signed_range_and_full_width_message_id_cross_the_model_callback() {
    unsafe extern "C" fn one(_: *mut c_void, out: *mut DunitUiNodeV1, cap: u32) -> i32 {
        assert!(cap > 0);
        *out = node(DUNIT_UI_SLIDER, DUNIT_UI_ROOT, "root", "", u64::MAX);
        1
    }
    unsafe extern "C" fn receive(user: *mut c_void, id: u64, value: i64) -> i32 {
        let r = &mut *(user as *mut Raw);
        r.seen = Some((id, value));
        r.config.value = value;
        0
    }
    let mut r = Raw {
        config: raw_config(),
        result: 0,
        seen: None,
    };
    r.config.min = i64::MIN;
    r.config.max = i64::MAX;
    r.config.step = 1;
    r.config.value = i64::MIN;
    let cb = DunitUiCallbacksV1 {
        abi_version: 1,
        struct_size: 32,
        user: &mut r as *mut _ as *mut c_void,
        build: Some(one),
        update: Some(update),
    };
    let cfg = DunitUiNumbersV1 {
        configure: Some(raw),
        changed: Some(receive),
        ..numbers()
    };
    let m = unsafe { Model::new_app(cb, None, None, Some(cfg)) }.unwrap();
    let mut a = Application::new(m).unwrap();
    a.key(Key::plain('\t')).unwrap();
    a.slider_command(S::End).unwrap();
    assert_eq!(r.seen, Some((u64::MAX, i64::MAX)));
    a.slider_command(S::Home).unwrap();
    assert_eq!(r.seen, Some((u64::MAX, i64::MIN)));
}
#[test]
fn unified_app_and_nested_record_versions_fail_before_build() {
    let mut s = state();
    let cb = callbacks(&mut s);
    let env = [ptr::null()];
    let mut cfg = numbers();
    let mut app = DunitUiAppV1 {
        abi_version: 1,
        struct_size: 32,
        style: ptr::null(),
        text: ptr::null(),
        numbers: &cfg,
    };
    for case in 0..5 {
        let candidate = if case == 0 {
            ptr::null()
        } else {
            app.abi_version = if case == 1 { 99 } else { 1 };
            app.struct_size = if case == 2 { 0 } else { 32 };
            cfg.abi_version = if case == 3 { 99 } else { 1 };
            cfg.struct_size = if case == 4 { 0 } else { 24 };
            app.numbers = &cfg;
            &app
        };
        assert_eq!(
            unsafe { dunit_ui_run_app_v1(0, ptr::null(), env.as_ptr(), 560, 520, &cb, candidate) },
            dunit::errno::EINVAL as i32
        );
    }
    assert!(s.status.is_empty());
}
#[test]
fn numeric_labels_are_owned_and_passive_disabled_state_uses_sdk_painting() {
    let mut bytes = b"Range".to_vec();
    let mut r = Raw {
        config: raw_config(),
        result: 0,
        seen: None,
    };
    r.config.label = bytes.as_ptr();
    let view = raw_view(&mut r, DUNIT_UI_PROGRESS | DUNIT_UI_DISABLED, false).unwrap();
    bytes.fill(b'x');
    assert_eq!(view.accessibility_label(), Some("Range"));
    assert!(view.interaction().disabled);
}
struct Reference(State);
fn h(v: i64) -> Message {
    Message::Number(101, v)
}
fn vertical(v: i64) -> Message {
    Message::Number(u64::MAX, v)
}
fn spin(v: i64) -> Message {
    Message::Number(102, v)
}
fn edit(v: String) -> Message {
    Message::Edit(201, v)
}
fn options() -> SliderOptions {
    SliderOptions {
        min: -50,
        max: 50,
        step: 5,
        page_step: 10,
    }
}
impl Component for Reference {
    type Message = Message;
    fn update(&mut self, m: Message) {
        match m {
            Message::Rename(_, _) => panic!("unexpected rename in reference"),
            Message::Refresh => panic!("unexpected transfer refresh in reference"),
            Message::Collection(_, _) => panic!("unexpected collection event in numeric reference"),
            Message::Action(id) => {
                self.0.action(id);
            }
            Message::Edit(id, v) => {
                self.0.edit(id, &v);
            }
            Message::Number(id, v) => {
                self.0.number(id, v);
            }
        }
    }
    fn view(&self) -> View<Message> {
        let s = &self.0;
        View::column(
            "root",
            vec![
                View::text("title", "C SDK numeric controls"),
                View::slider("horizontal", "horizontal", s.h, options(), h),
                View::row(
                    "body",
                    vec![
                        View::vertical_slider("vertical", "vertical", s.v, options(), vertical),
                        View::column(
                            "values",
                            vec![
                                View::spinbox(
                                    "spin",
                                    "spin",
                                    s.spin,
                                    SpinboxOptions {
                                        range: options(),
                                        read_only: false,
                                    },
                                    spin,
                                ),
                                View::progress(
                                    "progress",
                                    "progress",
                                    s.h,
                                    ProgressOptions::new(-50, 50),
                                ),
                                View::busy("busy", "busy", s.running),
                                View::spinbox(
                                    "readonly",
                                    "readonly",
                                    0,
                                    SpinboxOptions {
                                        range: options(),
                                        read_only: true,
                                    },
                                    spin,
                                ),
                            ],
                        ),
                    ],
                ),
                View::slider("disabled", "disabled", 0, options(), h).disabled(true),
                View::input(
                    "note",
                    "note",
                    s.note.clone(),
                    edit,
                    InputOptions {
                        max_chars: 16,
                        ..InputOptions::default()
                    },
                ),
                View::row(
                    "actions",
                    vec![
                        View::button("toggle", "Busy", Message::Action(1)),
                        View::button("reset", "Reset", Message::Action(2)),
                        View::button("quit", "Quit", Message::Action(8)),
                    ],
                ),
                View::text(
                    "status",
                    alloc::format!("H {} / V {} / Spin {}", s.h, s.v, s.spin),
                ),
            ],
        )
    }
}
fn sequence<C: Component>(a: &mut Application<C>, phase: usize)
where
    C::Message: Clone,
{
    if phase == 1 {
        a.key(Key::plain('\t')).unwrap();
        a.slider_command(S::Increase).unwrap();
        a.slider_command(S::Increase).unwrap();
        a.slider_command(S::End).unwrap();
        a.key(Key::plain('\t')).unwrap();
        a.slider_command(S::Increase).unwrap();
        a.slider_command(S::PageUp).unwrap();
        a.key(Key::plain('\t')).unwrap();
        a.edit(EditCommand::SelectAll).unwrap();
        for c in "-15".chars() {
            a.key(Key::plain(c)).unwrap();
        }
        a.key(Key::plain('\r')).unwrap();
        a.key(Key::plain('\t')).unwrap();
        a.spinbox_step(S::Increase).unwrap();
        a.key(Key::plain('\t')).unwrap();
        assert_eq!(a.focused_key(), Some("note"));
        a.edit(EditCommand::SelectAll).unwrap();
        for c in "ok".chars() {
            a.key(Key::plain(c)).unwrap();
        }
        a.key(Key::plain('\t')).unwrap();
        a.key(Key::plain(' ')).unwrap();
        a.key(Key::plain('\t')).unwrap();
    } else {
        a.key(Key::plain('\t')).unwrap();
        a.key(Key::plain('\t')).unwrap();
        a.slider_command(S::Decrease).unwrap();
        for _ in 0..6 {
            a.key(Key::plain('\t')).unwrap();
        }
        assert_eq!(a.focused_key(), Some("reset"));
        a.key(Key::plain(' ')).unwrap();
    }
}
#[test]
fn mixed_numeric_text_and_passive_views_match_direct_sdk_pixels() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sheet = std::fs::read_to_string(root.join("assets/tcc/numbers.dss")).unwrap();
    let mut s = state();
    let mut adapted = Application::new(model(&mut s)).unwrap();
    let mut direct = Application::new(Reference(state())).unwrap();
    for (phase, name) in ["initial", "changed", "reset"].iter().enumerate() {
        if phase > 0 {
            sequence(&mut adapted, phase);
            sequence(&mut direct, phase);
        }
        let mut expected = super::checkable_tests::renderer_with_style(&sheet);
        let mut actual = super::checkable_tests::renderer_with_style(&sheet);
        expected.render_application(&direct, 560, 520).unwrap();
        actual.render_application(&adapted, 560, 520).unwrap();
        assert_eq!(expected.pixels(), actual.pixels());
        if let Some(output) = std::env::var_os("DUNIT_C_GOLDEN_DIR") {
            let pixels: Vec<u8> = expected
                .pixels()
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect();
            std::fs::write(
                std::path::Path::new(&output).join(alloc::format!("c-ui-numbers-{name}.rgb")),
                pixels,
            )
            .unwrap();
        }
    }
    assert_eq!(s.accepted, 6);
    assert_eq!(s.declined, 1);
    assert_eq!(s.edits, 2);
    assert_eq!(s.resets, 1);
    assert!(!s.running);
}
