use dunit_component::{
    Application, Attrs, Component, Font, Key, Navigation, PointerEvent as P, Renderer, Sizing,
    SliderCommand as K, SliderGeometry, SliderOptions as O, SliderOrientation as Axis, Theme, View,
};
#[derive(Clone)]
enum Msg {
    Value(i64),
    Reject,
    Disable,
    Range,
    Horizontal,
    Hide,
}
struct Model {
    value: i64,
    calls: usize,
    reject: bool,
    disabled: bool,
    range: bool,
    horizontal: bool,
    hidden: bool,
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
            Msg::Disable => self.disabled = true,
            Msg::Range => self.range = true,
            Msg::Horizontal => self.horizontal = true,
            Msg::Hide => self.hidden = true,
        }
    }
    fn view(&self) -> View<Msg> {
        let mut children = vec![];
        if !self.hidden {
            let create = if self.horizontal {
                View::slider
            } else {
                View::vertical_slider
            };
            children.push(
                create(
                    "v",
                    "Level",
                    self.value,
                    O::new(0, if self.range { 200 } else { 100 }, 10),
                    Msg::Value as fn(i64) -> Msg,
                )
                .layout(Attrs {
                    width: Sizing::Px(48.),
                    height: Sizing::Grow(1.),
                    ..Default::default()
                })
                .disabled(self.disabled),
            );
        }
        children.push(
            View::vertical_slider("locked", "Disabled", 20, O::new(0, 100, 10), Msg::Value)
                .disabled(true),
        );
        children.push(View::button("last", "Last", Msg::Reject));
        View::row("root", children)
    }
}
fn app() -> Application<Model> {
    Application::new(Model {
        value: 50,
        calls: 0,
        reject: false,
        disabled: false,
        range: false,
        horizontal: false,
        hidden: false,
    })
    .unwrap()
}
fn theme(profile: &str, overlay: &str) -> Theme {
    Theme::parse(
        profile,
        include_str!("../../../assets/dwm/sdk/base.dss"),
        overlay,
    )
    .unwrap()
}
fn renderer(profile: &str) -> Renderer {
    let t = theme(profile, "");
    let font = Font::parse(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/DejaVuSans.ttf"
        ))
        .unwrap(),
    )
    .unwrap();
    Renderer::new(font, t.cascade, t.background)
}
const DARK: &str = include_str!("../../../assets/dwm/sdk/dark.dss");
fn geometry(r: &mut Renderer, a: &Application<Model>) -> SliderGeometry {
    r.render(a.view(), 300, 320).unwrap();
    r.slider_geometry("v").unwrap()
}
#[test]
fn vertical_geometry_maps_upward_and_preserves_asymmetric_padding() {
    let t = theme(DARK, "VerticalSlider {padding: 7 11 9 13;}");
    let style = t
        .cascade
        .resolve(&dunit_component::style::cascade::NodeStyle::element(
            "VerticalSlider",
        ));
    let g = SliderGeometry::oriented(
        dunit_component::Rect {
            x: 10.,
            y: 20.,
            w: 80.,
            h: 200.,
        },
        &style,
        0.5,
        Axis::Vertical,
    );
    assert_eq!(g.orientation, Axis::Vertical);
    assert_eq!(g.track.y, 20. + 1. + 7. + 9.);
    assert_eq!(g.track.h, 200. - 2. - 7. - 9. - 18.);
    assert_eq!(g.track.x, 10. + 1. + 13. + (80. - 2. - 13. - 11. - 4.) / 2.);
    assert_eq!(g.fraction(g.track.y, 0.), Some(1.));
    assert_eq!(g.fraction(g.track.y + g.track.h, 0.), Some(0.));
    assert_eq!(g.fraction(g.track.y - 100., 0.), Some(1.));
    assert_eq!(g.fraction(f32::NAN, 0.), None);
    assert_eq!(g.coordinate(900., 123.), 123.);
    assert_eq!(g.fill(0.).h, 0.);
    assert_eq!(g.fill(1.), g.track);
    assert_eq!(g.fill(0.5).y, g.track.y + g.track.h / 2.);
    let tiny = SliderGeometry::oriented(
        dunit_component::Rect {
            x: 0.,
            y: 0.,
            w: 1.,
            h: 1.,
        },
        &style,
        0.5,
        Axis::Vertical,
    );
    assert_eq!(tiny.fraction(0., 0.), None);
}
#[test]
fn keyboard_focus_and_accessibility_match_vertical_value_direction() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("v"));
    a.navigate(Navigation::Up).unwrap();
    assert_eq!(a.component().value, 60);
    a.navigate(Navigation::Down).unwrap();
    assert_eq!(a.component().value, 50);
    a.slider_command(K::Home).unwrap();
    assert_eq!(a.component().value, 0);
    a.slider_command(K::PageUp).unwrap();
    assert_eq!(a.component().value, 100);
    let calls = a.component().calls;
    a.slider_command(K::End).unwrap();
    assert_eq!(a.component().calls, calls);
    let v = &a.view().children()[0];
    assert_eq!(v.accessibility_role(), Some("slider"));
    assert_eq!(v.accessibility_label(), Some("Level"));
    assert_eq!(v.slider_range().unwrap().orientation, Axis::Vertical);
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("last"));
}
#[test]
fn pointer_grab_drag_clamp_and_model_rejection_are_controlled() {
    let mut a = app();
    let mut r = renderer(DARK);
    let g = geometry(&mut r, &a);
    let y = g.thumb.y + g.thumb.h / 2. + 4.;
    a.slider_pointer(P::Down, Some("v"), Some(("v", y, g)))
        .unwrap();
    assert_eq!(a.component().value, 50);
    assert_eq!(a.component().calls, 0);
    a.slider_pointer(P::Move, None, Some(("v", g.track.y + 4., g)))
        .unwrap();
    assert_eq!(a.component().value, 100);
    let g = geometry(&mut r, &a);
    a.update(Msg::Reject).unwrap();
    a.slider_pointer(P::Move, None, Some(("v", g.track.y + g.track.h + 100., g)))
        .unwrap();
    assert_eq!(a.component().value, 100);
    a.slider_pointer(P::Up, None, Some(("v", g.track.y + g.track.h + 100., g)))
        .unwrap();
    assert_eq!(a.dragging_slider_key(), None);
}
#[test]
fn escape_focus_range_orientation_disable_removal_and_leave_cancel_capture() {
    for cancel in 0..7 {
        let mut a = app();
        let mut r = renderer(DARK);
        let g = geometry(&mut r, &a);
        a.slider_pointer(
            P::Down,
            Some("v"),
            Some(("v", g.thumb.y + g.thumb.h / 2., g)),
        )
        .unwrap();
        match cancel {
            0 => {
                a.key(Key::from_native(1, 0, 0).unwrap()).unwrap();
            }
            1 => {
                a.key(Key::plain('\t')).unwrap();
            }
            2 => a.update(Msg::Range).unwrap(),
            3 => a.update(Msg::Horizontal).unwrap(),
            4 => a.update(Msg::Disable).unwrap(),
            5 => a.update(Msg::Hide).unwrap(),
            _ => {
                a.slider_pointer(P::Leave, None, None).unwrap();
            }
        }
        assert_eq!(a.dragging_slider_key(), None);
        assert_eq!(a.component().value, 50);
    }
}
#[test]
fn both_themes_damage_resize_and_intrinsic_orientation_match_fresh_frames() {
    for profile in [DARK, include_str!("../../../assets/dwm/sdk/light.dss")] {
        let mut a = app();
        let mut r = renderer(profile);
        let g = geometry(&mut r, &a);
        assert!(g.track.h > g.track.w);
        assert_eq!(
            r.hit_key(g.thumb.x + g.thumb.w / 2., g.thumb.y + g.thumb.h / 2.),
            Some("v")
        );
        let before = r.pixels().to_vec();
        a.key(Key::plain('\t')).unwrap();
        a.slider_command(K::End).unwrap();
        assert!(r.render(a.view(), 300, 320).unwrap().damage.is_some());
        assert_ne!(before, r.pixels());
        let mut fresh = renderer(profile);
        fresh.render(a.view(), 300, 320).unwrap();
        assert_eq!(fresh.pixels(), r.pixels());
        assert_eq!(r.render(a.view(), 300, 320).unwrap().damage, None);
        r.render(a.view(), 300, 500).unwrap();
        let new = r.slider_geometry("v").unwrap();
        assert!(new.track.h > g.track.h);
        r.render(a.view(), 1, 1).unwrap();
        assert_eq!(r.slider_geometry("v").unwrap().fraction(0., 0.), None);
        let view = View::<()>::vertical_slider("v", "Level", 50, O::new(0, 100, 10), |_| ());
        r.render(&view, 300, 320).unwrap();
        let g = r.slider_geometry("v").unwrap();
        assert!(g.track.h > g.track.w);
    }
}
#[test]
fn scroll_translation_and_clip_apply_to_vertical_slider_hits() {
    let view = View::<()>::scroll(
        "region",
        "Controls",
        vec![
            View::text("before", "Before").layout(Attrs {
                height: Sizing::Px(200.),
                ..Default::default()
            }),
            View::vertical_slider("v", "Level", 50, O::new(0, 100, 10), |_| ()).layout(Attrs {
                width: Sizing::Px(48.),
                height: Sizing::Px(180.),
                ..Default::default()
            }),
        ],
    );
    let mut r = renderer(DARK);
    r.render(&view, 200, 160).unwrap();
    assert!(r.scroll_by("region", 5000.));
    r.render(&view, 200, 160).unwrap();
    let g = r.slider_geometry("v").unwrap();
    assert_eq!(
        r.hit_key(g.thumb.x + g.thumb.w / 2., g.thumb.y + g.thumb.h / 2.),
        Some("v")
    );
    assert_ne!(r.hit_key(g.track.x, -1.), Some("v"));
}
