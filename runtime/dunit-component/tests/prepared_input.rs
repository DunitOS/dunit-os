use dunit_component::{
    Application, Attrs, Component, EditCommand, Font, InputOptions, Key, Renderer, Sizing, Theme,
    View,
};
#[derive(Clone)]
enum Message {
    Text(String),
}
struct Document(String);
impl Component for Document {
    type Message = Message;
    fn update(&mut self, m: Message) {
        let Message::Text(s) = m;
        self.0 = s;
    }
    fn view(&self) -> View<Message> {
        View::column(
            "root",
            vec![View::text_area(
                "field",
                "Document",
                &self.0,
                Message::Text,
                InputOptions {
                    soft_wrap: true,
                    ..Default::default()
                },
            )
            .layout(Attrs {
                width: Sizing::Grow(1.),
                height: Sizing::Grow(1.),
                ..Default::default()
            })],
        )
    }
}
fn renderer(px: u32) -> Renderer {
    let theme = Theme::parse(
        include_str!("../../../assets/dwm/sdk/dark.dss"),
        include_str!("../../../assets/dwm/sdk/base.dss"),
        &format!("Textarea {{font-size:{px};}}"),
    )
    .unwrap();
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        theme.cascade,
        theme.background,
    )
}
#[test]
fn edits_history_resize_and_style_rebuild_shared_geometry_without_stale_hits_or_pixels() {
    let value = "alpha e\u{301} beta gamma delta\n".repeat(20);
    let mut app = Application::new(Document(value.clone())).unwrap();
    let mut r = renderer(14);
    app.key(Key::plain('\t')).unwrap();
    for (w, h) in [(320, 160), (120, 80), (320, 160)] {
        r.render_application(&app, w, h).unwrap();
        r.edit_input(&mut app, EditCommand::DocumentEnd { extend: false })
            .unwrap();
        app.key(Key::plain('a')).unwrap();
        r.render_application(&app, w, h).unwrap();
        assert_eq!(app.component().0, format!("{value}a"));
        app.edit(EditCommand::Undo).unwrap();
        r.render_application(&app, w, h).unwrap();
        assert_eq!(app.component().0, value);
        let mut fresh = renderer(14);
        fresh.render_application(&app, w, h).unwrap();
        assert_eq!(r.pixels(), fresh.pixels());
        let rect = r.widget_rect("field").unwrap();
        for (x, y) in [
            (rect.x + 8., rect.y + 8.),
            (rect.x + rect.w - 8., rect.y + rect.h - 8.),
        ] {
            assert_eq!(
                r.input_position_at("field", x, y),
                fresh.input_position_at("field", x, y)
            );
        }
    }
    let theme = Theme::parse(
        include_str!("../../../assets/dwm/sdk/dark.dss"),
        include_str!("../../../assets/dwm/sdk/base.dss"),
        "Textarea {font-size:20;}",
    )
    .unwrap();
    r.set_theme(theme, None);
    r.render_application(&app, 200, 100).unwrap();
    let mut fresh = renderer(20);
    fresh.render_application(&app, 200, 100).unwrap();
    assert_eq!(r.pixels(), fresh.pixels());
    let rect = r.widget_rect("field").unwrap();
    assert_eq!(
        r.input_position_at("field", rect.x + 30., rect.y + 15.),
        fresh.input_position_at("field", rect.x + 30., rect.y + 15.)
    );
}
