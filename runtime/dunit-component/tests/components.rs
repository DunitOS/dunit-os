use dunit_component::{
    Application, Attrs, Cascade, Color, Component, Font, Key, Renderer, Sizing, UiError, View,
};

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Add,
    Text(String),
}
struct Counter {
    count: u32,
    text: String,
}
impl Component for Counter {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Add => self.count += 1,
            Msg::Text(text) => self.text = text,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                line("header", "Header"),
                line("counter", format!("{} {}", self.count, self.text)),
                line("footer", "Footer"),
            ],
        )
        .on_key(Key::plain(' '), Msg::Add)
    }
}
fn line<M>(key: &str, text: impl Into<String>) -> View<M> {
    View::text(key, text).layout(Attrs {
        height: Sizing::Px(30.0),
        ..Attrs::default()
    })
}
fn counter() -> Application<Counter> {
    Application::new(Counter {
        count: 0,
        text: "hello".into(),
    })
    .unwrap()
}
fn renderer() -> Renderer {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/fonts/DejaVuSans.ttf"
    ))
    .unwrap();
    let mut cascade = Cascade::new();
    cascade.push(
        dunit_component::style::parse(
            "Column { background: #213141; } Text { color: #ffffff; font-size: 14; }",
        )
        .unwrap(),
    );
    Renderer::new(Font::parse(bytes).unwrap(), cascade, Color::rgb(0, 0, 0))
}

#[test]
fn messages_rebuild_typed_view_without_markup_escaping() {
    let mut app = counter();
    assert!(!app
        .key(Key {
            character: ' ',
            modifiers: 2
        })
        .unwrap());
    assert_eq!(app.component().count, 0);
    assert!(app.key(Key::plain(' ')).unwrap());
    app.update(Msg::Text("\"{braces}\\\nПривет".into()))
        .unwrap();
    let tree = app.view().tree().unwrap();
    assert_eq!(
        tree.node(tree.by_name("counter").unwrap()).text.as_deref(),
        Some("1 \"{braces}\\\nПривет")
    );
    assert_eq!(app.view().children()[0].key(), "header");
    assert_eq!(app.component().count, 1);
}

#[test]
fn unchanged_view_has_zero_damage_and_one_label_does_not_repaint_header() {
    let mut app = counter();
    let mut r = renderer();
    assert_eq!(
        r.render(app.view(), 200, 120).unwrap().damaged_pixels,
        24000
    );
    let before = r.pixels().to_vec();
    assert_eq!(r.render(app.view(), 200, 120).unwrap().damaged_pixels, 0);
    assert_eq!(before, r.pixels());
    app.update(Msg::Add).unwrap();
    let stats = r.render(app.view(), 200, 120).unwrap();
    assert_eq!(stats.damaged_pixels, 6000);
    let damage = stats.damage.unwrap();
    assert_eq!(
        (damage.x, damage.y, damage.width, damage.height),
        (0, 30, 200, 30)
    );
    assert_eq!(&before[..200 * 30], &r.pixels()[..200 * 30]);
    assert_eq!(&before[200 * 60..], &r.pixels()[200 * 60..]);
    assert_ne!(&before[200 * 30..200 * 60], &r.pixels()[200 * 30..200 * 60]);
    let mut full = renderer();
    full.render(app.view(), 200, 120).unwrap();
    assert_eq!(r.pixels(), full.pixels());
}

#[test]
fn long_text_shrinks_without_ghost_pixels_and_clips_to_its_box() {
    let mut app = counter();
    let mut r = renderer();
    app.update(Msg::Text("long text ".repeat(80))).unwrap();
    r.render(app.view(), 200, 120).unwrap();
    app.update(Msg::Text("x".into())).unwrap();
    assert_eq!(r.render(app.view(), 200, 120).unwrap().damaged_pixels, 6000);
    let mut full = renderer();
    full.render(app.view(), 200, 120).unwrap();
    assert_eq!(r.pixels(), full.pixels());
}

#[test]
fn structural_edits_and_resize_repaint_complete_scene() {
    let mut r = renderer();
    let first: View<()> = View::column("root", vec![line("a", "A"), line("b", "B")]);
    r.render(&first, 200, 120).unwrap();
    let second: View<()> = View::column("root", vec![line("b", "B"), line("a", "A")]);
    assert_eq!(r.render(&second, 200, 120).unwrap().damaged_pixels, 24000);
    let third: View<()> = View::column("root", vec![line("b", "B")]);
    assert_eq!(r.render(&third, 200, 120).unwrap().damaged_pixels, 24000);
    assert_eq!(r.render(&third, 100, 100).unwrap().damaged_pixels, 10000);
    let mut full = renderer();
    full.render(&third, 100, 100).unwrap();
    assert_eq!(r.pixels(), full.pixels());
}

