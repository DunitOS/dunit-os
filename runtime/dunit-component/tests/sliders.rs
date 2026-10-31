use dunit_component::{
    Application, Component, Font, Key, Navigation, PointerEvent as P, Renderer, SliderCommand as K,
    SliderOptions as O, Theme, UiError, View,
};
#[derive(Clone)]
enum Msg {
    Value(i64),
    Reject(bool),
    Disable,
    Range,
    Hide,
}
struct Model {
    value: i64,
    reject: bool,
    disabled: bool,
    changed: bool,
    hidden: bool,
    calls: usize,
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
            Msg::Reject(r) => self.reject = r,
            Msg::Disable => self.disabled = true,
            Msg::Range => self.changed = true,
            Msg::Hide => self.hidden = true,
        }
    }
    fn view(&self) -> View<Msg> {
        let mut children = vec![View::button("first", "First", Msg::Reject(false)).disabled(true)];
        if !self.hidden {
            children.push(
                View::slider(
                    "s",
                    "Volume",
                    self.value,
                    O::new(0, if self.changed { 200 } else { 100 }, 10),
                    Msg::Value,
                )
                .disabled(self.disabled),
            );
        }
        children.push(View::button("last", "Last", Msg::Reject(false)));
        View::column("root", children)
    }
}
fn app() -> Application<Model> {
    Application::new(Model {
        value: 50,
        reject: false,
        disabled: false,
        changed: false,
        hidden: false,
        calls: 0,
    })
    .unwrap()
}
fn renderer() -> Renderer {
    let theme = Theme::parse(
        include_str!("../../../assets/dwm/sdk/dark.dss"),
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
fn geometry(r: &mut Renderer, a: &Application<Model>) -> dunit_component::SliderGeometry {
    r.render(a.view(), 300, 160).unwrap();
    r.slider_geometry("s").unwrap()
}
#[test]
fn invalid_ranges_and_off_grid_values_fail_before_dispatch() {
    for (value, options) in [
        (0, O::new(1, 0, 1)),
        (0, O::new(0, 10, 0)),
        (0, O::new(0, 10, -1)),
        (101, O::new(0, 100, 1)),
        (3, O::new(0, 100, 10)),
        (
            0,
            O {
                min: 0,
                max: 100,
                step: 10,
                page_step: 15,
            },
        ),
    ] {
        assert_eq!(
            View::<()>::slider("s", "Range", value, options, |_| ()).validate(),
            Err(UiError::InvalidSlider)
        );
    }
    assert!(O::new(5, 5, 1).valid(5));
}
#[test]
fn integer_extremes_endpoint_and_pointer_mapping_do_not_overflow() {
    let r = O::new(i64::MIN, i64::MAX, 1);
    assert!(r.valid(i64::MIN));
    assert!(r.valid(i64::MAX));
    assert_eq!(r.command(i64::MIN, K::Decrease), i64::MIN);
    assert_eq!(r.command(i64::MAX, K::Increase), i64::MAX);
    assert_eq!(r.command(i64::MIN, K::Increase), i64::MIN + 1);
    assert_eq!(r.command(i64::MAX, K::Decrease), i64::MAX - 1);
    assert_eq!(r.at_fraction(0.), i64::MIN);
    assert_eq!(r.at_fraction(1.), i64::MAX);
    let r = O::new(-10, 10, 6);
    assert_eq!(r.command(10, K::Decrease), 8);
    assert_eq!(r.command(8, K::Increase), 10);
    let mut prev = r.min;
    for n in 0..=1000 {
        let v = r.at_fraction(n as f32 / 1000.);
        assert!(v >= prev && v <= r.max);
        assert!(r.valid(v));
        prev = v;
    }
}
#[test]
fn keyboard_focus_steps_pages_and_endpoints_are_controlled() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("s"));
    a.navigate(Navigation::Right).unwrap();
    assert_eq!(a.component().value, 60);
    a.navigate(Navigation::Down).unwrap();
    assert_eq!(a.component().value, 50);
    a.slider_command(K::Home).unwrap();
    assert_eq!(a.component().value, 0);
    let calls = a.component().calls;
    a.slider_command(K::Decrease).unwrap();
    assert_eq!(a.component().calls, calls);
    a.slider_command(K::PageUp).unwrap();
    assert_eq!(a.component().value, 100);
    a.slider_command(K::PageDown).unwrap();
    assert_eq!(a.component().value, 0);
    a.slider_command(K::End).unwrap();
    assert_eq!(a.component().value, 100);
    let node = &a.view().children()[1];
    assert_eq!(node.accessibility_role(), Some("slider"));
    assert_eq!(node.accessibility_label(), Some("Volume"));
    assert_eq!(node.slider_range().unwrap().value, 100);
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("last"));
    assert!(!a.slider_command(K::Home).unwrap());
}
#[test]
fn track_click_drag_outside_and_release_are_not_button_activation() {
    let mut a = app();
    let mut r = renderer();
    let g = geometry(&mut r, &a);
    a.slider_pointer(P::Down, Some("s"), Some(("s", g.track.x, g)))
        .unwrap();
    assert_eq!(a.component().value, 0);
    let g = geometry(&mut r, &a);
    a.slider_pointer(P::Move, None, Some(("s", g.track.x + g.track.w + 50., g)))
        .unwrap();
    assert_eq!(a.component().value, 100);
    assert_eq!(a.dragging_slider_key(), Some("s"));
    let calls = a.component().calls;
    a.slider_pointer(P::Up, None, Some(("s", g.track.x + g.track.w + 50., g)))
        .unwrap();
    assert_eq!(a.component().calls, calls);
    assert_eq!(a.dragging_slider_key(), None);
}
#[test]
fn thumb_grab_does_not_jump_and_rejected_changes_are_not_optimistic() {
    let mut a = app();
    let mut r = renderer();
    let g = geometry(&mut r, &a);
    let x = g.thumb.x + g.thumb.w / 2. + 3.;
    a.slider_pointer(P::Down, Some("s"), Some(("s", x, g)))
        .unwrap();
    assert_eq!(a.component().value, 50);
    assert_eq!(a.component().calls, 0);
    a.update(Msg::Reject(true)).unwrap();
    a.slider_pointer(P::Move, None, Some(("s", g.track.x + g.track.w + 3., g)))
        .unwrap();
    assert_eq!(a.component().value, 50);
    assert_eq!(a.view().children()[1].slider_range().unwrap().value, 50);
    a.update(Msg::Reject(false)).unwrap();
    a.slider_pointer(P::Move, None, Some(("s", g.track.x + g.track.w + 3., g)))
        .unwrap();
    assert_eq!(a.component().value, 100);
    // A fresh Down belongs to its new target, not an old slider capture.
    assert!(!a.slider_pointer(P::Down, Some("last"), None).unwrap());
    a.pointer(P::Down, Some("last")).unwrap();
    assert_eq!(a.dragging_slider_key(), None);
}
#[test]
fn cancellation_disable_range_and_removal_end_capture() {
    for action in 0..6 {
        let mut a = app();
        let mut r = renderer();
        let g = geometry(&mut r, &a);
        a.slider_pointer(
            P::Down,
            Some("s"),
            Some(("s", g.thumb.x + g.thumb.w / 2., g)),
        )
        .unwrap();
        match action {
            0 => {
                a.slider_pointer(P::Leave, None, None).unwrap();
            }
            1 => {
                a.key(Key::plain('\u{1b}')).unwrap();
            }
            2 => {
                a.update(Msg::Disable).unwrap();
            }
            3 => {
                a.update(Msg::Range).unwrap();
            }
            4 => {
                a.update(Msg::Hide).unwrap();
            }
            _ => {
                a.key(Key::plain('\t')).unwrap();
            }
        }
        assert_eq!(a.dragging_slider_key(), None);
    }
}
#[test]
fn pixels_damage_and_resize_match_a_fresh_renderer() {
    let mut a = app();
    let mut r = renderer();
    let initial = r.render(a.view(), 300, 160).unwrap();
    assert!(initial.damage.is_some());
    let before = r.pixels().to_vec();
    a.key(Key::plain('\t')).unwrap();
    a.slider_command(K::End).unwrap();
    let changed = r.render(a.view(), 300, 160).unwrap();
    assert!(changed.damage.is_some());
    assert_ne!(before, r.pixels());
    let mut fresh = renderer();
    fresh.render(a.view(), 300, 160).unwrap();
    assert_eq!(fresh.pixels(), r.pixels());
    assert_eq!(r.render(a.view(), 300, 160).unwrap().damage, None);
    let old = r.slider_geometry("s").unwrap();
    r.render(a.view(), 500, 160).unwrap();
    let new = r.slider_geometry("s").unwrap();
    assert!(new.track.w > old.track.w);
    r.render(a.view(), 1, 1).unwrap();
    let tiny = r.slider_geometry("s").unwrap();
    assert_eq!(tiny.fraction(0., 0.), None);
}

#[test]
fn scrolled_slider_uses_the_same_geometry_for_painting_and_hit_testing() {
    let view = View::<()>::scroll(
        "region",
        "Controls",
        vec![
            View::text("before", "Before").layout(dunit_component::Attrs {
                height: dunit_component::Sizing::Px(160.),
                ..Default::default()
            }),
            View::slider("s", "Volume", 50, O::new(0, 100, 10), |_| ()).layout(
                dunit_component::Attrs {
                    height: dunit_component::Sizing::Px(48.),
                    ..Default::default()
                },
            ),
        ],
    );
    let mut r = renderer();
    r.render(&view, 200, 100).unwrap();
    assert!(r.scroll_by("region", 5000.));
    r.render(&view, 200, 100).unwrap();
    let g = r.slider_geometry("s").unwrap();
    assert_eq!(
        r.hit_key(g.thumb.x + g.thumb.w / 2., g.thumb.y + g.thumb.h / 2.),
        Some("s")
    );
}
