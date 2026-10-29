//! Timed selection scrolling is tested as deterministic renderer steps.
use dunit_component::{
    style, Application, Attrs, Cascade, Color, Component, Font, InputOptions, PointerEvent as P,
    Renderer, Sizing, View,
};

struct Model {
    multiline: bool,
    readonly: bool,
    disabled: bool,
    calls: usize,
    value: String,
}
impl Component for Model {
    type Message = String;
    fn update(&mut self, value: String) {
        self.calls += 1;
        self.value = value;
    }
    fn view(&self) -> View<String> {
        let options = InputOptions {
            read_only: self.readonly,
            ..InputOptions::default()
        };
        let view = if self.multiline {
            View::text_area("field", "Document", self.value.clone(), |s| s, options)
        } else {
            View::input("field", "Name", self.value.clone(), |s| s, options)
        };
        view.disabled(self.disabled)
    }
}
fn renderer(interval: Option<f32>, step: Option<f32>) -> Renderer {
    let font = Font::parse(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/DejaVuSans.ttf"
        ))
        .unwrap(),
    )
    .unwrap();
    let mut declarations = String::from("padding: 0; border-width: 0; font-size: 12; scrollbar-width: 8; caret-width: 1; background: #102030; color: #ffffff; caret-color: #ffffff; caret-blink: 500ms;");
    if let Some(interval) = interval {
        declarations.push_str(&format!(" selection-scroll-interval: {interval}ms;"));
    }
    if let Some(step) = step {
        declarations.push_str(&format!(" selection-scroll-step: {step};"));
    }
    let mut cascade = Cascade::new();
    cascade.push(
        style::parse(&format!(
            "Input, Textarea {{ {declarations} }} Scroll {{ padding: 0; scrollbar-width: 8; }}"
        ))
        .unwrap(),
    );
    Renderer::new(font, cascade, Color::rgb(0, 0, 0))
}
fn model(multiline: bool, readonly: bool, disabled: bool) -> Model {
    let line = "aé界🙂".repeat(20);
    Model {
        multiline,
        readonly,
        disabled,
        calls: 0,
        value: if multiline {
            vec![line; 40].join("\n")
        } else {
            line
        },
    }
}
fn paint(r: &mut Renderer, app: &Application<Model>) {
    r.render(app.view(), 100, 80).unwrap();
}
fn captured(multiline: bool, readonly: bool) -> (Application<Model>, Renderer) {
    let mut app = Application::new(model(multiline, readonly, false)).unwrap();
    let mut r = renderer(Some(25.), Some(20.));
    paint(&mut r, &app);
    app.pointer_at(P::Down, Some("field"), Some(("field", 0)))
        .unwrap();
    paint(&mut r, &app);
    assert_eq!(app.dragging_input_key(), Some("field"));
    (app, r)
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.02, "{a} != {b}");
}

#[test]
fn stationary_single_line_drag_steps_both_directions_without_mutating_painted_snapshot() {
    let (mut app, mut r) = captured(false, false);
    let old_text = app.component().value.clone();
    let old_pixels = r.pixels().to_vec();
    let old_index = r.input_index_at("field", 50., 40.);
    let mut indices = Vec::new();
    for _ in 0..3 {
        assert_eq!(r.input_autoscroll_ms("field", 1000., 40.), Some(25));
        let index = r.input_autoscroll_step("field", 1000., 40.).unwrap();
        assert!(app.drag_selection("field", index));
        indices.push(index);
        assert_eq!(r.pixels(), old_pixels);
        assert_eq!(r.input_index_at("field", 50., 40.), old_index);
        close(r.input_scroll_x("field").unwrap(), 0.);
    }
    assert!(indices.windows(2).all(|w| w[0] <= w[1]));
    paint(&mut r, &app);
    close(r.input_scroll_x("field").unwrap(), 60.);
    close(r.input_scroll_y("field").unwrap(), 0.);
    assert_eq!(app.view().text_editor().unwrap().anchor(), 0);
    assert_eq!(app.component().value, old_text);
    assert_eq!(app.component().calls, 0);
    assert!(!app.view().text_editor().unwrap().can_undo());
    for _ in 0..3 {
        let index = r.input_autoscroll_step("field", -1000., 40.).unwrap();
        app.drag_selection("field", index);
    }
    assert!(r.input_autoscroll_ms("field", -1000., 40.).is_none());
    assert!(r.input_autoscroll_step("field", -1000., 40.).is_none());
    paint(&mut r, &app);
    close(r.input_scroll_x("field").unwrap(), 0.);
    // Single-line fields never scroll vertically, regardless of pointer distance.
    assert!(r.input_autoscroll_ms("field", 50., 1000.).is_none());
}

