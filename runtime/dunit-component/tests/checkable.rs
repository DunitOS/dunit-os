use dunit_component::{
    Application, Component, Key, Navigation as N, PointerEvent as P, UiError, View,
};
#[derive(Clone)]
enum Msg {
    Toggle(usize),
    Select(usize),
    Noop,
    Disable,
    Reorder,
    Group,
    Retype,
}
struct Demo {
    values: [bool; 3],
    selected: usize,
    calls: usize,
    disabled: bool,
    reverse: bool,
    group: bool,
    retype: bool,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Toggle(i) => {
                self.values[i] = !self.values[i];
                self.calls += 1;
            }
            Msg::Select(i) => {
                self.selected = i;
                self.calls += 1;
            }
            Msg::Noop => {}
            Msg::Disable => self.disabled = true,
            Msg::Reorder => self.reverse = !self.reverse,
            Msg::Group => self.group = !self.group,
            Msg::Retype => self.retype = !self.retype,
        }
    }
    fn view(&self) -> View<Msg> {
        let mut controls = vec![
            if self.retype {
                View::checkbox("toggle", "Checkbox", self.values[0], Msg::Toggle(0))
            } else {
                View::toggle_button("toggle", "Toggle", self.values[0], Msg::Toggle(0))
            },
            View::checkbox("check", "Check", self.values[1], Msg::Toggle(1))
                .disabled(self.disabled),
            View::switch("switch", "Switch", self.values[2], Msg::Toggle(2)),
            View::radio(
                "a",
                "Alpha",
                if self.group { "other" } else { "main" },
                self.selected == 0,
                Msg::Select(0),
            ),
            View::radio("disabled", "Disabled", "main", false, Msg::Select(99)).disabled(true),
            View::radio("b", "Beta", "main", self.selected == 1, Msg::Select(1)),
            View::radio("c", "Gamma", "main", self.selected == 2, Msg::Select(2)),
        ];
        if self.reverse {
            controls.reverse();
        }
        View::column("root", controls)
    }
}
fn app() -> Application<Demo> {
    Application::new(Demo {
        values: [false; 3],
        selected: 0,
        calls: 0,
        disabled: false,
        reverse: false,
        group: false,
        retype: false,
    })
    .unwrap()
}
fn node<'a>(app: &'a Application<Demo>, key: &str) -> &'a View<Msg> {
    app.view()
        .children()
        .iter()
        .find(|v| v.key() == key)
        .unwrap()
}
fn click(app: &mut Application<Demo>, key: &str) {
    app.pointer(P::Down, Some(key)).unwrap();
    app.pointer(P::Up, Some(key)).unwrap();
}
#[test]
fn checkable_controls_toggle_by_click_space_enter_and_keep_model_values() {
    for (i, key) in ["toggle", "check", "switch"].into_iter().enumerate() {
        let mut a = app();
        click(&mut a, key);
        assert!(a.component().values[i]);
        assert!(node(&a, key).interaction().checked);
        a.key(Key::plain(' ')).unwrap();
        assert!(!a.component().values[i]);
        a.key(Key::plain('\r')).unwrap();
        assert!(a.component().values[i]);
        a.update(Msg::Noop).unwrap();
        assert!(node(&a, key).interaction().checked);
        assert_eq!(a.component().calls, 3);
    }
}
#[test]
fn declined_activation_does_not_mutate_checked_behind_the_model() {
    struct Declines;
    impl Component for Declines {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self) -> View<()> {
            View::checkbox("c", "Check", false, ())
        }
    }
    let mut a = Application::new(Declines).unwrap();
    a.key(Key::plain('\t')).unwrap();
    a.key(Key::plain(' ')).unwrap();
    assert!(!a.view().interaction().checked);
    a.pointer(P::Down, Some("c")).unwrap();
    a.pointer(P::Up, Some("c")).unwrap();
    assert!(!a.view().interaction().checked);
}
#[test]
fn radio_group_has_one_tab_stop_arrows_wrap_skip_disabled_and_select_once() {
    let mut a = app();
    for expected in ["toggle", "check", "switch", "a"] {
        a.key(Key::plain('\t')).unwrap();
        assert_eq!(a.focused_key(), Some(expected));
    }
    assert!(a.key(Key::plain(' ')).unwrap());
    assert_eq!(a.component().calls, 0);
    for (dir, index, key) in [
        (N::Right, 1, "b"),
        (N::Down, 2, "c"),
        (N::Right, 0, "a"),
        (N::Up, 2, "c"),
        (N::Left, 1, "b"),
    ] {
        assert!(a.navigate(dir).unwrap());
        assert_eq!(a.component().selected, index);
        assert_eq!(a.focused_key(), Some(key));
        assert_eq!(
            a.view()
                .children()
                .iter()
                .filter(|v| v.accessibility_role() == Some("radio") && v.interaction().checked)
                .count(),
            1
        );
    }
    assert_eq!(a.component().calls, 5);
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("toggle"));
    a.key(Key {
        character: '\t',
        modifiers: 1,
    })
    .unwrap();
    assert_eq!(a.focused_key(), Some("b"));
    click(&mut a, "b");
    assert_eq!(a.component().calls, 5);
    click(&mut a, "a");
    assert_eq!(a.component().calls, 6);
    assert_eq!(a.component().selected, 0);
}
#[test]
fn disabled_and_cancelled_controls_never_change_the_model() {
    let mut a = app();
    click(&mut a, "check");
    a.update(Msg::Disable).unwrap();
    assert!(node(&a, "check").interaction().checked);
    assert!(!node(&a, "check").interaction().focused);
    click(&mut a, "check");
    click(&mut a, "disabled");
    assert_eq!(a.component().calls, 1);
    for key in ["toggle", "switch", "b"] {
        a.pointer(P::Down, Some(key)).unwrap();
        a.pointer(P::Leave, None).unwrap();
        a.pointer(P::Move, Some(key)).unwrap();
        a.pointer(P::Up, Some(key)).unwrap();
    }
    assert_eq!(a.component().calls, 1);
}
#[test]
fn transient_state_survives_reorder_but_checked_always_comes_from_new_view() {
    let mut a = app();
    click(&mut a, "toggle");
    a.update(Msg::Toggle(0)).unwrap();
    assert!(!node(&a, "toggle").interaction().checked);
    assert_eq!(a.focused_key(), Some("toggle"));
    a.update(Msg::Reorder).unwrap();
    assert_eq!(a.focused_key(), Some("toggle"));
    a.update(Msg::Retype).unwrap();
    assert_eq!(a.focused_key(), None);
    click(&mut a, "a");
    assert_eq!(a.focused_key(), Some("a"));
    a.update(Msg::Group).unwrap();
    assert_eq!(a.focused_key(), None);
}
#[test]
fn invalid_radio_groups_are_rejected_and_independent_groups_are_allowed() {
    let radio = |key, group, checked| View::radio(key, "Radio", group, checked, ());
    assert_eq!(
        radio("a", " ", false).validate(),
        Err(UiError::InvalidRadioGroup)
    );
    assert_eq!(
        View::column("root", vec![radio("a", "g", true), radio("b", "g", true)]).validate(),
        Err(UiError::InvalidRadioGroup)
    );
    assert!(
        View::column("root", vec![radio("a", "g", true), radio("b", "h", true)])
            .validate()
            .is_ok()
    );
    let mut a = Application::new(ViewComponent(View::column(
        "root",
        vec![
            radio("a", "g", false).disabled(true),
            radio("b", "g", false),
            radio("c", "h", false),
        ],
    )))
    .unwrap();
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("b"));
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("c"));
    assert!(a.navigate(N::Right).unwrap());
    assert_eq!(a.focused_key(), Some("c"));
}
struct ViewComponent(View<()>);
impl Component for ViewComponent {
    type Message = ();
    fn update(&mut self, _: ()) {}
    fn view(&self) -> View<()> {
        self.0.clone()
    }
}
#[test]
fn accessibility_distinguishes_pressed_checked_and_radio_roles() {
    let a = app();
    assert_eq!(node(&a, "toggle").accessibility_role(), Some("button"));
    assert_eq!(node(&a, "toggle").accessibility_pressed(), Some(false));
    assert_eq!(node(&a, "toggle").accessibility_checked(), None);
    for (key, role) in [("check", "checkbox"), ("switch", "switch"), ("a", "radio")] {
        assert_eq!(node(&a, key).accessibility_role(), Some(role));
        assert_eq!(node(&a, key).accessibility_checked(), Some(key == "a"));
        assert!(node(&a, key).accessibility_label().is_some());
    }
}

