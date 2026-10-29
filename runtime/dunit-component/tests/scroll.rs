//! Independent retained geometry, pixel and event evidence for Scroll.
use dunit_component::{
    style, Application, Attrs, Cascade, Color, Component, Font, Key, PointerEvent as P, Renderer,
    ScrollCommand, Sizing, View,
};

fn fixed<M>(view: View<M>, height: f32) -> View<M> {
    view.layout(Attrs {
        height: Sizing::Px(height),
        ..Attrs::default()
    })
}
fn rows(prefix: &str, count: usize) -> Vec<View<()>> {
    (0..count)
        .map(|n| {
            fixed(
                View::button(format!("{prefix}{n}"), "", ()).class(if n % 2 == 0 {
                    "even"
                } else {
                    "odd"
                }),
                20.,
            )
        })
        .collect()
}
fn view(count: usize) -> View<()> {
    View::scroll("scroll", "Rows", rows("row", count))
}
fn cascade(step: f32, width: f32) -> Cascade {
    let mut cascade = Cascade::new();
    cascade.push(style::parse(&format!("Column, Row, Stack {{ padding: 0; }} Scroll {{ padding: 0; background: #102030; scrollbar-width: {width}; scrollbar-min-thumb: 12; scrollbar-radius: 0; scrollbar-track: #304050; scrollbar-thumb: #b0c0d0; scroll-step: {step}; }} Button.even {{ background: #224466; }} Button.odd {{ background: #aa4422; }} Text {{ background: #778899; }}")).unwrap());
    cascade
}
fn renderer() -> Renderer {
    let font = Font::parse(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/DejaVuSans.ttf"
        ))
        .unwrap(),
    )
    .unwrap();
    Renderer::new(font, cascade(20., 8.), Color::rgb(0, 0, 0))
}
fn paint(r: &mut Renderer, v: &View<()>) {
    r.render(v, 100, 80).unwrap();
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.01, "{a} != {b}");
}

#[test]
fn thirty_rows_keep_natural_extent_and_wheel_updates_pixels_and_hit_target() {
    let v = view(30);
    let mut r = renderer();
    paint(&mut r, &v);
    let vp = r.scroll_viewport("scroll").unwrap();
    assert_eq!((vp.w, vp.h), (92., 80.));
    assert_eq!(r.hit_key(10., 10.), Some("row0"));
    assert_eq!(r.pixels()[10 * 100 + 10], 0xff224466);
    assert!(r.scroll_wheel(Some("row0"), 1));
    paint(&mut r, &v);
    assert_eq!(r.scroll_offset("scroll"), Some(20.));
    assert_eq!(r.hit_key(10., 10.), Some("row1"));
    assert_eq!(r.pixels()[10 * 100 + 10], 0xffaa4422);
    assert!(r.scroll_by("scroll", 10000.));
    paint(&mut r, &v);
    assert_eq!(r.scroll_offset("scroll"), Some(520.));
    assert_eq!(r.hit_key(10., 70.), Some("row29"));
}

#[test]
fn incremental_scroll_matches_independent_fresh_renderer_at_same_offset() {
    let v = view(30);
    let mut incremental = renderer();
    paint(&mut incremental, &v);
    for delta in [13., 47., -11., 34.] {
        assert!(incremental.scroll_by("scroll", delta));
        paint(&mut incremental, &v);
        let offset = incremental.scroll_offset("scroll").unwrap();
        let mut fresh = renderer();
        paint(&mut fresh, &v);
        assert!(fresh.scroll_by("scroll", offset));
        paint(&mut fresh, &v);
        assert_eq!(incremental.pixels(), fresh.pixels());
        assert_eq!(
            incremental.scroll_thumb("scroll"),
            fresh.scroll_thumb("scroll")
        );
        for y in [1., 19., 39., 79.] {
            assert_eq!(incremental.hit_key(10., y), fresh.hit_key(10., y));
        }
    }
}