#[test]
fn readonly_textarea_diagonal_steps_extend_selection_and_stop_at_content_bounds() {
    let (mut app, mut r) = captured(true, true);
    let text = app.component().value.clone();
    let pixels = r.pixels().to_vec();
    let thumbs = (r.scroll_thumb_x("field"), r.scroll_thumb("field"));
    for _ in 0..3 {
        let index = r.input_autoscroll_step("field", 1000., 1000.).unwrap();
        assert!(text.is_char_boundary(index));
        app.drag_selection("field", index);
    }
    assert_eq!(r.pixels(), pixels);
    assert_eq!((r.scroll_thumb_x("field"), r.scroll_thumb("field")), thumbs);
    paint(&mut r, &app);
    close(r.input_scroll_x("field").unwrap(), 60.);
    close(r.input_scroll_y("field").unwrap(), 60.);
    assert!(app.view().text_editor().unwrap().selection().end > 0);
    assert_eq!(app.component().value, text);
    assert_eq!(app.component().calls, 0);
    assert!(!app.view().text_editor().unwrap().can_undo());
    assert!(!app.view().text_editor().unwrap().can_redo());
    // Repeated stationary ticks must terminate using pending controller state,
    // even without a new painted frame between ticks.
    let mut stopped = false;
    for _ in 0..200 {
        if let Some(index) = r.input_autoscroll_step("field", 1000., 1000.) {
            app.drag_selection("field", index);
        } else {
            stopped = true;
            break;
        }
    }
    assert!(stopped);
    assert!(r.input_autoscroll_ms("field", 1000., 1000.).is_none());
    paint(&mut r, &app);
    let at_end = (
        r.input_scroll_x("field").unwrap(),
        r.input_scroll_y("field").unwrap(),
    );
    assert!(at_end.0 > 60. && at_end.1 > 60.);
    let index = r.input_autoscroll_step("field", -1000., -1000.).unwrap();
    app.drag_selection("field", index);
    paint(&mut r, &app);
    close(r.input_scroll_x("field").unwrap(), at_end.0 - 20.);
    close(r.input_scroll_y("field").unwrap(), at_end.1 - 20.);
}

#[test]
fn timer_requires_focus_finite_pointer_positive_configuration_and_outward_motion() {
    for (interval, step) in [
        (None, Some(20.)),
        (Some(25.), None),
        (Some(0.), Some(20.)),
        (Some(25.), Some(0.)),
    ] {
        let mut app = Application::new(model(true, false, false)).unwrap();
        let mut r = renderer(interval, step);
        app.pointer_at(P::Down, Some("field"), Some(("field", 0)))
            .unwrap();
        paint(&mut r, &app);
        let pixels = r.pixels().to_vec();
        assert!(r.input_autoscroll_ms("field", 1000., 1000.).is_none());
        assert!(r.input_autoscroll_step("field", 1000., 1000.).is_none());
        assert_eq!(r.pixels(), pixels);
    }
    let app = Application::new(model(true, false, false)).unwrap();
    let mut r = renderer(Some(25.), Some(20.));
    paint(&mut r, &app);
    assert!(r.input_autoscroll_ms("field", -1000., -1000.).is_none());
    let app = Application::new(model(true, false, true)).unwrap();
    paint(&mut r, &app);
    assert!(r.input_autoscroll_ms("field", 1000., 1000.).is_none());
    let (mut app, mut r) = captured(true, false);
    assert!(r.input_autoscroll_ms("field", 40., 40.).is_none());
    assert!(r.input_autoscroll_ms("field", -1000., -1000.).is_none());
    for point in [
        (f32::NAN, 40.),
        (40., f32::INFINITY),
        (f32::NEG_INFINITY, 40.),
    ] {
        assert!(r.input_autoscroll_ms("field", point.0, point.1).is_none());
        assert!(r.input_autoscroll_step("field", point.0, point.1).is_none());
        assert!(r.input_drag_index_at("field", point.0, point.1).is_none());
    }
    app.pointer(P::Up, Some("field")).unwrap();
    let old = app.view().text_editor().unwrap().clone();
    assert!(!app.drag_selection("field", 3));
    assert_eq!(app.view().text_editor().unwrap(), &old);
}

#[test]
fn drag_hit_clamps_large_pointer_distance_and_nested_ancestor_clip() {
    let (app, mut r) = captured(true, false);
    let near = r.input_drag_index_at("field", 93., 73.).unwrap();
    assert_eq!(
        r.input_drag_index_at("field", f32::MAX, f32::MAX),
        Some(near)
    );
    assert_eq!(
        r.input_drag_index_at("field", -f32::MAX, -f32::MAX),
        Some(0)
    );
    assert!(near < app.component().value.len());
    let nested = View::scroll(
        "outer",
        "Outer",
        vec![app.view().clone().layout(Attrs {
            width: Sizing::Px(200.),
            height: Sizing::Px(200.),
            ..Attrs::default()
        })],
    );
    r.render(&nested, 100, 80).unwrap();
    // Effective viewport is clipped by the outer scroll content, not merely
    // the larger textarea's own 200px layout box.
    let far = r.input_drag_index_at("field", 1000., 1000.).unwrap();
    let edge = r
        .input_drag_index_at("field", 92_f32.next_down(), 80_f32.next_down())
        .unwrap();
    assert_eq!(far, edge);
    let raw = r.input_index_at("field", 199., 199.).unwrap();
    assert!(far < raw);
}

