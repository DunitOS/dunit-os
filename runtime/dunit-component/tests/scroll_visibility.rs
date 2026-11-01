use dunit_component::{
    style, Attrs, Cascade, Color, Font, PointerEvent as P, Renderer, ScrollAxes, Sizing, View,
};
fn renderer(delay: u64) -> Renderer {
    let mut cascade = Cascade::new();
    cascade.push(style::parse(&format!("Scroll {{ padding:0; background:#102030; scrollbar-width:8; scrollbar-min-thumb:12; scrollbar-track:#304050; scrollbar-thumb:#b0c0d0; scroll-step:20; scrollbar-hide-delay:{delay}ms; }} Button {{ background:#224466; }}")).unwrap());
    Renderer::new(
        Font::parse(
            std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/fonts/DejaVuSans.ttf"
            ))
            .unwrap(),
        )
        .unwrap(),
        cascade,
        Color::rgb(0, 0, 0),
    )
}
fn view(count: usize) -> View<()> {
    View::scroll(
        "scroll",
        "Rows",
        (0..count)
            .map(|i| {
                View::button(format!("row{i}"), "", ()).layout(Attrs {
                    width: Sizing::Px(200.),
                    height: Sizing::Px(20.),
                    ..Default::default()
                })
            })
            .collect(),
    )
    .scroll_axes(ScrollAxes::Both)
}
#[test]
fn idle_hides_pixels_on_deadline_without_changing_geometry_or_hit_testing() {
    let (mut r, v) = (renderer(100), view(30));
    r.render_at(&v, 100, 80, 0).unwrap();
    let vp = r.scroll_viewport("scroll");
    let thumb = r.scroll_thumb("scroll");
    let hit = r.hit_key(10., 10.).map(String::from);
    let visible = r.pixels().to_vec();
    assert_eq!(r.scroll_hide_deadline_ms(), Some(100));
    assert_eq!(r.scrollbars_visible("scroll"), Some(true));
    assert!(r.render_at(&v, 100, 80, 99).unwrap().damage.is_none());
    assert!(r.render_at(&v, 100, 80, 100).unwrap().damage.is_some());
    assert_eq!(r.scrollbars_visible("scroll"), Some(false));
    assert_eq!(r.scroll_hide_deadline_ms(), None);
    assert_eq!(r.scroll_viewport("scroll"), vp);
    assert_eq!(r.scroll_thumb("scroll"), thumb);
    assert_eq!(r.hit_key(10., 10.), hit.as_deref());
    assert_ne!(r.pixels(), visible);
    assert!(r.render_at(&v, 100, 80, 200).unwrap().damage.is_none());
    assert!(r.scroll_activity_at(Some("row0"), 250));
    r.render_at(&v, 100, 80, 250).unwrap();
    assert_eq!(r.pixels(), visible);
    assert_eq!(r.scroll_hide_deadline_ms(), Some(350));
}
#[test]
fn activity_reschedules_idle_without_unnecessary_repaint_and_scroll_wakes() {
    let (mut r, v) = (renderer(100), view(30));
    r.render_at(&v, 100, 80, 0).unwrap();
    assert!(!r.scroll_activity_at(Some("row0"), 90));
    assert!(r.render_at(&v, 100, 80, 100).unwrap().damage.is_none());
    assert_eq!(r.scroll_hide_deadline_ms(), Some(190));
    r.render_at(&v, 100, 80, 190).unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), Some(false));
    assert!(r.scroll_wheel(Some("row0"), 1));
    r.render_at(&v, 100, 80, 300).unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), Some(true));
    assert_eq!(r.scroll_hide_deadline_ms(), Some(400));
    r.render_at(&v, 100, 80, 400).unwrap();
    assert!(r.scroll_by_x("scroll", 15.));
    r.render_at(&v, 100, 80, 500).unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), Some(true));
    assert_eq!(r.scroll_hide_deadline_ms(), Some(600));
}
#[test]
fn hidden_thumb_can_be_grabbed_capture_stays_visible_and_release_starts_new_deadline() {
    let (mut r, v) = (renderer(100), view(30));
    r.render_at(&v, 100, 80, 0).unwrap();
    r.render_at(&v, 100, 80, 100).unwrap();
    let t = r.scroll_thumb("scroll").unwrap();
    let (x, y) = (t.x + t.w / 2., t.y + t.h / 2.);
    assert_eq!(r.scroll_pointer(P::Down, x, y).as_deref(), Some("scroll"));
    r.render_at(&v, 100, 80, 200).unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), Some(true));
    assert_eq!(r.scroll_hide_deadline_ms(), None);
    r.render_at(&v, 100, 80, 2000).unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), Some(true));
    assert_eq!(r.scroll_pointer(P::Up, x, y).as_deref(), Some("scroll"));
    r.render_at(&v, 100, 80, 2500).unwrap();
    assert_eq!(r.scroll_hide_deadline_ms(), Some(2600));
    r.render_at(&v, 100, 80, 2600).unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), Some(false));
}
#[test]
fn disabled_delay_immediate_render_no_overflow_and_removal_have_no_timer() {
    let (mut r, v) = (renderer(0), view(30));
    r.render_at(&v, 100, 80, 0).unwrap();
    r.render_at(&v, 100, 80, 10000).unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), Some(true));
    assert_eq!(r.scroll_hide_deadline_ms(), None);
    let mut r = renderer(100);
    r.render_at(&v, 100, 80, 0).unwrap();
    r.render(&v, 100, 80).unwrap();
    assert_eq!(r.scroll_hide_deadline_ms(), None);
    assert_eq!(r.scrollbars_visible("scroll"), Some(true));
    let small = View::<()>::scroll("scroll", "Empty", vec![]);
    r.render_at(&small, 100, 80, 1000).unwrap();
    assert_eq!(r.scroll_hide_deadline_ms(), None);
    r.render_at(&View::<()>::text("replacement", ""), 100, 80, 2000)
        .unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), None);
    assert_eq!(r.scroll_hide_deadline_ms(), None);
}

