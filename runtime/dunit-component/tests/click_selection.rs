use dunit_component::{
    Application, Component, EditCommand, InputOptions, Key, PointerEvent as P, TextClickConfig,
    TextClickSample, TextClickState, View,
};
#[derive(Clone)]
enum Msg {
    Text(String),
    External(String),
    Disable,
}
struct Model {
    value: String,
    disabled: bool,
    calls: usize,
}
impl Component for Model {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Text(s) => {
                self.value = s;
                self.calls += 1;
            }
            Msg::External(s) => self.value = s,
            Msg::Disable => self.disabled = true,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                View::text_area(
                    "input",
                    "Document",
                    &self.value,
                    Msg::Text,
                    InputOptions::default(),
                )
                .disabled(self.disabled),
                View::button("button", "Other", Msg::Disable),
            ],
        )
    }
}
fn app(text: &str) -> Application<Model> {
    Application::new(Model {
        value: text.into(),
        disabled: false,
        calls: 0,
    })
    .unwrap()
}
fn sample(time_ms: u64, x: f32, y: f32) -> TextClickSample {
    TextClickSample {
        time_ms,
        x,
        y,
        config: TextClickConfig {
            interval_ms: 400,
            distance: 5.,
        },
    }
}
fn pointer(
    app: &mut Application<Model>,
    clicks: &mut TextClickState,
    event: P,
    index: usize,
    s: TextClickSample,
) {
    app.pointer_text_at(event, Some("input"), Some(("input", index)), clicks, s)
        .unwrap();
}
fn click(
    app: &mut Application<Model>,
    clicks: &mut TextClickState,
    index: usize,
    s: TextClickSample,
) {
    pointer(app, clicks, P::Down, index, s);
    pointer(app, clicks, P::Up, index, s);
}
fn selection(app: &Application<Model>) -> core::ops::Range<usize> {
    app.view().children()[0].text_editor().unwrap().selection()
}
#[test]
fn single_double_triple_and_fourth_click_cycle_without_text_or_history_changes() {
    let mut app = app("alpha beta\nsecond\n");
    let mut clicks = TextClickState::default();
    click(&mut app, &mut clicks, 7, sample(0, 10., 10.));
    assert_eq!(selection(&app), 7..7);
    click(&mut app, &mut clicks, 7, sample(100, 10., 10.));
    assert_eq!(selection(&app), 6..10);
    click(&mut app, &mut clicks, 7, sample(200, 10., 10.));
    assert_eq!(selection(&app), 0..11);
    click(&mut app, &mut clicks, 7, sample(300, 10., 10.));
    assert_eq!(selection(&app), 7..7);
    assert_eq!(app.component().calls, 0);
    assert!(!app.view().children()[0].text_editor().unwrap().can_undo());
}
#[test]
fn elapsed_distance_and_monotonic_reset_are_bounded() {
    let mut app = app("alpha beta");
    let mut c = TextClickState::default();
    click(&mut app, &mut c, 7, sample(100, 10., 10.));
    click(&mut app, &mut c, 7, sample(501, 10., 10.));
    assert_eq!(selection(&app), 7..7);
    click(&mut app, &mut c, 7, sample(600, 16., 10.));
    assert_eq!(selection(&app), 7..7);
    click(&mut app, &mut c, 7, sample(599, 16., 10.));
    assert_eq!(selection(&app), 7..7);
    click(&mut app, &mut c, 7, sample(650, 19., 14.));
    assert_eq!(selection(&app), 6..10);
}
#[test]
fn zero_interval_disables_multi_click_and_missing_pointer_position_cancels_streak() {
    let mut app = app("alpha beta");
    let mut c = TextClickState::default();
    let mut s = sample(0, 1., 1.);
    s.config.interval_ms = 0;
    click(&mut app, &mut c, 7, s);
    s.time_ms = 1;
    click(&mut app, &mut c, 7, s);
    assert_eq!(selection(&app), 7..7);
    click(&mut app, &mut c, 7, sample(2, 1., 1.));
    app.pointer_text_at(P::Down, Some("button"), None, &mut c, sample(3, 1., 1.))
        .unwrap();
    click(&mut app, &mut c, 7, sample(4, 1., 1.));
    assert_eq!(selection(&app), 7..7);
}
#[test]
fn word_drag_retains_initial_word_and_reverses_across_anchor() {
    let mut app = app("one two three four");
    let mut c = TextClickState::default();
    click(&mut app, &mut c, 5, sample(0, 10., 10.));
    pointer(&mut app, &mut c, P::Down, 5, sample(100, 10., 10.));
    assert_eq!(selection(&app), 4..7);
    pointer(&mut app, &mut c, P::Move, 10, sample(110, 20., 10.));
    assert_eq!(selection(&app), 4..13);
    pointer(&mut app, &mut c, P::Move, 1, sample(120, 0., 10.));
    assert_eq!(selection(&app), 0..7);
    pointer(&mut app, &mut c, P::Move, 6, sample(130, 10., 10.));
    assert_eq!(selection(&app), 4..7);
    pointer(&mut app, &mut c, P::Up, 10, sample(140, 20., 10.));
    assert_eq!(selection(&app), 4..13);
    assert_eq!(app.dragging_input_key(), None);
}
#[test]
fn line_drag_and_stationary_autoscroll_share_whole_line_anchor() {
    let mut app = app("one\ntwo\nthree\nfour");
    let mut c = TextClickState::default();
    click(&mut app, &mut c, 5, sample(0, 10., 10.));
    click(&mut app, &mut c, 5, sample(50, 10., 10.));
    pointer(&mut app, &mut c, P::Down, 5, sample(100, 10., 10.));
    assert_eq!(selection(&app), 4..8);
    assert!(app.drag_text_selection("input", 10, &mut c));
    assert_eq!(selection(&app), 4..14);
    assert!(app.drag_text_selection("input", 1, &mut c));
    assert_eq!(selection(&app), 0..8);
    pointer(&mut app, &mut c, P::Up, 6, sample(120, 10., 10.));
    assert_eq!(selection(&app), 4..8);
    assert!(!app.drag_text_selection("input", 15, &mut c));
}
#[test]
fn non_ascii_offsets_are_normalized_and_empty_final_line_is_selectable() {
    let mut app = app("é界 abc\n");
    let mut c = TextClickState::default();
    click(&mut app, &mut c, 3, sample(0, 1., 1.));
    assert_eq!(selection(&app), 2..2);
    click(&mut app, &mut c, 3, sample(10, 1., 1.));
    assert_eq!(selection(&app), 0..5);
    click(&mut app, &mut c, 10, sample(20, 1., 1.));
    assert_eq!(selection(&app), 10..10);
}
#[test]
fn leave_external_text_and_disabled_transition_cancel_capture() {
    let mut app = app("alpha beta");
    let mut c = TextClickState::default();
    click(&mut app, &mut c, 7, sample(0, 1., 1.));
    pointer(&mut app, &mut c, P::Down, 7, sample(10, 1., 1.));
    app.update(Msg::External("replacement".into())).unwrap();
    assert!(!app.drag_text_selection("input", 2, &mut c));
    click(&mut app, &mut c, 2, sample(20, 1., 1.));
    assert_eq!(selection(&app), 2..2);
    app.pointer_text_at(P::Leave, None, None, &mut c, sample(30, 1., 1.))
        .unwrap();
    click(&mut app, &mut c, 2, sample(40, 1., 1.));
    assert_eq!(selection(&app), 2..2);
    app.update(Msg::Disable).unwrap();
    let old = selection(&app);
    click(&mut app, &mut c, 4, sample(50, 1., 1.));
    assert_eq!(selection(&app), old);
}
#[test]
fn word_replacement_is_one_controlled_edit_and_undo_restores_selected_word() {
    let mut app = app("alpha beta");
    let mut c = TextClickState::default();
    click(&mut app, &mut c, 7, sample(0, 1., 1.));
    click(&mut app, &mut c, 7, sample(10, 1., 1.));
    app.key(Key::plain('x')).unwrap();
    assert_eq!(app.component().value, "alpha x");
    assert_eq!(app.component().calls, 1);
    app.edit(EditCommand::Undo).unwrap();
    assert_eq!(app.component().value, "alpha beta");
    assert_eq!(selection(&app), 6..10);
}
