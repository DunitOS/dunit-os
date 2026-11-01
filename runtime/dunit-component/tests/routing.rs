use dunit_component::{
    Application, Component, EventPhase as Phase, EventResponse as Response, InputOptions, Key,
    PointerEvent as P, UiEvent, UiEventKind as Kind, View,
};
#[derive(Clone)]
enum Msg {
    Event(UiEvent),
    Activate,
    Clear,
    Mode(u8),
    Text(String),
    Close,
}
struct Demo {
    events: Vec<UiEvent>,
    activated: usize,
    mode: u8,
    text: String,
    changed: bool,
}
fn observe(e: UiEvent) -> Response<Msg> {
    Response::message(Msg::Event(e))
}
fn prevent(e: UiEvent) -> Response<Msg> {
    observe(e).prevent()
}
fn stop(e: UiEvent) -> Response<Msg> {
    observe(e).stop()
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Event(e) => {
                let final_bubble = e.phase == Phase::Bubble && e.current_target == "root";
                self.events.push(e);
                if (3..=6).contains(&self.mode) || (self.mode == 8 && final_bubble) {
                    self.changed = true;
                }
            }
            Msg::Activate => self.activated += 1,
            Msg::Clear => self.events.clear(),
            Msg::Mode(m) => {
                self.mode = m;
                self.changed = false;
            }
            Msg::Text(t) => self.text = t,
            Msg::Close => {
                self.mode = 0;
                self.changed = false;
            }
        }
    }
    fn view(&self) -> View<Msg> {
        let target = if self.mode == 7 {
            View::input(
                "target",
                "Name",
                &self.text,
                Msg::Text,
                InputOptions::default(),
            )
        } else if self.mode == 5 && self.changed {
            View::text("target", "Replaced")
        } else {
            View::button("target", "Do it", Msg::Activate)
        };
        let target = target
            .disabled(matches!(self.mode, 4 | 8) && self.changed)
            .on_capture(observe)
            .on_bubble(observe);
        let children = if self.mode == 3 && self.changed {
            vec![]
        } else {
            vec![target]
        };
        let middle = View::column("middle", children)
            .on_capture(if self.mode == 2 { stop } else { observe })
            .on_bubble(if self.mode == 1 || self.mode == 7 {
                prevent
            } else {
                observe
            });
        let body = View::column("root", vec![middle])
            .on_capture(observe)
            .on_bubble(observe);
        View::modal(
            "host",
            body,
            (self.mode == 6 && self.changed).then(|| {
                View::message_box(
                    "modal",
                    "Confirm",
                    "Changed",
                    vec![dunit_component::DialogAction::new(
                        "close",
                        "Close",
                        Msg::Close,
                    )],
                    Some("close"),
                    Some(Msg::Close),
                )
            }),
        )
    }
}
fn app() -> Application<Demo> {
    Application::new(Demo {
        events: vec![],
        activated: 0,
        mode: 0,
        text: "hello".into(),
        changed: false,
    })
    .unwrap()
}
fn focused() -> Application<Demo> {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.update(Msg::Clear).unwrap();
    a
}
#[test]
fn capture_then_bubble_precedes_control_default_once() {
    let mut a = focused();
    a.key(Key::plain('\r')).unwrap();
    let order: Vec<_> = a
        .component()
        .events
        .iter()
        .map(|e| (e.current_target.as_str(), e.phase))
        .collect();
    assert_eq!(
        order,
        vec![
            ("root", Phase::Capture),
            ("middle", Phase::Capture),
            ("target", Phase::Capture),
            ("target", Phase::Bubble),
            ("middle", Phase::Bubble),
            ("root", Phase::Bubble)
        ]
    );
    assert_eq!(a.component().activated, 1);
    assert!(a.component().events.iter().all(|e| e.target == "target"));
}
#[test]
fn preventing_default_keeps_bubbling_and_stop_does_not_prevent_default() {
    let mut a = focused();
    a.update(Msg::Mode(1)).unwrap();
    a.key(Key::plain('\r')).unwrap();
    assert_eq!(a.component().activated, 0);
    assert!(a.component().events.last().unwrap().default_prevented);
    a.update(Msg::Mode(2)).unwrap();
    a.update(Msg::Clear).unwrap();
    a.key(Key::plain('\r')).unwrap();
    assert_eq!(a.component().events.len(), 2);
    assert_eq!(a.component().activated, 1);
}
#[test]
fn handler_mutations_cancel_stale_target_and_default_including_final_handler() {
    for mode in 3..=6 {
        let mut a = focused();
        a.update(Msg::Mode(mode)).unwrap();
        a.key(Key::plain('\r')).unwrap();
        assert_eq!(a.component().events.len(), 1);
        assert_eq!(a.component().activated, 0);
        if mode == 6 {
            assert_eq!(a.active_dialog(), Some("modal"));
        }
    }
}
#[test]
fn native_raw_keys_route_once_inside_bracketed_dispatch() {
    let mut a = focused();
    let out = a
        .begin_event_dispatch(
            Some("target"),
            Kind::NativeKey {
                scancode: 0x1c,
                character: 13,
                modifiers: 0,
            },
        )
        .unwrap();
    assert!(!out.default_prevented);
    a.key(Key::plain('\r')).unwrap();
    a.end_event_dispatch();
    assert_eq!(a.component().events.len(), 6);
    assert_eq!(a.component().activated, 1);
    a.key(Key::plain('\r')).unwrap();
    assert_eq!(a.component().events.len(), 12);
}
#[test]
fn prevented_release_or_leave_cancels_pressed_capture_without_later_click() {
    for cancelled in [P::Up, P::Leave, P::Down] {
        let mut a = app();
        a.pointer(P::Down, Some("target")).unwrap();
        assert_eq!(a.pointer_capture_key(), Some("target"));
        a.update(Msg::Mode(1)).unwrap();
        a.pointer(cancelled, Some("target")).unwrap();
        assert_eq!(a.pointer_capture_key(), None);
        a.update(Msg::Mode(0)).unwrap();
        a.pointer(P::Up, Some("target")).unwrap();
        assert_eq!(a.component().activated, 0);
    }
}
#[test]
fn release_outside_routes_to_capture_but_does_not_activate() {
    let mut a = app();
    a.pointer(P::Down, Some("target")).unwrap();
    a.update(Msg::Clear).unwrap();
    a.pointer(P::Up, None).unwrap();
    assert!(a.component().events.iter().all(|e| e.target == "target"));
    assert_eq!(a.pointer_capture_key(), None);
    assert_eq!(a.component().activated, 0);
}
#[test]
fn prevented_pointer_at_has_no_caret_or_drag_side_effect() {
    let mut a = app();
    a.update(Msg::Mode(7)).unwrap();
    let before = a.view().children()[0].children()[0].children()[0]
        .text_editor()
        .unwrap()
        .clone();
    a.pointer_at(P::Down, Some("target"), Some(("target", 4)))
        .unwrap();
    assert_eq!(
        a.view().children()[0].children()[0].children()[0].text_editor(),
        Some(&before)
    );
    assert_eq!(a.dragging_input_key(), None);
}

#[test]
fn final_bubble_mutation_is_revalidated_before_default() {
    let mut a = focused();
    a.update(Msg::Mode(8)).unwrap();
    a.key(Key::plain('\r')).unwrap();
    assert_eq!(a.component().events.len(), 6);
    assert_eq!(a.component().activated, 0);
}
