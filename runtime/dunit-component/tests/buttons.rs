use dunit_component::{
    Application, Attrs, Cascade, Color, Component, Font, Key, PointerEvent as P, Renderer, Sizing,
    View,
};

struct Buttons {
    count: usize,
    reversed: bool,
    hidden: bool,
    disabled: bool,
    replacement: bool,
}
#[derive(Clone)]
enum Msg {
    Add,
    Reverse,
    Hide,
    Show,
    Disable,
    Replace,
}
impl Component for Buttons {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Add => self.count += 1,
            Msg::Reverse => self.reversed = !self.reversed,
            Msg::Hide => self.hidden = true,
            Msg::Show => self.hidden = false,
            Msg::Disable => self.disabled = true,
            Msg::Replace => self.replacement = true,
        }
    }
    fn view(&self) -> View<Msg> {
        let mut children = vec![];
        if !self.hidden {
            children.push(if self.replacement {
                View::text("a", "Replacement")
            } else {
                View::button("a", format!("Add {}", self.count), Msg::Add).disabled(self.disabled)
            });
        }
        children.push(View::button("disabled", "Unavailable", Msg::Add).disabled(true));
        children.push(View::button("b", "Other", Msg::Add));
        if self.reversed {
            children.reverse();
        }
        View::column(
            "root",
            children
                .into_iter()
                .map(|v| {
                    v.layout(Attrs {
                        height: Sizing::Px(30.),
                        ..Attrs::default()
                    })
                })
                .collect(),
        )
    }
}
fn app() -> Application<Buttons> {
    Application::new(Buttons {
        count: 0,
        reversed: false,
        hidden: false,
        disabled: false,
        replacement: false,
    })
    .unwrap()
}
fn state(app: &Application<Buttons>, key: &str) -> dunit_widgets::Interaction {
    app.view()
        .children()
        .iter()
        .find(|v| v.key() == key)
        .unwrap()
        .interaction()
}
fn renderer() -> Renderer {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/fonts/DejaVuSans.ttf"
    ))
    .unwrap();
    let mut cascade = Cascade::new();
    cascade.push(dunit_component::style::parse("Column { background: #111111; } Button { background: #222222; color: #ffffff; font-size: 14; } Button:hover { background: #333333; } Button:focus { border-width: 2; border-color: #00ff00; } Button:active { background: #444444; } Button:disabled { background: #555555; }").unwrap());
    Renderer::new(Font::parse(bytes).unwrap(), cascade, Color::rgb(0, 0, 0))
}
#[test]
fn click_activates_on_release_and_cannot_activate_twice() {
    let mut app = app();
    app.pointer(P::Down, Some("a")).unwrap();
    assert!(state(&app, "a").pressed);
    assert_eq!(app.component().count, 0);
    app.pointer(P::Up, Some("a")).unwrap();
    assert_eq!(app.component().count, 1);
    assert!(!state(&app, "a").pressed);
    app.pointer(P::Up, Some("a")).unwrap();
    assert_eq!(app.component().count, 1);
    assert_eq!(app.focused_key(), Some("a"));
    assert_eq!(
        app.view().children()[0].accessibility_role(),
        Some("button")
    );
    assert_eq!(
        app.view().children()[0].accessibility_label(),
        Some("Add 1")
    );
}
#[test]
fn leaving_or_releasing_elsewhere_cancels_even_after_return() {
    for event in [P::Move, P::Leave, P::Up] {
        let mut app = app();
        app.pointer(P::Down, Some("a")).unwrap();
        app.pointer(event, Some("b")).unwrap();
        app.pointer(P::Move, Some("a")).unwrap();
        app.pointer(P::Up, Some("a")).unwrap();
        assert_eq!(app.component().count, 0);
        assert!(!state(&app, "a").pressed);
    }
}
#[test]
fn focus_cycles_both_ways_skips_disabled_and_activates_without_shortcuts() {
    let mut app = app();
    app.key(Key::plain('\t')).unwrap();
    assert_eq!(app.focused_key(), Some("a"));
    for character in [' ', '\r', '\n'] {
        assert!(app.key(Key::plain(character)).unwrap());
    }
    assert_eq!(app.component().count, 3);
    app.key(Key::plain('\t')).unwrap();
    assert_eq!(app.focused_key(), Some("b"));
    app.key(Key::plain('\t')).unwrap();
    assert_eq!(app.focused_key(), Some("a"));
    app.key(Key {
        character: '\t',
        modifiers: 1,
    })
    .unwrap();
    assert_eq!(app.focused_key(), Some("b"));
    assert!(!app
        .key(Key {
            character: ' ',
            modifiers: 2
        })
        .unwrap());
    assert_eq!(app.component().count, 3);
}
#[test]
fn stable_identity_survives_reorder_text_update_and_resize() {
    let mut app = app();
    app.pointer(P::Down, Some("a")).unwrap();
    app.update(Msg::Reverse).unwrap();
    assert_eq!(app.focused_key(), Some("a"));
    assert!(state(&app, "a").pressed);
    app.pointer(P::Up, Some("a")).unwrap();
    assert_eq!(app.component().count, 1);
    let mut renderer = renderer();
    renderer.render(app.view(), 200, 120).unwrap();
    renderer.render(app.view(), 320, 200).unwrap();
    assert_eq!(app.focused_key(), Some("a"));
    app.key(Key::plain(' ')).unwrap();
    assert_eq!(app.component().count, 2);
}
#[test]
fn removed_replaced_and_disabled_nodes_do_not_retain_focus_or_presses() {
    for change in [Msg::Hide, Msg::Replace, Msg::Disable] {
        let mut app = app();
        app.pointer(P::Down, Some("a")).unwrap();
        app.update(change).unwrap();
        assert_eq!(app.focused_key(), None);
        app.pointer(P::Up, Some("a")).unwrap();
        assert_eq!(app.component().count, 0);
        app.update(Msg::Show).unwrap();
        assert_eq!(app.focused_key(), None);
    }
    let mut app = app();
    app.pointer(P::Down, Some("disabled")).unwrap();
    app.pointer(P::Up, Some("disabled")).unwrap();
    assert_eq!(app.component().count, 0);
    assert_eq!(app.focused_key(), None);
}
#[test]
fn escape_and_background_click_cancel_presses() {
    let mut app = app();
    app.pointer(P::Down, Some("a")).unwrap();
    assert!(app.key(Key::plain('\u{1b}')).unwrap());
    app.pointer(P::Up, Some("a")).unwrap();
    assert_eq!(app.component().count, 0);
    app.pointer(P::Down, Some("root")).unwrap();
    assert_eq!(app.focused_key(), None);
    assert!(!app.key(Key::plain(' ')).unwrap());
}
#[test]
fn states_drive_dss_damage_and_real_layout_hit_testing() {
    let mut app = app();
    let mut r = renderer();
    r.render(app.view(), 200, 120).unwrap();
    assert_eq!(r.hit_key(10., 10.), Some("a"));
    assert_eq!(r.hit_key(-1., 10.), None);
    assert_eq!(r.hit_key(200., 10.), None);
    let before = r.pixels().to_vec();
    app.pointer(P::Move, r.hit_key(10., 10.)).unwrap();
    assert_eq!(r.render(app.view(), 200, 120).unwrap().damaged_pixels, 6000);
    assert_ne!(r.pixels(), before);
    for event in [P::Down, P::Up, P::Leave] {
        app.pointer(event, Some("a")).unwrap();
        r.render(app.view(), 200, 120).unwrap();
        let mut full = renderer();
        full.render(app.view(), 200, 120).unwrap();
        assert_eq!(r.pixels(), full.pixels());
    }
    assert_eq!(r.render(app.view(), 200, 120).unwrap().damaged_pixels, 0);
}