#[test]
fn nested_scroll_clips_hits_and_bubbles_wheel_at_inner_boundary() {
    let v = View::scroll(
        "outer",
        "Outer",
        vec![
            fixed(View::text("before", ""), 40.),
            fixed(View::scroll("inner", "Inner", rows("inner-row", 30)), 60.),
            fixed(View::text("after", ""), 200.),
        ],
    );
    let mut r = renderer();
    r.render(&v, 100, 100).unwrap();
    assert!(r.scroll_wheel(Some("inner-row0"), 1));
    r.render(&v, 100, 100).unwrap();
    assert_eq!(r.scroll_offset("inner"), Some(20.));
    assert_eq!(r.scroll_offset("outer"), Some(0.));
    assert_eq!(r.hit_key(10., 50.), Some("inner-row1"));
    assert!(r.scroll_by("outer", 50.));
    r.render(&v, 100, 100).unwrap();
    assert_eq!(r.scroll_viewport("inner").unwrap().y, -10.);
    assert_eq!(r.hit_key(10., 10.), Some("inner-row2"));
    assert_eq!(
        r.hit_key(10., 60.),
        Some("after"),
        "inner content cannot hit through viewport bottom"
    );
    assert_eq!(
        r.pixels()[60 * 100 + 10],
        0xff778899,
        "inner paint is clipped too"
    );
    assert!(r.scroll_by("inner", 10000.));
    r.render(&v, 100, 100).unwrap();
    let inner_end = r.scroll_offset("inner");
    assert!(r.scroll_wheel(Some("inner-row29"), 1));
    r.render(&v, 100, 100).unwrap();
    assert_eq!(r.scroll_offset("inner"), inner_end);
    assert_eq!(r.scroll_offset("outer"), Some(70.));
}

struct Model;
impl Component for Model {
    type Message = ();
    fn update(&mut self, _: ()) {}
    fn view(&self) -> View<()> {
        view(30)
    }
}
#[test]
fn tab_reveals_offscreen_child_and_scroll_keys_reach_boundaries() {
    let mut app = Application::new(Model).unwrap();
    let mut r = renderer();
    paint(&mut r, app.view());
    app.key(Key::plain('\t')).unwrap();
    paint(&mut r, app.view());
    assert!(r.scroll_key(app.view(), ScrollCommand::End));
    paint(&mut r, app.view());
    assert_eq!(r.scroll_offset("scroll"), Some(520.));
    assert!(r.scroll_key(app.view(), ScrollCommand::Home));
    paint(&mut r, app.view());
    assert_eq!(r.scroll_offset("scroll"), Some(0.));
    for _ in 0..30 {
        app.key(Key::plain('\t')).unwrap();
        paint(&mut r, app.view());
    }
    assert_eq!(r.scroll_offset("scroll"), Some(520.));
    assert_eq!(r.hit_key(10., 70.), Some("row29"));
}

#[test]
fn shrink_disable_removal_and_retype_reset_retained_offset() {
    let mut r = renderer();
    let long = view(30);
    paint(&mut r, &long);
    r.scroll_by("scroll", 400.);
    paint(&mut r, &long);
    paint(&mut r, &view(2));
    assert_eq!(r.scroll_offset("scroll"), Some(0.));
    assert!(r.scroll_thumb("scroll").is_none());
    paint(&mut r, &long);
    r.scroll_by("scroll", 100.);
    paint(&mut r, &long);
    paint(&mut r, &view(30).disabled(true));
    assert_eq!(r.scroll_offset("scroll"), None);
    paint(&mut r, &long);
    assert_eq!(r.scroll_offset("scroll"), Some(0.));
    r.scroll_by("scroll", 100.);
    paint(&mut r, &long);
    paint(&mut r, &View::column("empty", vec![]));
    assert_eq!(r.scroll_offset("scroll"), None);
    paint(&mut r, &long);
    assert_eq!(r.scroll_offset("scroll"), Some(0.));
    r.scroll_by("scroll", 100.);
    paint(&mut r, &long);
    paint(&mut r, &View::text("scroll", "replacement"));
    assert_eq!(r.scroll_offset("scroll"), None);
    paint(&mut r, &long);
    assert_eq!(r.scroll_offset("scroll"), Some(0.));
}

