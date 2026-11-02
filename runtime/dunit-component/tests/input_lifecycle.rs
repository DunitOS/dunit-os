use dunit_component::{
    Application, Component, DialogAction, EditCommand, InputOptions, Key, MenuItem,
    PointerEvent as P, TextClickConfig, TextClickSample, TextClickState, View,
};
#[derive(Clone)]
enum Msg {
    Add,
    Toggle,
    Text(String),
    Disable,
    Remove,
    Modal,
    Close,
}
struct Model {
    calls: usize,
    checked: bool,
    value: String,
    disabled: bool,
    removed: bool,
    modal: bool,
}
impl Component for Model {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Add => self.calls += 1,
            Msg::Toggle => self.checked = !self.checked,
            Msg::Text(s) => self.value = s,
            Msg::Disable => self.disabled = true,
            Msg::Remove => self.removed = true,
            Msg::Modal => self.modal = true,
            Msg::Close => self.modal = false,
        }
    }
    fn view(&self) -> View<Msg> {
        View::modal(
            "host",
            View::column(
                "base",
                vec![
                    if self.removed {
                        View::text("button", "Replaced")
                    } else {
                        View::button("button", "Action", Msg::Add)
                            .disabled(self.disabled)
                            .tooltip("Action hint")
                    },
                    View::checkbox("check", "Check", self.checked, Msg::Toggle),
                    View::input(
                        "input",
                        "Text",
                        &self.value,
                        Msg::Text,
                        InputOptions::default(),
                    ),
                    View::menu(
                        "menu",
                        "Menu",
                        vec![MenuItem::action("item", "Action", Msg::Add)],
                    ),
                ],
            ),
            self.modal.then(|| {
                View::dialog(
                    "dialog",
                    "Title",
                    View::input(
                        "draft",
                        "Draft",
                        &self.value,
                        Msg::Text,
                        InputOptions::default(),
                    ),
                    vec![
                        DialogAction::new("accept", "Accept", Msg::Add),
                        DialogAction::new("cancel", "Cancel", Msg::Close),
                    ],
                    Some("accept"),
                    Some(Msg::Close),
                )
            }),
        )
    }
}
fn app() -> Application<Model> {
    Application::new(Model {
        calls: 0,
        checked: false,
        value: "alpha beta gamma".into(),
        disabled: false,
        removed: false,
        modal: false,
    })
    .unwrap()
}
fn focus(a: &mut Application<Model>, key: &str) {
    a.pointer(P::Down, Some(key)).unwrap();
    a.pointer(P::Up, Some(key)).unwrap();
}
fn pressed(a: &Application<Model>, key: &str) -> bool {
    fn find<'a>(v: &'a View<Msg>, key: &str) -> Option<&'a View<Msg>> {
        if v.key() == key {
            return Some(v);
        }
        v.children().iter().find_map(|v| find(v, key))
    }
    find(a.view(), key).unwrap().interaction().pressed
}
#[test]
fn held_space_and_enter_activate_only_on_matching_release_once() {
    for (scan, ch) in [(0x39, ' '), (0x1c, '\r')] {
        let mut a = app();
        a.key(Key::plain('\t')).unwrap();
        assert!(a.key_press(scan, Key::plain(ch)).unwrap());
        assert!(pressed(&a, "button"));
        for _ in 0..4 {
            assert!(a.key_press(scan, Key::plain(ch)).unwrap());
        }
        assert_eq!(a.component().calls, 0);
        assert!(!a.key_release(0x10, 0).unwrap());
        assert!(pressed(&a, "button"));
        assert!(a.key_release(scan, 0).unwrap());
        assert_eq!(a.component().calls, 1);
        assert!(!pressed(&a, "button"));
        assert!(!a.key_release(scan, 0).unwrap());
        assert_eq!(a.component().calls, 1);
    }
}
#[test]
fn cooked_legacy_activation_remains_immediate() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.key(Key::plain(' ')).unwrap();
    assert_eq!(a.component().calls, 1);
    assert!(!pressed(&a, "button"));
}
#[test]
fn escape_cancels_and_suppresses_repeats_until_break() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.key_press(0x39, Key::plain(' ')).unwrap();
    assert!(!a.key_press(1, Key::plain('\x1b')).unwrap());
    assert!(!pressed(&a, "button"));
    a.key_press(0x39, Key::plain(' ')).unwrap();
    a.key_release(0x39, 0).unwrap();
    assert_eq!(a.component().calls, 0);
    a.key_press(0x39, Key::plain(' ')).unwrap();
    a.key_release(0x39, 0).unwrap();
    assert_eq!(a.component().calls, 1);
}
#[test]
fn disable_replace_modal_and_focus_change_invalidate_pending_press() {
    for change in [Msg::Disable, Msg::Remove, Msg::Modal] {
        let mut a = app();
        a.key(Key::plain('\t')).unwrap();
        a.key_press(0x39, Key::plain(' ')).unwrap();
        a.update(change).unwrap();
        a.key_release(0x39, 0).unwrap();
        assert_eq!(a.component().calls, 0);
    }
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.key_press(0x39, Key::plain(' ')).unwrap();
    a.key(Key::plain('\t')).unwrap();
    a.key_release(0x39, 0).unwrap();
    assert_eq!(a.component().calls, 0);
}
#[test]
fn focus_out_cancels_capture_and_press_and_restores_valid_focus() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.key_press(0x39, Key::plain(' ')).unwrap();
    assert!(a.window_focus(false));
    assert!(!a.window_focused());
    assert_eq!(a.focused_key(), None);
    assert!(!pressed(&a, "button"));
    a.window_focus(true);
    assert_eq!(a.focused_key(), Some("button"));
    a.key_release(0x39, 0).unwrap();
    assert_eq!(a.component().calls, 0);
    a.pointer_at(P::Down, Some("input"), Some(("input", 2)))
        .unwrap();
    assert_eq!(a.dragging_input_key(), Some("input"));
    a.window_focus(false);
    assert_eq!(a.dragging_input_key(), None);
    a.update(Msg::Text("external".into())).unwrap();
    assert_eq!(a.focused_key(), None);
    a.window_focus(true);
    assert_eq!(a.focused_key(), Some("input"));
    assert_eq!(a.component().value, "external");
}
#[test]
fn focus_loss_dismisses_popup_and_tooltip_without_model_actions() {
    let mut a = app();
    focus(&mut a, "menu");
    assert!(a.popup_state().is_some());
    a.window_focus(false);
    assert!(a.popup_state().is_none());
    assert_eq!(a.component().calls, 0);
    a.window_focus(true);
    a.tooltip_pointer(Some("button"), 0);
    assert!(a.sample_tooltip(20, 10));
    assert_eq!(a.visible_tooltip(), Some("button"));
    a.window_focus(false);
    assert!(a.visible_tooltip().is_none());
    assert!(a.tooltip_candidate().is_none());
}
#[test]
fn pointer_motion_and_stray_release_do_not_activate_keyboard_press() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.key_press(0x39, Key::plain(' ')).unwrap();
    a.pointer(P::Move, Some("input")).unwrap();
    a.pointer(P::Up, Some("button")).unwrap();
    a.pointer(P::Leave, None).unwrap();
    assert!(pressed(&a, "button"));
    assert_eq!(a.component().calls, 0);
    a.key_release(0x39, 0).unwrap();
    assert_eq!(a.component().calls, 1);
}
#[test]
fn pointer_down_cancels_pending_keyboard_activation() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.key_press(0x39, Key::plain(' ')).unwrap();
    a.pointer(P::Down, Some("input")).unwrap();
    a.key_press(0x39, Key::plain(' ')).unwrap();
    a.key_release(0x39, 0).unwrap();
    assert_eq!(a.component().calls, 0);
    assert!(!pressed(&a, "button"));
}
#[test]
fn checkable_and_popup_trigger_use_make_break_without_repeat() {
    let mut a = app();
    focus(&mut a, "check");
    assert!(a.component().checked);
    a.key_press(0x39, Key::plain(' ')).unwrap();
    assert!(a.component().checked);
    a.key_release(0x39, 0).unwrap();
    assert!(!a.component().checked);
    a.pointer(P::Down, Some("menu")).unwrap();
    a.pointer(P::Leave, None).unwrap();
    a.key_press(0x39, Key::plain(' ')).unwrap();
    assert!(a.popup_state().is_none());
    a.key_release(0x39, 0).unwrap();
    assert!(a.popup_state().is_some());
    a.key_press(0x1c, Key::plain('\r')).unwrap();
    a.key(Key::plain('\r')).unwrap();
    assert_eq!(a.component().calls, 1);
    assert!(a.key_press(0x1c, Key::plain('\r')).unwrap());
    a.key_release(0x1c, 0).unwrap();
    assert_eq!(a.component().calls, 1);
}
#[test]
fn default_dialog_enter_arms_default_without_mutating_input() {
    let mut a = app();
    a.update(Msg::Modal).unwrap();
    focus(&mut a, "draft");
    let before = a.component().value.clone();
    a.key_press(0x1c, Key::plain('\r')).unwrap();
    assert!(pressed(&a, "accept"));
    assert_eq!(a.component().calls, 0);
    a.key_release(0x1c, 0).unwrap();
    assert_eq!(a.component().calls, 1);
    assert_eq!(a.component().value, before);
}
fn sample(t: u64) -> TextClickSample {
    TextClickSample {
        time_ms: t,
        x: 10.,
        y: 10.,
        config: TextClickConfig {
            interval_ms: 400,
            distance: 4.,
        },
    }
}
fn click(a: &mut Application<Model>, state: &mut TextClickState, index: usize, mods: u8, t: u64) {
    a.pointer_text_at_with_modifiers(
        P::Down,
        Some("input"),
        Some(("input", index)),
        state,
        sample(t),
        mods,
    )
    .unwrap();
    a.pointer_text_at_with_modifiers(
        P::Up,
        Some("input"),
        Some(("input", index)),
        state,
        sample(t + 1),
        mods,
    )
    .unwrap();
}
fn editor(a: &Application<Model>) -> &dunit_component::TextEditor {
    a.view().children()[0].children()[2].text_editor().unwrap()
}
#[test]
fn shift_click_extends_anchor_and_drag_keeps_it_without_history_edits() {
    let mut a = app();
    let mut state = TextClickState::default();
    click(&mut a, &mut state, 2, 0, 0);
    click(&mut a, &mut state, 8, 1, 100);
    assert_eq!(editor(&a).selection(), 2..8);
    a.pointer_text_at_with_modifiers(
        P::Down,
        Some("input"),
        Some(("input", 12)),
        &mut state,
        sample(200),
        1,
    )
    .unwrap();
    a.drag_text_selection("input", 1, &mut state);
    assert_eq!(editor(&a).anchor(), 2);
    assert_eq!(editor(&a).selection(), 1..2);
    assert!(!editor(&a).can_undo());
    assert_eq!(a.component().value, "alpha beta gamma");
}
#[test]
fn ctrl_and_ctrl_shift_click_select_whole_word_ranges() {
    let mut a = app();
    let mut state = TextClickState::default();
    click(&mut a, &mut state, 7, 2, 0);
    assert_eq!(editor(&a).selection(), 6..10);
    click(&mut a, &mut state, 13, 3, 50);
    assert_eq!(editor(&a).selection(), 6..16);
    a.edit(EditCommand::Insert('z')).unwrap();
    assert_eq!(a.component().value, "alpha z");
    a.edit(EditCommand::Undo).unwrap();
    assert_eq!(a.component().value, "alpha beta gamma");
}
#[test]
fn overlapping_activation_keys_keep_cancelled_press_tombstones_until_release() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.key_press(0x39, Key::plain(' ')).unwrap();
    a.key_press(1, Key::plain('\x1b')).unwrap();
    a.key_press(0x1c, Key::plain('\r')).unwrap();
    assert!(pressed(&a, "button"));
    a.key_press(0x39, Key::plain(' ')).unwrap();
    a.key_release(0x39, 0).unwrap();
    assert_eq!(
        a.component().calls,
        0,
        "old cancelledSpace release must not replace pendingEnter"
    );
    assert!(pressed(&a, "button"));
    a.key_release(0x1c, 0).unwrap();
    assert_eq!(a.component().calls, 1);
}
#[test]
fn ctrl_shift_click_inside_anchor_word_keeps_original_anchor_on_release_and_drag() {
    let mut a = app();
    let mut state = TextClickState::default();
    click(&mut a, &mut state, 8, 0, 0);
    click(&mut a, &mut state, 9, 3, 100);
    assert_eq!(editor(&a).anchor(), 8);
    assert_eq!(editor(&a).selection(), 8..10);
    a.pointer_text_at_with_modifiers(
        P::Down,
        Some("input"),
        Some(("input", 9)),
        &mut state,
        sample(200),
        3,
    )
    .unwrap();
    a.pointer_text_at_with_modifiers(
        P::Move,
        Some("input"),
        Some(("input", 7)),
        &mut state,
        sample(210),
        3,
    )
    .unwrap();
    a.pointer_text_at_with_modifiers(
        P::Up,
        Some("input"),
        Some(("input", 7)),
        &mut state,
        sample(220),
        3,
    )
    .unwrap();
    assert_eq!(editor(&a).anchor(), 8);
    assert_eq!(editor(&a).selection(), 6..8);
    assert!(!editor(&a).can_undo());
    assert_eq!(a.component().value, "alpha beta gamma");
}
#[test]
fn ordinary_text_key_repeats_remain_available_to_the_editor() {
    let mut a = app();
    focus(&mut a, "input");
    for _ in 0..3 {
        assert!(!a.key_press(0x39, Key::plain(' ')).unwrap());
        a.key(Key::plain(' ')).unwrap();
    }
    assert!(a.component().value.ends_with("   "));
    assert!(!a.key_release(0x39, 0).unwrap());
}
