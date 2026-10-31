use dunit_component::{
    Application, Component, EditCommand as E, Font, Key, PointerEvent as P, Renderer,
    SliderCommand as S, SpinButton as B, SpinboxOptions as O, Theme, UiError, View,
};
#[derive(Clone)]
enum Msg {
    Value(i64),
    Reject,
    ReadOnly,
    Disable,
    External(i64),
    Ping,
    Apply,
}
struct Model {
    value: i64,
    calls: usize,
    reject: bool,
    readonly: bool,
    disabled: bool,
    applied: usize,
}
impl Component for Model {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Value(v) => {
                self.calls += 1;
                if !self.reject {
                    self.value = v;
                }
            }
            Msg::Reject => self.reject = true,
            Msg::ReadOnly => self.readonly = true,
            Msg::Disable => self.disabled = true,
            Msg::External(v) => self.value = v,
            Msg::Ping => {}
            Msg::Apply => self.applied += 1,
        }
    }
    fn view(&self) -> View<Msg> {
        let mut o = O::new(-100, 100, 10);
        o.read_only = self.readonly;
        View::column(
            "root",
            vec![
                View::spinbox("s", "Quantity", self.value, o, Msg::Value).disabled(self.disabled),
                View::button("apply", "Apply", Msg::Apply),
            ],
        )
    }
}
fn app() -> Application<Model> {
    let mut a = Application::new(Model {
        value: 20,
        calls: 0,
        reject: false,
        readonly: false,
        disabled: false,
        applied: 0,
    })
    .unwrap();
    a.key(Key::plain('\t')).unwrap();
    a
}
fn draft(a: &Application<Model>) -> &str {
    a.view().children()[0].text_editor().unwrap().text()
}
fn type_text(a: &mut Application<Model>, text: &str) {
    a.edit(E::SelectAll).unwrap();
    for c in text.chars() {
        a.key(Key::plain(c)).unwrap();
    }
}
fn renderer() -> Renderer {
    renderer_profile(include_str!("../../../assets/dwm/sdk/dark.dss"))
}
fn renderer_profile(profile: &str) -> Renderer {
    let theme = Theme::parse(
        profile,
        include_str!("../../../assets/dwm/sdk/base.dss"),
        "",
    )
    .unwrap();
    let font = Font::parse(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/DejaVuSans.ttf"
        ))
        .unwrap(),
    )
    .unwrap();
    Renderer::new(font, theme.cascade, theme.background)
}
#[test]
fn validates_ranges_and_reports_numeric_accessibility() {
    assert_eq!(
        View::<()>::spinbox("s", "Quantity", 3, O::new(0, 100, 10), |_| ()).validate(),
        Err(UiError::InvalidSpinbox)
    );
    assert_eq!(
        View::<()>::spinbox("s", "Quantity", 0, O::new(10, 0, 1), |_| ()).validate(),
        Err(UiError::InvalidSpinbox)
    );
    let a = app();
    let v = &a.view().children()[0];
    assert_eq!(v.accessibility_role(), Some("spinbutton"));
    assert_eq!(v.accessibility_label(), Some("Quantity"));
    assert_eq!(v.spinbox_range().unwrap().value, 20);
}
#[test]
fn draft_commit_clamp_snap_and_escape() {
    let mut a = app();
    type_text(&mut a, "35");
    assert_eq!(draft(&a), "35");
    assert_eq!(a.component().value, 20);
    a.key(Key::plain('\r')).unwrap();
    assert_eq!(a.component().value, 40);
    assert_eq!(draft(&a), "40");
    type_text(&mut a, "99999999999999999999");
    a.commit_spinbox().unwrap();
    assert_eq!(a.component().value, 100);
    type_text(&mut a, "-9999999999999999999");
    a.commit_spinbox().unwrap();
    assert_eq!(a.component().value, -100);
    type_text(&mut a, "50");
    a.key(Key::from_native(0x01, 0, 0).unwrap()).unwrap();
    assert_eq!(draft(&a), "-100");
    assert_eq!(a.component().calls, 3);
}
#[test]
fn incomplete_and_invalid_drafts_do_not_escape_or_dispatch() {
    let mut a = app();
    type_text(&mut a, "-");
    assert_eq!(draft(&a), "-");
    a.key(Key::plain('x')).unwrap();
    assert_eq!(draft(&a), "-");
    a.key(Key::plain('+')).unwrap();
    assert_eq!(draft(&a), "-");
    a.commit_spinbox().unwrap();
    assert_eq!(draft(&a), "20");
    assert_eq!(a.component().calls, 0);
    type_text(&mut a, "");
    a.edit(E::Backspace { word: false }).unwrap();
    a.commit_spinbox().unwrap();
    assert_eq!(draft(&a), "20");
}
#[test]
fn arrows_pages_and_endpoints_use_one_controlled_callback() {
    let mut a = app();
    type_text(&mut a, "35");
    a.edit(E::Up { extend: false }).unwrap();
    assert_eq!(a.component().value, 50);
    assert_eq!(a.component().calls, 1);
    a.edit(E::Down { extend: false }).unwrap();
    assert_eq!(a.component().value, 40);
    a.spinbox_step(S::PageUp).unwrap();
    assert_eq!(a.component().value, 100);
    a.edit(E::Up { extend: false }).unwrap();
    assert_eq!(a.component().calls, 3);
    a.spinbox_step(S::PageDown).unwrap();
    assert_eq!(a.component().value, 0);
}
#[test]
fn reconciliation_preserves_draft_but_external_values_replace_it() {
    let mut a = app();
    type_text(&mut a, "65");
    a.update(Msg::Ping).unwrap();
    assert_eq!(draft(&a), "65");
    a.edit(E::Undo).unwrap();
    assert_eq!(draft(&a), "6");
    a.edit(E::Redo).unwrap();
    assert_eq!(draft(&a), "65");
    a.update(Msg::External(80)).unwrap();
    assert_eq!(draft(&a), "80");
    a.edit(E::Undo).unwrap();
    assert_eq!(draft(&a), "80");
    a.update(Msg::Reject).unwrap();
    type_text(&mut a, "-10");
    a.commit_spinbox().unwrap();
    assert_eq!(a.component().value, 80);
    assert_eq!(draft(&a), "80");
}
#[test]
fn tab_and_pointer_blur_commit_before_other_controls_activate() {
    let mut a = app();
    type_text(&mut a, "50");
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.component().value, 50);
    assert_eq!(a.focused_key(), Some("apply"));
    a.key(Key::plain(' ')).unwrap();
    assert_eq!(a.component().applied, 1);
    a.key(Key {
        character: '\t',
        modifiers: 1,
    })
    .unwrap();
    type_text(&mut a, "70");
    a.pointer(P::Down, Some("apply")).unwrap();
    assert_eq!(a.component().value, 70);
    a.pointer(P::Up, Some("apply")).unwrap();
    assert_eq!(a.component().applied, 2);
}
#[test]
fn readonly_and_disabled_never_change_values() {
    let mut a = app();
    type_text(&mut a, "50");
    a.update(Msg::ReadOnly).unwrap();
    assert_eq!(draft(&a), "20");
    type_text(&mut a, "80");
    a.spinbox_step(S::Increase).unwrap();
    assert_eq!(draft(&a), "20");
    assert_eq!(a.component().calls, 0);
    assert!(!a
        .spinbox_pointer(P::Down, Some("s"), Some(B::Increase))
        .unwrap());
    a.update(Msg::Disable).unwrap();
    assert_eq!(a.focused_key(), None);
    assert!(!a.edit(E::Insert('4')).unwrap());
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("apply"));
}
#[test]
fn buttons_activate_on_release_and_leave_escape_or_tab_cancel() {
    let mut a = app();
    a.spinbox_pointer(P::Down, Some("s"), Some(B::Increase))
        .unwrap();
    assert_eq!(a.component().calls, 0);
    assert_eq!(a.pressed_spinbox_key(), Some("s"));
    a.spinbox_pointer(P::Up, Some("s"), Some(B::Increase))
        .unwrap();
    assert_eq!(a.component().value, 30);
    assert_eq!(a.component().calls, 1);
    a.spinbox_pointer(P::Down, Some("s"), Some(B::Decrease))
        .unwrap();
    a.spinbox_pointer(P::Up, Some("s"), Some(B::Decrease))
        .unwrap();
    assert_eq!(a.component().value, 20);
    assert_eq!(a.component().calls, 2);
    for cancel in 0..4 {
        a.pointer(P::Down, Some("s")).unwrap();
        a.pointer(P::Up, Some("s")).unwrap();
        a.spinbox_pointer(P::Down, Some("s"), Some(B::Decrease))
            .unwrap();
        match cancel {
            0 => {
                a.spinbox_pointer(P::Move, Some("s"), None).unwrap();
            }
            1 => {
                a.spinbox_pointer(P::Leave, None, None).unwrap();
            }
            2 => {
                a.key(Key::from_native(0x01, 0, 0).unwrap()).unwrap();
            }
            _ => {
                a.key(Key::plain('\t')).unwrap();
            }
        }
        a.spinbox_pointer(P::Up, Some("s"), Some(B::Decrease))
            .unwrap();
        assert_eq!(a.component().value, 20);
        assert_eq!(a.pressed_spinbox_key(), None);
    }
}
#[test]
fn painting_hit_testing_damage_and_resize_share_geometry() {
    let mut a = app();
    let mut r = renderer();
    r.render(a.view(), 300, 160).unwrap();
    let before = r.pixels().to_vec();
    // Search through the public hit API; the result must lie on the same node.
    let find = |r: &Renderer, b| {
        (0..80)
            .flat_map(|y| (0..500).map(move |x| (x, y)))
            .find(|&(x, y)| r.spinbox_button_at("s", x as f32, y as f32) == Some(b))
            .unwrap()
    };
    let (x, y) = find(&r, B::Increase);
    assert_eq!(r.hit_key(x as f32, y as f32), Some("s"));
    a.spinbox_pointer(P::Down, Some("s"), Some(B::Increase))
        .unwrap();
    assert!(r.render(a.view(), 300, 160).unwrap().damage.is_some());
    assert_ne!(before, r.pixels());
    a.spinbox_pointer(P::Up, Some("s"), Some(B::Increase))
        .unwrap();
    type_text(&mut a, "45");
    r.render(a.view(), 300, 160).unwrap();
    let mut fresh = renderer();
    fresh.render(a.view(), 300, 160).unwrap();
    assert_eq!(fresh.pixels(), r.pixels());
    assert_eq!(r.render(a.view(), 300, 160).unwrap().damage, None);
    r.render(a.view(), 500, 160).unwrap();
    let (new_x, _) = find(&r, B::Increase);
    assert!(new_x > x);
    r.render(a.view(), 1, 1).unwrap();
    assert_eq!(r.spinbox_button_at("s", 0., 0.), None);
}

#[test]
fn both_profiles_paint_controls_and_reject_invalid_spinbox_overrides() {
    let a = app();
    let mut pixels = Vec::new();
    for profile in [
        include_str!("../../../assets/dwm/sdk/dark.dss"),
        include_str!("../../../assets/dwm/sdk/light.dss"),
    ] {
        let mut r = renderer_profile(profile);
        r.render(a.view(), 300, 160).unwrap();
        assert!((0..80)
            .any(|y| (0..300)
                .any(|x| r.spinbox_button_at("s", x as f32, y as f32) == Some(B::Increase))));
        pixels.push(r.pixels().to_vec());
        for overlay in [
            "Spinbox {spinbox-button-width: -1;}",
            "Spinbox {spinbox-mark-size: red;}",
            "Spinbox {spinbox-mark-width: 100;}",
            "Spinbox {spinbox-button-color: 10;}",
        ] {
            assert!(Theme::parse(
                profile,
                include_str!("../../../assets/dwm/sdk/base.dss"),
                overlay
            )
            .is_err());
        }
    }
    assert_ne!(pixels[0], pixels[1]);
}
