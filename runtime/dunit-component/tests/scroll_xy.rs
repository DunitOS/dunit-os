//! Two-axis viewport, scrollbar capture and immutable-render evidence.
use dunit_component::{
    style, Application, Attrs, Cascade, Color, Component, Font, InputOptions, Key,
    PointerEvent as P, Renderer, ScrollAxes, ScrollCommand, ScrollDirection as D, Sizing, View,
};

fn size<M>(view: View<M>, width: f32, height: f32) -> View<M> {
    view.layout(Attrs {
        width: Sizing::Px(width),
        height: Sizing::Px(height),
        ..Attrs::default()
    })
}
fn grid(prefix: &str) -> Vec<View<()>> {
    (0..12)
        .map(|row| {
            size(
                View::row(
                    format!("{prefix}-row{row}"),
                    (0..3)
                        .map(|col| {
                            size(
                                View::button(format!("{prefix}-{row}-{col}"), "", ())
                                    .class(if col % 2 == 0 { "even" } else { "odd" }),
                                80.,
                                20.,
                            )
                        })
                        .collect(),
                ),
                240.,
                20.,
            )
        })
        .collect()
}
fn view() -> View<()> {
    View::scroll("xy", "Grid", grid("cell")).scroll_axes(ScrollAxes::Both)
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
    let mut cascade = Cascade::new();
    cascade.push(style::parse("Column, Row, Stack { padding: 0; } Scroll, Textarea { padding: 0; border-width: 0; scrollbar-width: 8; scrollbar-min-thumb: 12; scrollbar-radius: 0; scrollbar-track: #304050; scrollbar-thumb: #b0c0d0; scroll-step: 20; background: #102030; } Textarea { font-size: 12; caret-width: 1; caret-color: #ffffff; caret-blink: 500ms; color: #ffffff; } Button.even { background: #224466; } Button.odd { background: #aa4422; }").unwrap());
    Renderer::new(font, cascade, Color::rgb(0, 0, 0))
}
fn paint(r: &mut Renderer, v: &View<()>) {
    r.render(v, 100, 80).unwrap();
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.02, "{a} != {b}");
}

#[test]
fn two_axis_offsets_move_pixels_hits_and_reserve_disjoint_gutters() {
    let v = view();
    let mut r = renderer();
    paint(&mut r, &v);
    let vp = r.scroll_viewport("xy").unwrap();
    assert_eq!((vp.w, vp.h), (92., 72.));
    let vertical = r.scroll_track("xy").unwrap();
    let horizontal = r.scroll_track_x("xy").unwrap();
    assert_eq!((vertical.x, vertical.y, vertical.h), (92., 0., 72.));
    assert_eq!((horizontal.x, horizontal.y, horizontal.w), (0., 72., 92.));
    assert_eq!(r.hit_key(95., 10.), Some("xy"));
    assert_eq!(r.hit_key(10., 75.), Some("xy"));
    assert_eq!(r.hit_key(95., 75.), Some("xy"));
    assert_eq!(r.hit_key(10., 10.), Some("cell-0-0"));
    let before = r.pixels().to_vec();
    assert!(r.scroll_by_x("xy", 90.));
    assert!(r.scroll_by("xy", 20.));
    assert_eq!(r.pixels(), before); // mutations cannot alter painted pixels
    assert_eq!(r.hit_key(10., 10.), Some("cell-0-0"));
    paint(&mut r, &v);
    assert_eq!(r.hit_key(10., 10.), Some("cell-1-1"));
    assert_eq!(r.pixels()[10 * 100 + 10], 0xffaa4422);
    assert_eq!(r.scroll_offset_x("xy"), Some(90.));
    assert_eq!(r.scroll_offset("xy"), Some(20.));
    let mut fresh = renderer();
    paint(&mut fresh, &v);
    fresh.scroll_by_x("xy", 90.);
    fresh.scroll_by("xy", 20.);
    paint(&mut fresh, &v);
    assert_eq!(r.pixels(), fresh.pixels());
}

