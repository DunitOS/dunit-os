use dunit_component::{
    Application, Attrs, Component, Font, Key, Renderer, Sizing, Theme, UiError, View,
};

fn theme(rules: &str) -> Theme {
    Theme::parse(include_str!("../../../assets/dwm/sdk/dark.dss"), "", rules).unwrap()
}
fn renderer(rules: &str) -> Renderer {
    let theme = theme(rules);
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        theme.cascade,
        theme.background,
    )
}
fn button(key: &str) -> View<()> {
    View::button(key, key, ())
}
fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
}

struct Stateful {
    disabled: bool,
}
#[derive(Clone)]
enum Msg {
    Disable,
}
impl Component for Stateful {
    type Message = Msg;
    fn update(&mut self, _: Msg) {
        self.disabled = true;
    }
    fn view(&self) -> View<Msg> {
        View::row(
            "row",
            vec![
                View::button("first", "First", Msg::Disable).disabled(self.disabled),
                View::button("second", "Second", Msg::Disable),
            ],
        )
    }
}

#[test]
fn focused_and_disabled_states_reflow_siblings_and_restore_exact_pixels() {
    let rules = "Row { gap: 10; align-items: start; }\n\
        Button { width: 40; height: 24; background: #204060; }\n\
        Button:focus { width: 100; }\n\
        Button:disabled { width: 65; }";
    let mut app = Application::new(Stateful { disabled: false }).unwrap();
    let mut r = renderer(rules);
    r.render_application(&app, 320, 80).unwrap();
    near(r.widget_rect("second").unwrap().x, 50.);
    app.key(Key::plain('\t')).unwrap();
    r.render_application(&app, 320, 80).unwrap();
    near(r.widget_rect("first").unwrap().w, 100.);
    near(r.widget_rect("second").unwrap().x, 110.);
    let focused = r.pixels().to_vec();
    app.key(Key::plain('\t')).unwrap();
    r.render_application(&app, 320, 80).unwrap();
    near(r.widget_rect("first").unwrap().w, 40.);
    app.key(Key {
        character: '\t',
        modifiers: 1,
    })
    .unwrap();
    r.render_application(&app, 320, 80).unwrap();
    assert_eq!(r.pixels(), focused);
    app.update(Msg::Disable).unwrap();
    r.render_application(&app, 320, 80).unwrap();
    near(r.widget_rect("first").unwrap().w, 65.);
    near(r.widget_rect("second").unwrap().x, 75.);
    let mut fresh = renderer(rules);
    fresh.render_application(&app, 320, 80).unwrap();
    assert_eq!(r.pixels(), fresh.pixels());
}

#[test]
fn dss_overrides_typed_sizing_and_resolves_percent_against_resized_parent() {
    let view = View::column(
        "column",
        vec![button("child").layout(Attrs {
            width: Sizing::Px(1.),
            height: Sizing::Px(1.),
            ..Default::default()
        })],
    );
    let mut r = renderer("Column { align-items: start; } Button { width: 50%; height: 30; min-width: 80; max-width: 160; }");
    for (viewport, expected) in [(100, 80.), (200, 100.), (600, 160.)] {
        r.render(&view, viewport, 100).unwrap();
        let rect = r.widget_rect("child").unwrap();
        near(rect.w, expected);
        near(rect.h, 30.);
    }
}

#[test]
fn flex_growth_alignment_and_constraints_apply_to_arranged_geometry() {
    let view = View::row("row", vec![button("a"), button("b"), button("c")]);
    let mut r = renderer(
        "Row { gap: 10; align-items: center; }\n\
        Button { width: 20; height: 20; }\n\
        #a { flex-grow: 1; max-width: 60; }\n\
        #b { flex-grow: 2; min-height: 30; }\n\
        #c { width: 40; height: 25; }",
    );
    r.render(&view, 240, 100).unwrap();
    let a = r.widget_rect("a").unwrap();
    let b = r.widget_rect("b").unwrap();
    let c = r.widget_rect("c").unwrap();
    near(a.w, 60.);
    near(b.w, 20. + 140. * 2. / 3.);
    near(c.w, 40.);
    near(a.y, 40.);
    near(b.y, 35.);
    near(c.y, 37.5);
    near(b.x, a.x + a.w + 10.);
    near(c.x, b.x + b.w + 10.);
}

#[test]
fn grid_columns_gap_and_stretch_follow_dss_and_theme_changes() {
    let grid = View::grid("grid", vec![button("a"), button("b"), button("c")]);
    let mut r = renderer("Grid { grid-columns: 2; gap: 10; } Button { width: grow; height: 20; }");
    r.render(&grid, 210, 100).unwrap();
    let a = r.widget_rect("a").unwrap();
    let b = r.widget_rect("b").unwrap();
    let c = r.widget_rect("c").unwrap();
    near(a.w, 100.);
    near(b.x, 110.);
    near(c.x, 0.);
    near(c.y, 30.);
    // A theme replacement may change container policy without rebuilding views.
    r.set_theme(
        theme("Grid { grid-columns: 3; gap: 15; } Button { width: grow; height: 20; }"),
        None,
    );
    r.render(&grid, 210, 100).unwrap();
    near(r.widget_rect("c").unwrap().x, 150.);
    near(r.widget_rect("a").unwrap().w, 60.);
}