#[test]
fn checked_repaint_matches_full_canvas_and_theme_changes_keep_values() {
    use dunit_component::{Attrs, Font, Renderer, Sizing, Theme};
    let base = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/dwm/sdk/base.dss"
    ))
    .unwrap();
    let profile = |name| {
        std::fs::read_to_string(format!(
            "{}/../../assets/dwm/sdk/{name}.dss",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    };
    let font = || {
        Font::parse(
            std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/fonts/DejaVuSans.ttf"
            ))
            .unwrap(),
        )
        .unwrap()
    };
    let make = |name| {
        let theme = Theme::parse(&profile(name), &base, "").unwrap();
        Renderer::new(font(), theme.cascade, theme.background)
    };
    struct Gallery([bool; 3]);
    impl Component for Gallery {
        type Message = usize;
        fn update(&mut self, i: usize) {
            self.0[i] = !self.0[i];
        }
        fn view(&self) -> View<usize> {
            View::column(
                "root",
                vec![
                    View::checkbox("c", "Checkbox", self.0[0], 0),
                    View::switch("s", "Switch", self.0[1], 1),
                    View::toggle_button("t", "Toggle", self.0[2], 2),
                ]
                .into_iter()
                .map(|v| {
                    v.layout(Attrs {
                        height: Sizing::Px(44.),
                        ..Attrs::default()
                    })
                })
                .collect(),
            )
        }
    }
    let mut a = Application::new(Gallery([false; 3])).unwrap();
    let mut r = make("dark");
    r.render(a.view(), 320, 240).unwrap();
    for i in 0..3 {
        let before = r.pixels().to_vec();
        a.update(i).unwrap();
        let stats = r.render(a.view(), 320, 240).unwrap();
        assert!(stats.damaged_pixels > 0 && stats.damaged_pixels < 320 * 240);
        assert_ne!(r.pixels(), before);
        let mut full = make("dark");
        full.render(a.view(), 320, 240).unwrap();
        assert_eq!(r.pixels(), full.pixels());
    }
    let dark = r.pixels().to_vec();
    let light = Theme::parse(&profile("light"), &base, "").unwrap();
    r.set_theme(light, None);
    r.render(a.view(), 320, 240).unwrap();
    assert_ne!(r.pixels(), dark);
    assert_eq!(a.component().0, [true; 3]);
    let theme = Theme::parse(&profile("dark"), &base, "").unwrap();
    r.set_theme(theme, None);
    r.render(a.view(), 320, 240).unwrap();
    assert_eq!(r.pixels(), dark);
    // Unsupported appearance values are rejected before an existing theme can be replaced.
    for overlay in [
        "Checkbox { indicator-position: 2; }",
        "Radio { indicator-width: -1; }",
        "Switch { indicator-mark-color: 12; }",
        "Switch { indicator-height: huge; }",
    ] {
        assert!(Theme::parse(&profile("dark"), &base, overlay).is_err());
    }
}