#[test]
fn keyed_reorder_retains_its_offset_and_resize_clamps_geometry() {
    let make = |reversed| {
        let a = View::scroll("a", "A", rows("a-row", 30));
        let b = View::scroll("b", "B", rows("b-row", 30));
        View::row("root", if reversed { vec![b, a] } else { vec![a, b] })
    };
    let mut r = renderer();
    paint(&mut r, &make(false));
    assert!(r.scroll_by("a", 200.));
    paint(&mut r, &make(false));
    paint(&mut r, &make(true));
    assert_eq!(r.scroll_offset("a"), Some(200.));
    assert_eq!(r.scroll_offset("b"), Some(0.));
    let v = view(30);
    paint(&mut r, &v);
    r.scroll_by("scroll", 10000.);
    paint(&mut r, &v);
    r.render(&v, 100, 200).unwrap();
    assert_eq!(r.scroll_offset("scroll"), Some(400.));
    assert_eq!(r.scroll_viewport("scroll").unwrap().h, 200.);
    r.render(&v, 100, 700).unwrap();
    assert_eq!(r.scroll_offset("scroll"), Some(0.));
    assert!(r.scroll_thumb("scroll").is_none());
}

#[test]
fn scrollbar_drag_keeps_grab_anchor_leave_cancels_and_track_pages() {
    let v = view(30);
    let mut r = renderer();
    paint(&mut r, &v);
    let track = r.scroll_track("scroll").unwrap();
    let thumb = r.scroll_thumb("scroll").unwrap();
    let x = thumb.x + thumb.w / 2.;
    let grab_y = thumb.y + 3.;
    assert_eq!(
        r.scroll_pointer(P::Down, x, grab_y).as_deref(),
        Some("scroll")
    );
    r.scroll_pointer(P::Move, x, grab_y);
    paint(&mut r, &v);
    assert_eq!(r.scroll_offset("scroll"), Some(0.));
    r.scroll_pointer(P::Move, x, grab_y + 20.);
    paint(&mut r, &v);
    let moved = r.scroll_offset("scroll").unwrap();
    assert!(moved > 0.);
    close(r.scroll_thumb("scroll").unwrap().y, thumb.y + 20.);
    r.scroll_pointer(P::Move, x, grab_y + 20.);
    paint(&mut r, &v);
    close(r.scroll_offset("scroll").unwrap(), moved);
    assert_eq!(
        r.scroll_pointer(P::Leave, -100., -100.).as_deref(),
        Some("scroll")
    );
    assert!(r.scroll_pointer(P::Move, -100., -100.).is_none());
    paint(&mut r, &v);
    close(r.scroll_offset("scroll").unwrap(), moved);
    r.scroll_by("scroll", -10000.);
    paint(&mut r, &v);
    let thumb = r.scroll_thumb("scroll").unwrap();
    r.scroll_pointer(P::Down, track.x + 1., thumb.y + thumb.h + 2.);
    paint(&mut r, &v);
    assert_eq!(r.scroll_offset("scroll"), Some(80.));
}

