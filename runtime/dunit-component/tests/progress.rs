use dunit_component::{
    Application, Attrs, Component, Font, Key, PointerEvent as P, ProgressOptions as O,
    ProgressState, Renderer, Sizing, Theme, UiError, View,
};
use dunit_widgets::progress::ProgressPaint;
const DARK: &str = include_str!("../../../assets/dwm/sdk/dark.dss");
const LIGHT: &str = include_str!("../../../assets/dwm/sdk/light.dss");
const BASE: &str = include_str!("../../../assets/dwm/sdk/base.dss");
fn theme(profile: &str, overlay: &str) -> Theme {
    Theme::parse(profile, BASE, overlay).unwrap()
}
fn renderer(profile: &str, overlay: &str) -> Renderer {
    let t = theme(profile, overlay);
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        t.cascade,
        t.background,
    )
}
fn bar<M>(v: View<M>) -> View<M> {
    v.layout(Attrs {
        width: Sizing::Px(200.),
        height: Sizing::Px(32.),
        ..Default::default()
    })
}
fn scene(value: i64, running: bool) -> View<()> {
    View::column(
        "root",
        vec![
            bar(View::progress("p", "Download", value, O::new(0, 100))),
            bar(View::busy("b", "Working", running)),
        ],
    )
}
#[test]
fn ranges_extremes_and_metadata_are_validated_without_overflow() {
    for (value, o) in [
        (0, O::new(10, 0)),
        (-1, O::new(0, 100)),
        (101, O::new(0, 100)),
    ] {
        assert_eq!(
            View::<()>::progress("p", "Download", value, o).validate(),
            Err(UiError::InvalidProgress)
        );
    }
    let o = O::new(i64::MIN, i64::MAX);
    assert!(o.valid(i64::MAX));
    assert_eq!(o.fraction(i64::MIN), 0.);
    assert_eq!(o.fraction(i64::MAX), 1.);
    assert_eq!(o.fraction(0), 0.5);
    assert_eq!(O::new(7, 7).fraction(7), 1.);
    let v = View::<()>::progress("p", "Download", 50, O::new(0, 100));
    assert_eq!(v.accessibility_role(), Some("progressbar"));
    assert_eq!(v.accessibility_label(), Some("Download"));
    assert_eq!(v.accessibility_busy(), Some(false));
    assert_eq!(
        v.progress_state(),
        Some(ProgressState::Determinate {
            value: 50,
            options: O::new(0, 100)
        })
    );
    assert_eq!(
        View::<()>::busy("b", "Working", true).accessibility_busy(),
        Some(true)
    );
}
#[test]
fn passive_indicators_skip_focus_and_never_dispatch_or_capture() {
    struct Model(u32);
    impl Component for Model {
        type Message = ();
        fn update(&mut self, _: ()) {
            self.0 += 1;
        }
        fn view(&self) -> View<()> {
            View::column(
                "root",
                vec![
                    View::progress("p", "Download", 0, O::new(0, 100)),
                    View::busy("b", "Working", true),
                    View::busy("off", "Paused", false).disabled(true),
                    View::button("go", "Go", ()),
                ],
            )
        }
    }
    let mut a = Application::new(Model(0)).unwrap();
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("go"));
    a.pointer(P::Down, Some("b")).unwrap();
    a.pointer(P::Up, Some("b")).unwrap();
    assert_eq!(a.focused_key(), None);
    assert_eq!(a.component().0, 0);
    assert!(a.view().children()[..3]
        .iter()
        .all(|v| !v.interaction().pressed));
    a.key(Key::plain('\t')).unwrap();
    a.key(Key::plain(' ')).unwrap();
    assert_eq!(a.component().0, 1);
}
#[test]
fn determinate_geometry_pixels_and_damage_match_fresh_frames_in_both_themes() {
    let t = theme(DARK, "");
    let style = t
        .cascade
        .resolve(&dunit_component::style::cascade::NodeStyle::element(
            "Progress",
        ));
    let rect = dunit_component::Rect {
        x: 10.,
        y: 20.,
        w: 200.,
        h: 32.,
    };
    let (track, fill) = ProgressPaint::determinate(0.5).geometry(&style, rect);
    assert_eq!(fill.w, track.w / 2.);
    assert_eq!(fill.x, track.x);
    assert_eq!(fill.h, track.h);
    for profile in [DARK, LIGHT] {
        let mut r = renderer(profile, "");
        r.set_motion_enabled(false);
        let mut previous = Vec::new();
        for value in [0, 25, 50, 100] {
            let v = scene(value, false);
            let stats = r.render(&v, 300, 160).unwrap();
            assert!(stats.damage.is_some());
            let mut fresh = renderer(profile, "");
            fresh.render(&v, 300, 160).unwrap();
            assert_eq!(fresh.pixels(), r.pixels());
            assert_ne!(previous, r.pixels());
            previous = r.pixels().to_vec();
            assert_eq!(r.render(&v, 300, 160).unwrap().damage, None);
        }
        r.render(&scene(100, false), 500, 200).unwrap();
        let mut fresh = renderer(profile, "");
        fresh.render(&scene(100, false), 500, 200).unwrap();
        assert_eq!(fresh.pixels(), r.pixels());
        r.render(&scene(100, false), 1, 1).unwrap();
    }
}
#[test]
fn busy_clock_cycle_is_deterministic_and_only_repaints_its_rect() {
    let v = scene(50, true);
    let mut r = renderer(DARK, "");
    let first = r.render_at(&v, 300, 160, 0).unwrap();
    assert_eq!(first.active_busy_indicators, 1);
    assert_eq!(r.busy_interval_ms(), Some(40));
    let start = r.pixels().to_vec();
    let mid = r.render_at(&v, 300, 160, 300).unwrap();
    assert!(mid.damaged_pixels > 0 && mid.damaged_pixels < 300 * 160);
    assert_ne!(start, r.pixels());
    let mut fresh = renderer(DARK, "");
    fresh.render_at(&v, 300, 160, 300).unwrap();
    assert_eq!(fresh.pixels(), r.pixels());
    assert_eq!(r.render_at(&v, 300, 160, 300).unwrap().damage, None);
    r.render_at(&v, 300, 160, 1200).unwrap();
    assert_eq!(start, r.pixels());
    assert_eq!(
        ProgressPaint::busy(Some(0), 1200, 0.25, true),
        ProgressPaint {
            start: 0.,
            end: 0.25
        }
    );
    assert_eq!(
        ProgressPaint::busy(Some(600), 1200, 0.25, true),
        ProgressPaint {
            start: 0.75,
            end: 1.
        }
    );
    r.render_at(&v, 300, 160, u64::MAX).unwrap();
}
#[test]
fn pause_disable_reduced_motion_and_zero_timing_need_no_busy_timer() {
    for overlay in [
        "",
        "Busy {busy-duration: 0ms;}",
        "Busy {busy-interval: 0ms;}",
        "Busy {busy-segment: 0;}",
        "Busy {busy-segment: 1;}",
    ] {
        let mut r = renderer(DARK, overlay);
        let v = bar(View::<()>::busy("b", "Working", true));
        r.render(&v, 240, 80).unwrap();
        assert_eq!(r.busy_interval_ms(), None);
        r.set_motion_enabled(false);
        assert_eq!(
            r.render_at(&v, 240, 80, 0).unwrap().active_busy_indicators,
            0
        );
        let pixels = r.pixels().to_vec();
        assert_eq!(r.render_at(&v, 240, 80, 300).unwrap().damage, None);
        assert_eq!(pixels, r.pixels());
        r.set_motion_enabled(true);
        let expected = if overlay.is_empty() { 1 } else { 0 };
        assert_eq!(
            r.render_at(&v, 240, 80, 300)
                .unwrap()
                .active_busy_indicators,
            expected
        );
        r.render_at(&v.disabled(true), 240, 80, 600).unwrap();
        assert_eq!(r.busy_interval_ms(), None);
        r.render_at(&bar(View::<()>::busy("b", "Working", false)), 240, 80, 700)
            .unwrap();
        assert_eq!(r.busy_interval_ms(), None);
        assert_eq!(
            r.render_at(&bar(View::<()>::busy("b", "Working", false)), 240, 80, 900)
                .unwrap()
                .damage,
            None
        );
    }
}
#[test]
fn clipping_removal_and_multiple_busy_indicators_bound_scheduling() {
    let v = View::<()>::scroll(
        "region",
        "Work",
        vec![
            View::text("before", "").layout(Attrs {
                height: Sizing::Px(240.),
                ..Default::default()
            }),
            bar(View::busy("b", "Working", true)),
        ],
    );
    let mut r = renderer(DARK, "");
    r.render_at(&v, 300, 120, 0).unwrap();
    assert_eq!(r.busy_interval_ms(), None);
    assert!(r.scroll_by("region", 1000.));
    let s = r.render_at(&v, 300, 120, 300).unwrap();
    assert_eq!(s.active_busy_indicators, 1);
    assert_eq!(r.busy_interval_ms(), Some(40));
    let pair = View::<()>::column(
        "root",
        vec![
            bar(View::busy("fast", "Fast", true)),
            bar(View::busy("slow", "Slow", true)),
        ],
    );
    let mut r = renderer(
        DARK,
        "Busy#fast {busy-interval: 20ms;} Busy#slow {busy-interval: 80ms;}",
    );
    assert_eq!(
        r.render_at(&pair, 300, 120, 0)
            .unwrap()
            .active_busy_indicators,
        2
    );
    assert_eq!(r.busy_interval_ms(), Some(20));
    r.cancel_motion();
    assert_eq!(r.busy_interval_ms(), None);
    r.render_at(
        &View::<()>::column("root", vec![bar(View::busy("slow", "Slow", true))]),
        300,
        120,
        300,
    )
    .unwrap();
    assert_eq!(r.busy_interval_ms(), Some(80));
    r.render_at(&View::<()>::text("none", "Done"), 300, 120, 600)
        .unwrap();
    assert_eq!(r.busy_interval_ms(), None);
}
#[test]
fn theme_validation_and_reload_preserve_values_and_update_cadence() {
    for profile in [DARK, LIGHT] {
        for overlay in [
            "Progress {progress-fill-color: 1;}",
            "Progress {progress-thickness: -1;}",
            "Busy {busy-segment: 2;}",
            "Busy {busy-duration: -1ms;}",
            "Busy {busy-interval: red;}",
        ] {
            assert!(Theme::parse(profile, BASE, overlay).is_err());
        }
    }
    let mut r = renderer(DARK, "");
    let v = scene(50, true);
    r.render_at(&v, 300, 160, 300).unwrap();
    let dark = r.pixels().to_vec();
    r.set_theme(theme(LIGHT, "Busy {busy-interval: 80ms;}"), None);
    assert_eq!(r.busy_interval_ms(), None);
    r.render_at(&v, 300, 160, 300).unwrap();
    assert_eq!(r.busy_interval_ms(), Some(80));
    assert_ne!(dark, r.pixels());
    assert_eq!(
        v.children()[0].progress_state(),
        Some(ProgressState::Determinate {
            value: 50,
            options: O::new(0, 100)
        })
    );
}
