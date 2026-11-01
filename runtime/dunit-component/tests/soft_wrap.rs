use dunit_component::{
    Application, Attrs, CaretAffinity, Component, EditCommand as E, Font, InputOptions, Key,
    PointerEvent, Renderer, Sizing, TextPosition, Theme, View,
};
struct Demo {
    value: String,
    reject: bool,
    wrap: bool,
}
#[derive(Clone)]
enum Msg {
    Value(String),
    Reject,
    Unwrap,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Value(s) => {
                if !self.reject {
                    self.value = s
                }
            }
            Msg::Reject => self.reject = true,
            Msg::Unwrap => self.wrap = false,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                View::text_area(
                    "editor",
                    "Document",
                    &self.value,
                    Msg::Value,
                    InputOptions {
                        soft_wrap: self.wrap,
                        ..Default::default()
                    },
                )
                .layout(Attrs {
                    width: Sizing::Px(64.),
                    height: Sizing::Px(40.),
                    ..Default::default()
                }),
                View::button("other", "Other", Msg::Reject),
            ],
        )
    }
}
fn app(value: &str) -> Application<Demo> {
    Application::new(Demo {
        value: value.into(),
        reject: false,
        wrap: true,
    })
    .unwrap()
}
fn renderer() -> Renderer {
    let theme=Theme::parse(include_str!("../../../assets/dwm/sdk/dark.dss"),"","Column { align-items: start; } Textarea { font-size: 14; caret-width: 2; caret-color: #ff0000; color: #ffffff; selection-color: #334455; }").unwrap();
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        theme.cascade,
        theme.background,
    )
}
fn editor(app: &Application<Demo>) -> &dunit_component::TextEditor {
    app.view().children()[0].text_editor().unwrap()
}
fn edit(r: &mut Renderer, a: &mut Application<Demo>, e: E) {
    assert!(r.edit_input(a, e).unwrap());
    r.render_application(a, 300, 160).unwrap();
}
fn focus(r: &mut Renderer, a: &mut Application<Demo>) {
    a.key(Key::plain('\t')).unwrap();
    r.render_application(a, 300, 160).unwrap();
    edit(r, a, E::DocumentHome { extend: false });
}
#[test]
fn line_end_affinity_and_home_navigate_visual_rows_without_model_changes() {
    let mut a = app("abcdefghijklmnopqrstuv");
    let mut r = renderer();
    focus(&mut r, &mut a);
    edit(&mut r, &mut a, E::End { extend: false });
    let end = editor(&a).cursor();
    assert!(end > 0 && end < a.component().value.len());
    edit(&mut r, &mut a, E::Home { extend: false });
    assert_eq!(editor(&a).cursor(), 0);
    edit(&mut r, &mut a, E::Down { extend: false });
    let next = editor(&a).cursor();
    assert_eq!(next, end);
    edit(&mut r, &mut a, E::End { extend: true });
    assert!(editor(&a).selection().len() > 0);
    assert!(editor(&a).cursor() > next);
    edit(&mut r, &mut a, E::DocumentEnd { extend: false });
    assert_eq!(editor(&a).cursor(), a.component().value.len());
    assert!(r.input_scroll_y("editor").unwrap() > 0.);
    assert_eq!(r.input_scroll_x("editor"), Some(0.));
    assert_eq!(a.component().value, "abcdefghijklmnopqrstuv");
    assert!(!editor(&a).can_undo());
}
#[test]
fn pointer_hit_at_wrap_end_keeps_caret_on_clicked_row_after_render() {
    let mut a = app("abcdefghijklmnopqrstuv");
    let mut r = renderer();
    focus(&mut r, &mut a);
    let rect = r.widget_rect("editor").unwrap();
    let hit = r
        .input_position_at("editor", rect.x + rect.w - 1., rect.y + 1.)
        .unwrap();
    assert_eq!(hit.affinity, CaretAffinity::Upstream);
    a.pointer_at(
        PointerEvent::Down,
        Some("editor"),
        Some(("editor", hit.index)),
    )
    .unwrap();
    a.set_input_affinity("editor", hit);
    r.render_application(&a, 300, 160).unwrap();
    edit(&mut r, &mut a, E::Home { extend: false });
    assert_eq!(editor(&a).cursor(), 0);
    let second = TextPosition {
        index: hit.index,
        affinity: CaretAffinity::Downstream,
    };
    a.place_visual_input("editor", second, false, None);
    r.render_application(&a, 300, 160).unwrap();
    edit(&mut r, &mut a, E::Home { extend: false });
    assert_eq!(editor(&a).cursor(), hit.index);
}
#[test]
fn pixel_goal_survives_short_logical_rows_and_shift_extends_selection() {
    let mut a = app("WWWW\ni\nWWWW");
    let mut r = renderer();
    focus(&mut r, &mut a);
    a.place_visual_input(
        "editor",
        TextPosition {
            index: 3,
            affinity: CaretAffinity::Downstream,
        },
        false,
        None,
    );
    r.render_application(&a, 300, 160).unwrap();
    edit(&mut r, &mut a, E::Down { extend: true });
    assert_eq!(editor(&a).cursor(), 6);
    assert_eq!(editor(&a).anchor(), 3);
    edit(&mut r, &mut a, E::Down { extend: true });
    assert_eq!(editor(&a).cursor(), 10);
    assert_eq!(editor(&a).selection(), 3..10);
    edit(&mut r, &mut a, E::Up { extend: false });
    assert_eq!(editor(&a).cursor(), 6);
    assert!(editor(&a).selection().is_empty());
}
#[test]
fn rejected_edits_restore_affinity_and_history_then_resize_matches_fresh_renderer() {
    let mut a = app("abcdefghijklmnopqrstuv");
    let mut r = renderer();
    focus(&mut r, &mut a);
    edit(&mut r, &mut a, E::End { extend: false });
    let end = editor(&a).cursor();
    a.update(Msg::Reject).unwrap();
    edit(&mut r, &mut a, E::Insert('z'));
    assert_eq!(editor(&a).cursor(), end);
    assert!(!editor(&a).can_undo());
    edit(&mut r, &mut a, E::Home { extend: false });
    assert_eq!(editor(&a).cursor(), 0);
    r.render_application(&a, 400, 220).unwrap();
    let mut fresh = renderer();
    fresh.render_application(&a, 400, 220).unwrap();
    assert_eq!(r.pixels(), fresh.pixels());
    a.update(Msg::Unwrap).unwrap();
    r.render_application(&a, 300, 160).unwrap();
    assert!(r.input_scroll_x("editor").unwrap() >= 0.);
    assert_eq!(a.component().value, "abcdefghijklmnopqrstuv");
}