#[test]
fn horizontal_drag_keeps_anchor_track_pages_and_leave_cancels() {
    let v = view();
    let mut r = renderer();
    paint(&mut r, &v);
    let thumb = r.scroll_thumb_x("xy").unwrap();
    let y = thumb.y + thumb.h / 2.;
    assert_eq!(
        r.scroll_pointer(P::Down, thumb.x + 3., y).as_deref(),
        Some("xy")
    );
    r.scroll_pointer(P::Move, thumb.x + 23., y);
    paint(&mut r, &v);
    close(r.scroll_thumb_x("xy").unwrap().x, thumb.x + 20.);
    let offset = r.scroll_offset_x("xy").unwrap();
    r.scroll_pointer(P::Leave, 0., 0.);
    assert!(r.scroll_pointer(P::Move, 99., y).is_none());
    paint(&mut r, &v);
    close(r.scroll_offset_x("xy").unwrap(), offset);
    r.scroll_by_x("xy", -f32::MAX);
    paint(&mut r, &v);
    let track = r.scroll_track_x("xy").unwrap();
    assert_eq!(
        r.scroll_pointer(P::Down, track.x + track.w - 1., y)
            .as_deref(),
        Some("xy")
    );
    paint(&mut r, &v);
    close(r.scroll_offset_x("xy").unwrap(), 92.);
    close(r.scroll_offset("xy").unwrap(), 0.);
}

#[test]
fn nested_horizontal_wheel_bubbles_only_at_inner_boundary() {
    let inner = size(
        View::scroll("inner", "Inner", grid("inner-cell")).scroll_axes(ScrollAxes::Both),
        180.,
        100.,
    );
    let v = View::scroll(
        "outer",
        "Outer",
        vec![inner, size(View::text("tail", ""), 300., 200.)],
    )
    .scroll_axes(ScrollAxes::Both);
    let mut r = renderer();
    paint(&mut r, &v);
    assert!(r.scroll_wheel_axis(Some("inner-cell-0-0"), 1, D::Horizontal));
    paint(&mut r, &v);
    assert_eq!(r.scroll_offset_x("inner"), Some(20.));
    assert_eq!(r.scroll_offset_x("outer"), Some(0.));
    r.scroll_by_x("inner", f32::MAX);
    paint(&mut r, &v);
    assert!(r.scroll_wheel_axis(Some("inner-cell-0-0"), 1, D::Horizontal));
    paint(&mut r, &v);
    assert_eq!(r.scroll_offset_x("outer"), Some(20.));
    assert_eq!(r.hit_key(95., 10.), Some("outer"));
}

struct Grid;
impl Component for Grid {
    type Message = ();
    fn update(&mut self, _: ()) {}
    fn view(&self) -> View<()> {
        view()
    }
}
#[test]
fn keyboard_focus_reveals_both_axes_and_axis_mode_changes_reset_disabled_axis() {
    let mut app = Application::new(Grid).unwrap();
    let mut r = renderer();
    paint(&mut r, app.view());
    for _ in 0..40 {
        if app.focused_key() == Some("cell-10-2") {
            break;
        }
        app.key(Key::plain('\t')).unwrap();
        paint(&mut r, app.view());
    }
    assert_eq!(app.focused_key(), Some("cell-10-2"));
    assert!(r.scroll_offset_x("xy").unwrap() > 0.);
    assert!(r.scroll_offset("xy").unwrap() > 0.);
    assert!(r.scroll_key_axis(app.view(), ScrollCommand::Home, D::Horizontal));
    paint(&mut r, app.view());
    assert_eq!(r.scroll_offset_x("xy"), Some(0.));
    let vertical = view().scroll_axes(ScrollAxes::Vertical);
    paint(&mut r, &vertical);
    assert_eq!(r.scroll_offset_x("xy"), None);
    assert!(r.scroll_track_x("xy").is_none());
    let horizontal = view().scroll_axes(ScrollAxes::Horizontal);
    paint(&mut r, &horizontal);
    assert_eq!(r.scroll_offset("xy"), None);
    assert!(r.scroll_track("xy").is_none());
    paint(&mut r, &view().disabled(true));
    assert_eq!(r.scroll_offset_x("xy"), None);
    assert_eq!(r.scroll_offset("xy"), None);
}