#[test]
fn invalid_layout_values_are_rejected_before_theme_application() {
    for declaration in [
        "width: -1",
        "height: 101%",
        "min-width: auto",
        "max-height: -1",
        "flex-grow: -1",
        "gap: -1",
        "grid-columns: 0",
        "grid-columns: 1.5",
        "align-items: nonsense",
        "justify-content: nonsense",
        "min-width: 100; max-width: 20",
        "min-height: 100; max-height: 20",
    ] {
        assert!(
            Theme::parse(
                include_str!("../../../assets/dwm/sdk/dark.dss"),
                "",
                &format!("Button {{ {declaration}; }}")
            )
            .is_err(),
            "{declaration}"
        );
    }
}

#[test]
fn contradictory_resolved_bounds_preserve_last_frame_and_recover() {
    let mut app = Application::new(Stateful { disabled: false }).unwrap();
    // The invalid combination only exists after resolving a widget state;
    // each declaration and rule is otherwise valid in isolation.
    let mut r = renderer(
        "Button { width: 40; height: 20; max-width: 80; } Button:focus { min-width: 100; }",
    );
    r.render_application(&app, 200, 100).unwrap();
    let pixels = r.pixels().to_vec();
    let rect = r.widget_rect("first").unwrap();
    app.key(Key::plain('\t')).unwrap();
    assert!(matches!(
        r.render_application(&app, 200, 100),
        Err(UiError::InvalidLayout)
    ));
    assert_eq!(r.pixels(), pixels);
    assert_eq!(r.widget_rect("first"), Some(rect));
    app.update(Msg::Disable).unwrap();
    r.render_application(&app, 200, 100).unwrap();
    let mut fresh = renderer(
        "Button { width: 40; height: 20; max-width: 80; } Button:focus { min-width: 100; }",
    );
    fresh.render_application(&app, 200, 100).unwrap();
    assert_eq!(r.pixels(), fresh.pixels());
}

#[test]
fn grid_cell_constraints_explicit_size_and_alignment_survive_resize() {
    let grid = View::grid("grid", vec![button("a"), button("b")]);
    let mut r = renderer("Grid { grid-columns: 2; gap: 10; justify-content: end; align-items: center; } Button { width: grow; height: 40; } #a { max-width: 30; height: 20; } #b { width: 50%; }");
    for width in [210, 410] {
        r.render(&grid, width, 100).unwrap();
        let cell = (width as f32 - 10.) / 2.;
        let a = r.widget_rect("a").unwrap();
        let b = r.widget_rect("b").unwrap();
        near(a.w, 30.);
        near(a.h, 20.);
        near(a.x, cell - 30.);
        near(a.y, 10.);
        near(b.w, cell / 2.);
        near(b.x, cell + 10. + cell / 2.);
        near(b.h, 40.);
    }
}

#[test]
fn explicit_zero_flex_weight_disables_typed_grow_without_losing_cross_fill() {
    let view = View::row(
        "row",
        vec![
            button("a").layout(Attrs {
                width: Sizing::Grow(1.),
                height: Sizing::Grow(1.),
                ..Default::default()
            }),
            button("b").layout(Attrs {
                width: Sizing::Grow(1.),
                ..Default::default()
            }),
        ],
    );
    let mut r = renderer("Button { min-width: 40; } #a { flex-grow: 0; }");
    r.render(&view, 200, 80).unwrap();
    near(r.widget_rect("a").unwrap().w, 40.);
    near(r.widget_rect("a").unwrap().h, 80.);
    near(r.widget_rect("b").unwrap().w, 160.);
    near(r.widget_rect("b").unwrap().x, 40.);
}

#[test]
fn cleared_bounds_and_main_alignment_follow_available_space() {
    let view = View::row(
        "row",
        vec![
            button("a").layout(Attrs {
                min_w: Some(120.),
                max_w: Some(120.),
                ..Default::default()
            }),
            button("b"),
            button("c"),
        ],
    );
    let mut r = renderer("Row { justify-content: space-between; align-items: stretch; } Button { width: 40; } #a { min-width: none; max-width: none; }");
    for width in [200, 400] {
        r.render(&view, width, 80).unwrap();
        let a = r.widget_rect("a").unwrap();
        let b = r.widget_rect("b").unwrap();
        let c = r.widget_rect("c").unwrap();
        near(a.w, 40.);
        near(a.h, 80.);
        near(a.x, 0.);
        near(b.x, (width as f32 - 40.) / 2.);
        near(c.x, width as f32 - 40.);
    }
}

#[test]
fn rejected_theme_preflight_keeps_published_frame_hit_geometry_and_focus() {
    let mut app = Application::new(Stateful { disabled: false }).unwrap();
    let mut r = renderer("Button { width: 40; height: 24; }");
    app.key(Key::plain('\t')).unwrap();
    r.render_application(&app, 320, 80).unwrap();
    let pixels = r.pixels().to_vec();
    let rect = r.widget_rect("first");
    let invalid = theme("Button { min-width: 100; } #first {max-width: 20;}");
    assert_eq!(
        r.validate_theme_application(&app, &invalid, None, 320, 80),
        Err(UiError::InvalidLayout)
    );
    assert_eq!(r.pixels(), pixels);
    assert_eq!(r.widget_rect("first"), rect);
    assert_eq!(app.focused_key(), Some("first"));
    let valid = theme("Button {width:80;height:30;}");
    r.validate_theme_application(&app, &valid, None, 320, 80)
        .unwrap();
    r.set_theme(valid, None);
    r.render_application(&app, 320, 80).unwrap();
    near(r.widget_rect("first").unwrap().w, 80.);
}