#[test]
fn theme_controls_step_and_gutter_and_noop_has_zero_damage() {
    let v = view(30);
    let mut r = renderer();
    paint(&mut r, &v);
    assert!(r.render(&v, 100, 80).unwrap().damage.is_none());
    r.set_style(cascade(35., 16.), Color::rgb(0, 0, 0));
    paint(&mut r, &v);
    assert_eq!(r.scroll_viewport("scroll").unwrap().w, 84.);
    assert_eq!(r.scroll_track("scroll").unwrap().w, 16.);
    assert!(r.scroll_wheel(Some("row0"), 1));
    paint(&mut r, &v);
    assert_eq!(r.scroll_offset("scroll"), Some(35.));
    assert!(r.render(&v, 100, 80).unwrap().damage.is_none());
    r.set_style(cascade(0., 0.), Color::rgb(0, 0, 0));
    paint(&mut r, &v);
    let offset = r.scroll_offset("scroll");
    assert_eq!(r.scroll_viewport("scroll").unwrap().w, 100.);
    assert!(r.scroll_thumb("scroll").is_none());
    assert!(!r.scroll_wheel(Some("row0"), 1));
    paint(&mut r, &v);
    assert_eq!(r.scroll_offset("scroll"), offset);
}

#[test]
fn default_scroll_grow_uses_remaining_column_height_before_fixed_footer() {
    let v = View::column("root", vec![view(30), fixed(View::text("footer", ""), 20.)]);
    let mut r = renderer();
    paint(&mut r, &v);
    assert_eq!(r.scroll_viewport("scroll").unwrap().h, 60.);
    assert_eq!(r.hit_key(10., 70.), Some("footer"));
    assert!(r.scroll_thumb("scroll").is_some());
    assert!(r.scroll_by("scroll", 10000.));
    paint(&mut r, &v);
    assert_eq!(r.scroll_offset("scroll"), Some(540.));
    assert_eq!(r.pixels()[70 * 100 + 10], 0xff778899);
}

#[test]
fn theme_geometry_reveals_focus_but_color_only_reload_preserves_manual_scroll() {
    let mut app = Application::new(Model).unwrap();
    let mut r = renderer();
    paint(&mut r, app.view());
    for _ in 0..5 {
        app.key(Key::plain('\t')).unwrap();
        paint(&mut r, app.view());
    }
    assert_eq!(app.focused_key(), Some("row3"));
    r.set_style(cascade(20., 8.), Color::rgb(1, 2, 3));
    r.scroll_by("scroll", 10.);
    paint(&mut r, app.view());
    assert_eq!(r.scroll_offset("scroll"), Some(10.));
    let mut css = cascade(20., 8.);
    css.push(style::parse("Scroll { padding: 30 0; }").unwrap());
    r.set_style(css, Color::rgb(0, 0, 0));
    paint(&mut r, app.view());
    assert_eq!(r.scroll_viewport("scroll").unwrap().h, 20.);
    assert_eq!(r.hit_key(10., 40.), Some("row3"));
}
#[test]
fn uncaptured_scrollbar_move_and_up_allow_text_selection_capture_to_continue() {
    let v = view(30);
    let mut r = renderer();
    paint(&mut r, &v);
    let t = r.scroll_track("scroll").unwrap();
    assert_eq!(r.scroll_pointer(P::Move, t.x + 1., t.y + 1.), None);
    assert_eq!(r.scroll_pointer(P::Up, t.x + 1., t.y + 1.), None);
    let thumb = r.scroll_thumb("scroll").unwrap();
    assert!(r.scroll_pointer(P::Down, t.x + 1., thumb.y + 1.).is_some());
    paint(&mut r, &view(1));
    assert_eq!(r.scroll_pointer(P::Move, 10., 200.), None);
}
#[test]
fn shared_theme_rejects_invalid_scrollbar_policy_values() {
    let base = include_str!("../../../assets/dwm/sdk/base.dss");
    let profile = include_str!("../../../assets/dwm/sdk/dark.dss");
    for overlay in [
        "Scroll { scrollbar-width: -1; }",
        "Scroll { scrollbar-min-thumb: 1025; }",
        "Scroll { scrollbar-thumb: 3; }",
    ] {
        assert!(dunit_component::Theme::parse(profile, base, overlay).is_err());
    }
}