struct Text;
impl Component for Text {
    type Message = ();
    fn update(&mut self, _: ()) {}
    fn view(&self) -> View<()> {
        View::text_area(
            "text",
            "Document",
            (0..30)
                .map(|_| "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz")
                .collect::<Vec<_>>()
                .join("\n"),
            |_| (),
            InputOptions::default(),
        )
    }
}
#[test]
fn textarea_bars_page_drag_and_blink_preserve_manual_offsets_and_pixels() {
    let mut app = Application::new(Text).unwrap();
    app.key(Key::plain('\t')).unwrap();
    let mut r = renderer();
    paint(&mut r, app.view());
    assert!(r.scroll_thumb("text").is_some());
    assert!(r.scroll_thumb_x("text").is_some());
    r.scroll_by("text", -f32::MAX);
    r.scroll_by_x("text", -f32::MAX);
    paint(&mut r, app.view());
    close(r.scroll_offset("text").unwrap(), 0.);
    close(r.scroll_offset_x("text").unwrap(), 0.);
    let track = r.scroll_track("text").unwrap();
    let page = r.scroll_viewport("text").unwrap().h;
    let x = track.x + track.w / 2.;
    assert_eq!(
        r.scroll_pointer(P::Down, x, track.y + track.h - 1.)
            .as_deref(),
        Some("text")
    );
    paint(&mut r, app.view());
    close(r.scroll_offset("text").unwrap(), page);
    r.scroll_by("text", -f32::MAX);
    paint(&mut r, app.view());
    let thumb = r.scroll_thumb("text").unwrap();
    let pixels = r.pixels().to_vec();
    r.scroll_pointer(P::Down, x, thumb.y + 2.);
    r.scroll_pointer(P::Move, x, thumb.y + 2.);
    paint(&mut r, app.view());
    close(r.scroll_offset("text").unwrap(), 0.);
    assert_eq!(r.pixels(), pixels);
    r.scroll_pointer(P::Move, x, thumb.y + 22.);
    assert_eq!(r.pixels(), pixels);
    paint(&mut r, app.view());
    assert!(r.scroll_offset("text").unwrap() > 0.);
    close(r.scroll_thumb("text").unwrap().y, thumb.y + 20.);
    r.scroll_pointer(P::Up, x, thumb.y + 22.);
    let offset = r.scroll_offset("text").unwrap();
    app.blink_caret();
    paint(&mut r, app.view());
    close(r.scroll_offset("text").unwrap(), offset);
    r.scroll_by_x("text", 100.);
    paint(&mut r, app.view());
    let horizontal = r.scroll_thumb_x("text").unwrap();
    let y = horizontal.y + horizontal.h / 2.;
    r.scroll_pointer(P::Down, horizontal.x + 2., y);
    r.scroll_pointer(P::Move, horizontal.x + 12., y);
    paint(&mut r, app.view());
    assert!(r.scroll_offset_x("text").unwrap() > 100.);
    r.scroll_pointer(P::Leave, 0., 0.);
    let offset = r.scroll_offset_x("text").unwrap();
    assert!(r.scroll_pointer(P::Move, 99., y).is_none());
    paint(&mut r, app.view());
    close(r.scroll_offset_x("text").unwrap(), offset);
}