#[test]
fn nested_activity_wakes_all_viewports_on_the_route() {
    let mut r = renderer(100);
    let inner = view(30).layout(Attrs {
        width: Sizing::Px(200.),
        height: Sizing::Px(200.),
        ..Default::default()
    });
    let nested = View::scroll("outer", "Outer", vec![inner]);
    r.render_at(&nested, 100, 80, 0).unwrap();
    r.render_at(&nested, 100, 80, 100).unwrap();
    assert_eq!(r.scrollbars_visible("outer"), Some(false));
    assert_eq!(r.scrollbars_visible("scroll"), Some(false));
    assert!(r.scroll_activity_at(Some("row0"), 200));
    r.render_at(&nested, 100, 80, 200).unwrap();
    assert_eq!(r.scrollbars_visible("outer"), Some(true));
    assert_eq!(r.scrollbars_visible("scroll"), Some(true));
    assert_eq!(r.scroll_hide_deadline_ms(), Some(300));
}

#[test]
fn textarea_uses_the_same_idle_deadline_and_keeps_its_selection_and_viewport() {
    use dunit_component::{Application, Component, InputOptions, Key, Theme};
    struct Text {
        value: String,
    }
    impl Component for Text {
        type Message = String;
        fn update(&mut self, value: String) {
            self.value = value;
        }
        fn view(&self) -> View<String> {
            View::text_area(
                "text",
                "Document",
                &self.value,
                |s| s,
                InputOptions::default(),
            )
            .layout(Attrs {
                width: Sizing::Grow(1.),
                height: Sizing::Grow(1.),
                ..Default::default()
            })
        }
    }
    let mut app = Application::new(Text {
        value: (0..30)
            .map(|i| format!("line {i} with long overflowing contents\n"))
            .collect(),
    })
    .unwrap();
    app.key(Key::plain('\t')).unwrap();
    app.edit(dunit_component::EditCommand::SelectAll).unwrap();
    let profile = include_str!("../../../assets/dwm/sdk/dark.dss");
    let base = include_str!("../../../assets/dwm/sdk/base.dss");
    let theme = Theme::parse(profile, base, "Textarea { scrollbar-hide-delay:100ms; }").unwrap();
    let mut r = renderer(0);
    r.set_theme(theme, None);
    r.render_application_at(&app, 160, 100, 0).unwrap();
    let before = app.view().text_editor().unwrap().selection();
    let viewport = r.scroll_viewport("text");
    let x = r.input_scroll_x("text");
    let y = r.input_scroll_y("text");
    assert_eq!(r.scroll_hide_deadline_ms(), Some(100));
    r.render_application_at(&app, 160, 100, 100).unwrap();
    assert_eq!(r.scrollbars_visible("text"), Some(false));
    assert_eq!(r.scroll_viewport("text"), viewport);
    assert_eq!(r.input_scroll_x("text"), x);
    assert_eq!(r.input_scroll_y("text"), y);
    assert_eq!(app.view().text_editor().unwrap().selection(), before);
    r.scroll_activity_at(Some("text"), 200);
    r.render_application_at(&app, 160, 100, 200).unwrap();
    assert_eq!(r.scrollbars_visible("text"), Some(true));
}

#[test]
fn tab_into_hidden_scroll_wakes_it_before_any_previous_focus_exists() {
    use dunit_component::{Application, Component, Key};
    struct Rows;
    impl Component for Rows {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self) -> View<()> {
            view(30)
        }
    }
    let mut app = Application::new(Rows).unwrap();
    let mut r = renderer(100);
    r.render_application_at(&app, 100, 80, 0).unwrap();
    r.render_application_at(&app, 100, 80, 100).unwrap();
    assert_eq!(app.focused_key(), None);
    assert_eq!(r.scrollbars_visible("scroll"), Some(false));
    assert!(app.key(Key::plain('\t')).unwrap());
    assert_eq!(app.focused_key(), Some("scroll"));
    r.render_application_at(&app, 100, 80, 200).unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), Some(true));
    assert_eq!(r.scroll_hide_deadline_ms(), Some(300));
    assert!(app.key(Key::plain('\t')).unwrap());
    assert_eq!(app.focused_key(), Some("row0"));
    r.render_application_at(&app, 100, 80, 250).unwrap();
    assert_eq!(r.scroll_hide_deadline_ms(), Some(350));
    r.render_application_at(&app, 100, 80, 350).unwrap();
    assert_eq!(r.scrollbars_visible("scroll"), Some(false));
}