#[test]
fn drag_hit_and_autoscroll_respect_ordinary_column_and_stack_ancestor_boxes() {
    for stack in [false, true] {
        let (app, _) = captured(true, false);
        let field = app.view().clone().layout(Attrs {
            width: Sizing::Px(200.),
            height: Sizing::Px(200.),
            ..Attrs::default()
        });
        let parent = if stack {
            View::stack("small", vec![field])
        } else {
            View::column("small", vec![field])
        }
        .layout(Attrs {
            width: Sizing::Px(50.),
            height: Sizing::Px(40.),
            ..Attrs::default()
        });
        let sibling = View::text("sibling", "").layout(Attrs {
            width: Sizing::Px(100.),
            height: Sizing::Px(100.),
            ..Attrs::default()
        });
        let tree = View::row("root", vec![parent, sibling]);
        let mut r = renderer(Some(25.), Some(20.));
        r.render(&tree, 160, 100).unwrap();
        let edge = r
            .input_drag_index_at("field", 50_f32.next_down(), 40_f32.next_down())
            .unwrap();
        assert_eq!(r.input_drag_index_at("field", 1000., 1000.), Some(edge));
        assert!(edge < r.input_index_at("field", 190., 190.).unwrap());
        // This pointer is inside the oversized field/root, but outside its
        // visible ordinary parent. It must activate both edge directions.
        assert_eq!(r.input_autoscroll_ms("field", 60., 50.), Some(25));
        assert!(r.input_autoscroll_ms("field", 25., 20.).is_none());
        let index = r.input_autoscroll_step("field", 60., 50.).unwrap();
        assert!(app.component().value.is_char_boundary(index));
        r.render(&tree, 160, 100).unwrap();
        close(r.input_scroll_x("field").unwrap(), 20.);
        close(r.input_scroll_y("field").unwrap(), 20.);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CancelAction {
    None,
    Disable,
    Remove,
}
struct CancelModel {
    disabled: bool,
    removed: bool,
}
impl Component for CancelModel {
    type Message = CancelAction;
    fn update(&mut self, action: CancelAction) {
        match action {
            CancelAction::None => {}
            CancelAction::Disable => self.disabled = true,
            CancelAction::Remove => self.removed = true,
        }
    }
    fn view(&self) -> View<CancelAction> {
        let mut children = Vec::new();
        if !self.removed {
            children.push(
                View::input(
                    "field",
                    "Name",
                    "abcdef",
                    |_| CancelAction::None,
                    InputOptions::default(),
                )
                .disabled(self.disabled),
            );
        }
        children.push(View::button("button", "Other", CancelAction::None));
        View::column("root", children)
    }
}
#[test]
fn drag_selection_stops_after_leave_focus_change_disable_or_field_removal() {
    for cancellation in 0..4 {
        let mut app = Application::new(CancelModel {
            disabled: false,
            removed: false,
        })
        .unwrap();
        app.pointer_at(P::Down, Some("field"), Some(("field", 1)))
            .unwrap();
        assert!(app.drag_selection("field", 4));
        assert_eq!(app.dragging_input_key(), Some("field"));
        match cancellation {
            0 => {
                app.pointer(P::Leave, None).unwrap();
            }
            1 => {
                app.key(dunit_component::Key::plain('\t')).unwrap();
                assert_eq!(app.focused_key(), Some("button"));
            }
            2 => {
                app.update(CancelAction::Disable).unwrap();
            }
            _ => {
                app.update(CancelAction::Remove).unwrap();
            }
        }
        let before = app.view().clone();
        assert_eq!(app.dragging_input_key(), None);
        assert!(!app.drag_selection("field", 6));
        assert_eq!(app.view(), &before);
    }
}

#[test]
fn pointer_motion_between_ticks_preserves_horizontal_scroll_across_short_lines() {
    let mut m = model(true, false, false);
    m.value = format!("{}\n{}", "W".repeat(100), "x\n".repeat(40));
    let mut app = Application::new(m).unwrap();
    let mut r = renderer(Some(25.), Some(20.));
    app.pointer_at(P::Down, Some("field"), Some(("field", 0)))
        .unwrap();
    paint(&mut r, &app);
    for expected in [20., 40., 60.] {
        let index = r.input_autoscroll_step("field", 120., 100.).unwrap();
        app.drag_selection("field", index);
        paint(&mut r, &app);
        close(r.input_scroll_x("field").unwrap(), expected);
        let index = r.input_drag_index_at("field", 121., 101.).unwrap();
        app.pointer_at(P::Move, None, Some(("field", index)))
            .unwrap();
        paint(&mut r, &app);
        close(r.input_scroll_x("field").unwrap(), expected);
    }
}