#[test]
fn unfocused_textarea_thumb_press_pins_both_axes_before_focus_and_first_move() {
    for direction in [D::Vertical, D::Horizontal] {
        let mut app = Application::new(Text).unwrap();
        let mut r = renderer();
        paint(&mut r, app.view());
        assert_eq!(app.focused_key(), None);
        let selection = app.view().text_editor().unwrap().selection();
        r.scroll_by("text", -f32::MAX);
        r.scroll_by_x("text", -f32::MAX);
        paint(&mut r, app.view());
        let thumb = match direction {
            D::Vertical => r.scroll_thumb("text"),
            D::Horizontal => r.scroll_thumb_x("text"),
        }
        .unwrap();
        let (x, y) = match direction {
            D::Vertical => (thumb.x + thumb.w / 2., thumb.y + 2.),
            D::Horizontal => (thumb.x + 2., thumb.y + thumb.h / 2.),
        };
        let pixels = r.pixels().to_vec();
        let key = r.scroll_pointer(P::Down, x, y).unwrap();
        assert_eq!(r.pixels(), pixels);
        app.scrollbar_pointer(P::Down, &key).unwrap();
        assert_eq!(app.focused_key(), Some("text"));
        assert_eq!(app.dragging_input_key(), None);
        assert_eq!(app.view().text_editor().unwrap().selection(), selection);
        paint(&mut r, app.view());
        close(r.scroll_offset("text").unwrap(), 0.);
        close(r.scroll_offset_x("text").unwrap(), 0.);
        assert_eq!(r.scroll_thumb("text").is_some(), true);
        let focused_thumb = match direction {
            D::Vertical => r.scroll_thumb("text"),
            D::Horizontal => r.scroll_thumb_x("text"),
        }
        .unwrap();
        assert_eq!(focused_thumb, thumb);
        r.scroll_pointer(P::Move, x, y);
        app.scrollbar_pointer(P::Move, &key).unwrap();
        paint(&mut r, app.view());
        close(r.scroll_offset("text").unwrap(), 0.);
        close(r.scroll_offset_x("text").unwrap(), 0.);
        let (move_x, move_y) = match direction {
            D::Vertical => (x, y + 10.),
            D::Horizontal => (x + 10., y),
        };
        r.scroll_pointer(P::Move, move_x, move_y);
        app.scrollbar_pointer(P::Move, &key).unwrap();
        paint(&mut r, app.view());
        let moved = match direction {
            D::Vertical => r.scroll_thumb("text").unwrap().y,
            D::Horizontal => r.scroll_thumb_x("text").unwrap().x,
        };
        let start = match direction {
            D::Vertical => thumb.y,
            D::Horizontal => thumb.x,
        };
        close(moved, start + 10.);
        assert_eq!(app.view().text_editor().unwrap().selection(), selection);
        assert_eq!(app.dragging_input_key(), None);
    }
}

struct EmptyText {
    disabled: bool,
}
impl Component for EmptyText {
    type Message = ();
    fn update(&mut self, _: ()) {}
    fn view(&self) -> View<()> {
        View::text_area("empty", "Empty", "", |_| (), InputOptions::default())
            .disabled(self.disabled)
    }
}

#[test]
fn reserved_textarea_gutters_and_corner_consume_down_even_when_bars_are_hidden() {
    for (x, y) in [(95., 10.), (10., 75.), (95., 75.)] {
        let mut app = Application::new(EmptyText { disabled: false }).unwrap();
        let mut r = renderer();
        paint(&mut r, app.view());
        assert!(r.scroll_track("empty").is_none());
        assert!(r.scroll_track_x("empty").is_none());
        let key = r.scroll_pointer(P::Down, x, y).unwrap();
        assert_eq!(key, "empty");
        app.scrollbar_pointer(P::Down, &key).unwrap();
        paint(&mut r, app.view());
        assert_eq!(app.dragging_input_key(), None);
        assert_eq!(app.view().text_editor().unwrap().selection(), 0..0);
        assert_eq!(app.focused_key(), Some("empty"));
        assert!(r.scroll_pointer(P::Move, x, y).is_none());
        assert!(r.scroll_pointer(P::Up, x, y).is_none());
        close(r.scroll_offset("empty").unwrap(), 0.);
        close(r.scroll_offset_x("empty").unwrap(), 0.);
    }
    let app = Application::new(EmptyText { disabled: true }).unwrap();
    let mut r = renderer();
    paint(&mut r, app.view());
    assert!(r.scroll_pointer(P::Down, 95., 75.).is_none());

    let app = Application::new(EmptyText { disabled: false }).unwrap();
    let mut padded = renderer();
    let mut cascade = Cascade::new();
    cascade
        .push(style::parse("Textarea { padding: 4; font-size: 12; scrollbar-width: 8; }").unwrap());
    padded.set_style(cascade, Color::rgb(0, 0, 0));
    paint(&mut padded, app.view());
    assert!(padded.scroll_pointer(P::Down, 1., 1.).is_none());
    assert_eq!(
        padded.scroll_pointer(P::Down, 90., 70.).as_deref(),
        Some("empty")
    );
}