#[test]
fn moved_boxes_cover_old_and_new_geometry() {
    let mut r = renderer();
    let tree = |height| {
        View::<()>::column(
            "root",
            vec![
                line("a", "A").layout(Attrs {
                    height: Sizing::Px(height),
                    ..Attrs::default()
                }),
                line("b", "B"),
            ],
        )
    };
    r.render(&tree(30.0), 200, 120).unwrap();
    r.render(&tree(60.0), 200, 120).unwrap();
    let mut full = renderer();
    full.render(&tree(60.0), 200, 120).unwrap();
    assert_eq!(r.pixels(), full.pixels());
}

#[test]
fn overlapping_translucent_nodes_do_not_accumulate_alpha() {
    let scene =
        |text| View::<()>::stack("root", vec![View::text("a", text), View::text("b", "top")]);
    let make = || {
        let mut r = renderer();
        let mut c = Cascade::new();
        c.push(
            dunit_component::style::parse(
                "Stack { background: #202020; } Text { background: #ff000080; color: #ffffff; }",
            )
            .unwrap(),
        );
        r.set_style(c, Color::rgb(0, 0, 0));
        r
    };
    let mut r = make();
    r.render(&scene("old"), 200, 120).unwrap();
    r.render(&scene("new"), 200, 120).unwrap();
    let mut full = make();
    full.render(&scene("new"), 200, 120).unwrap();
    assert_eq!(r.pixels(), full.pixels());
}

#[test]
fn invalid_tree_does_not_replace_last_rendered_pixels() {
    let mut r = renderer();
    let app = counter();
    r.render(app.view(), 200, 120).unwrap();
    let before = r.pixels().to_vec();
    let duplicate: View<()> = View::column("root", vec![line("a", "a"), line("a", "b")]);
    assert_eq!(r.render(&duplicate, 200, 120), Err(UiError::DuplicateKey));
    assert_eq!(r.render(app.view(), 0, 120), Err(UiError::InvalidSize));
    let bad: View<()> = line("bad", "bad").layout(Attrs {
        height: Sizing::Px(f32::NAN),
        ..Attrs::default()
    });
    assert_eq!(r.render(&bad, 200, 120), Err(UiError::InvalidLayout));
    assert_eq!(r.pixels(), before);
    assert_eq!(r.render(app.view(), 200, 120).unwrap().damaged_pixels, 0);
}

#[test]
fn duplicate_nested_shortcuts_and_unbounded_depth_are_rejected() {
    let duplicate = line("a", "a")
        .on_key(Key::plain(' '), Msg::Add)
        .on_key(Key::plain(' '), Msg::Add);
    assert_eq!(duplicate.validate(), Err(UiError::DuplicateBinding));
    let nested = View::column(
        "root",
        vec![line("a", "a").on_key(Key::plain(' '), Msg::Add)],
    );
    assert_eq!(nested.validate(), Err(UiError::ChildBinding));
    let empty: View<()> = line("", "a");
    assert_eq!(empty.validate(), Err(UiError::EmptyKey));
    let mut deep = line::<()>("leaf", "a");
    for i in 0..64 {
        deep = View::column(format!("n{i}"), vec![deep]);
    }
    assert_eq!(deep.validate(), Err(UiError::TreeLimit));
}

#[test]
fn configured_container_padding_moves_layout_and_damage_together() {
    let make = || {
        let mut r = renderer();
        let mut c = Cascade::new();
        c.push(dunit_component::style::parse("Column { background: #213141; padding: 7 11; } Text { color: #ffffff; font-size: 14; padding: 2; }").unwrap());
        r.set_style(c, Color::rgb(0, 0, 0));
        r
    };
    let mut app = counter();
    let mut r = make();
    r.render(app.view(), 200, 120).unwrap();
    app.update(Msg::Add).unwrap();
    let d = r.render(app.view(), 200, 120).unwrap().damage.unwrap();
    assert_eq!((d.x, d.y, d.width, d.height), (11, 37, 178, 30));
    let mut full = make();
    full.render(app.view(), 200, 120).unwrap();
    assert_eq!(r.pixels(), full.pixels());
}