#[test]
fn textarea_reserved_corner_focus_preserves_manual_offsets_and_selection() {
    let mut app = Application::new(Text).unwrap();
    let mut r = renderer();
    paint(&mut r, app.view());
    let selection = app.view().text_editor().unwrap().selection();
    r.scroll_by("text", -f32::MAX);
    r.scroll_by_x("text", -f32::MAX);
    paint(&mut r, app.view());
    let vp = r.scroll_viewport("text").unwrap();
    let key = r
        .scroll_pointer(P::Down, vp.x + vp.w + 2., vp.y + vp.h + 2.)
        .unwrap();
    app.scrollbar_pointer(P::Down, &key).unwrap();
    paint(&mut r, app.view());
    close(r.scroll_offset("text").unwrap(), 0.);
    close(r.scroll_offset_x("text").unwrap(), 0.);
    assert_eq!(app.view().text_editor().unwrap().selection(), selection);
    assert_eq!(app.dragging_input_key(), None);
}

#[test]
fn queued_textarea_horizontal_and_line_scroll_compose_in_either_order_before_frame() {
    for wheel in [false, true] {
        for vertical_first in [false, true] {
            let app = Application::new(Text).unwrap();
            let mut r = renderer();
            paint(&mut r, app.view());
            r.scroll_by("text", -f32::MAX);
            r.scroll_by_x("text", -f32::MAX);
            paint(&mut r, app.view());
            let pixels = r.pixels().to_vec();
            let thumb_x = r.scroll_thumb_x("text");
            let thumb_y = r.scroll_thumb("text");

            // Measure the same vertical action alone, so the combined test
            // does not duplicate font metrics or line-step policy.
            let mut vertical_only = renderer();
            paint(&mut vertical_only, app.view());
            vertical_only.scroll_by("text", -f32::MAX);
            vertical_only.scroll_by_x("text", -f32::MAX);
            paint(&mut vertical_only, app.view());
            if wheel {
                assert!(vertical_only.scroll_input_wheel("text", 1));
            } else {
                assert!(vertical_only.scroll_input("text", 2.));
            }
            paint(&mut vertical_only, app.view());
            let expected_y = vertical_only.scroll_offset("text").unwrap();
            assert!(expected_y > 0.);

            if !vertical_first {
                assert!(r.scroll_by_x("text", 45.));
            }
            if wheel {
                assert!(r.scroll_input_wheel("text", 1));
            } else {
                assert!(r.scroll_input("text", 2.));
            }
            if vertical_first {
                assert!(r.scroll_by_x("text", 45.));
            }
            // Queuing must leave the painted frame and its hit geometry intact.
            assert_eq!(r.pixels(), pixels);
            assert_eq!(r.scroll_thumb_x("text"), thumb_x);
            assert_eq!(r.scroll_thumb("text"), thumb_y);
            paint(&mut r, app.view());
            close(r.scroll_offset_x("text").unwrap(), 45.);
            close(r.scroll_offset("text").unwrap(), expected_y);

            let mut independent = renderer();
            paint(&mut independent, app.view());
            independent.scroll_by("text", -f32::MAX);
            independent.scroll_by_x("text", -f32::MAX);
            paint(&mut independent, app.view());
            independent.scroll_by("text", expected_y);
            independent.scroll_by_x("text", 45.);
            paint(&mut independent, app.view());
            assert_eq!(r.pixels(), independent.pixels());
            assert_eq!(r.scroll_thumb_x("text"), independent.scroll_thumb_x("text"));
            assert_eq!(r.scroll_thumb("text"), independent.scroll_thumb("text"));
        }
    }
}
