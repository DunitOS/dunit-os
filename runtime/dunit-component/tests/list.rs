use dunit_component::{
    Application, Attrs, Component, Font, Key, ListCommand as K, ListGeometry, ListItem as I,
    ListSelection as S, PointerEvent as P, Renderer, SelectionMode as M, Sizing, Theme, UiError,
    View,
};
fn items(n: usize) -> Vec<I> {
    (0..n)
        .map(|i| I::new(format!("r{i}"), format!("Document {i}")).disabled(i == 2))
        .collect()
}
#[derive(Clone)]
enum Msg {
    Select(S),
    Reject,
    Disable,
    Shrink,
    Reverse,
    Modal(bool),
}
struct Model {
    rows: Vec<I>,
    s: S,
    mode: M,
    reject: bool,
    disabled: bool,
    modal: bool,
    calls: usize,
}
impl Component for Model {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Select(s) => {
                self.calls += 1;
                if !self.reject {
                    self.s = s;
                }
            }
            Msg::Reject => self.reject = true,
            Msg::Disable => self.disabled = true,
            Msg::Shrink => {
                self.rows.truncate(5);
                self.s.retain(&self.rows);
            }
            Msg::Reverse => self.rows.reverse(),
            Msg::Modal(v) => self.modal = v,
        }
    }
    fn view(&self) -> View<Msg> {
        let list = View::list(
            "list",
            "Documents",
            self.rows.clone(),
            self.s.clone(),
            self.mode,
            Msg::Select,
        )
        .context_menu(vec![dunit_component::MenuItem::action(
            "noop",
            "Close menu",
            Msg::Modal(false),
        )])
        .disabled(self.disabled)
        .layout(Attrs {
            width: Sizing::Grow(1.),
            height: Sizing::Grow(1.),
            ..Default::default()
        });
        View::modal(
            "root",
            list,
            self.modal.then(|| {
                View::message_box(
                    "dialog",
                    "Scope",
                    "Message",
                    vec![dunit_component::DialogAction::new(
                        "ok",
                        "OK",
                        Msg::Modal(false),
                    )],
                    Some("ok"),
                    Some(Msg::Modal(false)),
                )
            }),
        )
    }
}
fn app(mode: M, n: usize) -> Application<Model> {
    Application::new(Model {
        rows: items(n),
        s: S::default(),
        mode,
        reject: false,
        disabled: false,
        modal: false,
        calls: 0,
    })
    .unwrap()
}
fn renderer(light: bool) -> Renderer {
    let t = Theme::parse(
        if light {
            include_str!("../../../assets/dwm/sdk/light.dss")
        } else {
            include_str!("../../../assets/dwm/sdk/dark.dss")
        },
        include_str!("../../../assets/dwm/sdk/base.dss"),
        "",
    )
    .unwrap();
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        t.cascade,
        t.background,
    )
}
fn focus(a: &mut Application<Model>) {
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("list"));
}
#[test]
fn single_navigation_skips_disabled_and_has_one_tab_stop() {
    let mut a = app(M::Single, 8);
    focus(&mut a);
    a.list_command(K::Next, 0, 1).unwrap();
    a.list_command(K::Next, 0, 1).unwrap();
    a.list_command(K::Next, 0, 1).unwrap();
    assert_eq!(a.component().s.current.as_deref(), Some("r3"));
    assert_eq!(a.component().s.selected.len(), 1);
    a.list_command(K::Last, 0, 1).unwrap();
    a.list_command(K::Next, 0, 1).unwrap();
    assert_eq!(a.component().calls, 4);
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("list"));
}
#[test]
fn multiple_ranges_toggle_and_ctrl_navigation_are_controlled() {
    let mut a = app(M::Multiple, 10);
    focus(&mut a);
    a.list_command(K::Next, 0, 1).unwrap();
    a.list_command(K::Next, 1, 1).unwrap();
    a.list_command(K::Next, 1, 1).unwrap();
    assert_eq!(
        a.component().s.selected.iter().cloned().collect::<Vec<_>>(),
        vec!["r0", "r1", "r3"]
    );
    a.list_command(K::Next, 2, 1).unwrap();
    assert_eq!(a.component().s.current.as_deref(), Some("r4"));
    assert_eq!(a.component().s.selected.len(), 3);
    a.key(Key::plain(' ')).unwrap();
    assert_eq!(a.component().s.selected.len(), 4);
    a.key(Key {
        character: 'a',
        modifiers: 2,
    })
    .unwrap();
    assert_eq!(a.component().s.selected.len(), 9);
}
#[test]
fn rejected_and_disabled_changes_preserve_model_and_scope() {
    let mut a = app(M::Single, 8);
    focus(&mut a);
    a.update(Msg::Reject).unwrap();
    a.list_command(K::Last, 0, 1).unwrap();
    assert_eq!(a.component().s, S::default());
    a.update(Msg::Disable).unwrap();
    assert!(!a.list_command(K::Next, 0, 1).unwrap());
    assert!(!a.list_pointer(P::Down, Some("list"), Some(0), 0).unwrap());
}
#[test]
fn pointer_uses_stable_row_and_preserves_selection_on_blank_or_disabled() {
    let mut a = app(M::Multiple, 8);
    a.list_pointer(P::Down, Some("list"), Some(1), 0).unwrap();
    a.list_pointer(P::Down, Some("list"), Some(4), 1).unwrap();
    assert_eq!(a.component().s.selected.len(), 3);
    let before = a.component().s.clone();
    for i in [None, Some(2), Some(99)] {
        a.list_pointer(P::Down, Some("list"), i, 0).unwrap();
        assert_eq!(a.component().s, before);
    }
    a.list_pointer(P::Down, Some("list"), Some(1), 2).unwrap();
    assert!(!a.component().s.selected.contains("r1"));
}
#[test]
fn reorder_and_removal_use_keys_instead_of_indexes() {
    let mut a = app(M::Multiple, 20);
    focus(&mut a);
    a.list_command(K::Last, 0, 1).unwrap();
    a.update(Msg::Reverse).unwrap();
    assert_eq!(a.component().s.current.as_deref(), Some("r19"));
    a.list_command(K::Next, 1, 1).unwrap();
    assert!(a.component().s.selected.contains("r18"));
    a.update(Msg::Reverse).unwrap();
    a.update(Msg::Shrink).unwrap();
    assert_eq!(a.component().s, S::default());
}
#[test]
fn modal_blocks_background_list_pointer_and_keyboard() {
    let mut a = app(M::Single, 20);
    focus(&mut a);
    a.update(Msg::Modal(true)).unwrap();
    assert!(!a.list_pointer(P::Down, Some("list"), Some(4), 0).unwrap());
    assert!(!a.list_command(K::Next, 0, 1).unwrap());
    a.key(Key::plain('\u{1b}')).unwrap();
    assert_eq!(a.focused_key(), Some("list"));
}
#[test]
fn list_validation_rejects_duplicate_keys_stale_selection_and_multiple_single() {
    let mut rows = items(3);
    rows[1].key = "r0".into();
    assert_eq!(
        View::list("l", "L", rows, S::default(), M::Single, Msg::Select).validate(),
        Err(UiError::InvalidList)
    );
    let mut s = S::default();
    s.selected.insert("missing".into());
    assert_eq!(
        View::list("l", "L", items(3), s, M::Single, Msg::Select).validate(),
        Err(UiError::InvalidList)
    );
    let s = S::default().select_all(&items(5), M::Multiple);
    assert_eq!(
        View::list("l", "L", items(5), s, M::Single, Msg::Select).validate(),
        Err(UiError::InvalidList)
    );
}
#[test]
fn geometry_visible_range_partial_rows_hit_testing_and_tiny_viewports() {
    let g = ListGeometry {
        viewport: dunit_component::Rect {
            x: 10.,
            y: 20.,
            w: 100.,
            h: 50.,
        },
        row_height: 28.,
        offset: 15.,
        count: 10000,
    };
    assert_eq!(g.visible(), 0..3);
    assert_eq!(g.row_at(11., 21.), Some(0));
    assert_eq!(g.row_at(11., 34.), Some(1));
    assert_eq!(g.row_at(110., 21.), None);
    assert_eq!(g.row_at(11., 70.), None);
    assert_eq!(
        ListGeometry {
            viewport: dunit_component::Rect {
                h: 0.,
                ..g.viewport
            },
            ..g
        }
        .visible(),
        0..0
    );
}
#[test]
fn ten_thousand_rows_virtualize_reveal_wheel_and_clamp_in_both_themes() {
    for light in [false, true] {
        let mut a = app(M::Multiple, 10000);
        focus(&mut a);
        let mut r = renderer(light);
        r.render(a.view(), 320, 200).unwrap();
        assert!(r.list_visible_rows("list").unwrap() <= 9);
        assert!(r.scroll_thumb("list").is_some());
        a.list_command(K::Last, 0, 1).unwrap();
        r.render(a.view(), 320, 200).unwrap();
        assert!(r.list_geometry("list").unwrap().visible().contains(&9999));
        let before = r.scroll_offset("list").unwrap();
        assert!(r.scroll_wheel(Some("list"), -1));
        r.render(a.view(), 320, 200).unwrap();
        assert!(r.scroll_offset("list").unwrap() < before);
        assert!(r.render(a.view(), 320, 200).unwrap().damage.is_none());
        a.update(Msg::Shrink).unwrap();
        r.render(a.view(), 320, 200).unwrap();
        assert_eq!(r.scroll_offset("list"), Some(0.));
        assert_eq!(r.list_visible_rows("list"), Some(5));
        assert!(r.scroll_thumb("list").is_none());
    }
}
#[test]
fn retained_pixels_equal_fresh_render_after_selection_and_resize() {
    for light in [false, true] {
        let mut a = app(M::Multiple, 20);
        focus(&mut a);
        let mut r = renderer(light);
        r.render(a.view(), 300, 180).unwrap();
        a.list_command(K::First, 0, 1).unwrap();
        assert!(r.render(a.view(), 300, 180).unwrap().damage.is_some());
        let mut fresh = renderer(light);
        fresh.render(a.view(), 300, 180).unwrap();
        assert_eq!(r.pixels(), fresh.pixels());
        for (w, h) in [(420, 260), (12, 12), (300, 180)] {
            r.render(a.view(), w, h).unwrap();
            let mut fresh = renderer(light);
            fresh.render(a.view(), w, h).unwrap();
            assert_eq!(r.pixels(), fresh.pixels());
        }
    }
}
#[test]
fn page_commands_clamp_and_ctrl_shift_adds_ranges() {
    let mut a = app(M::Multiple, 20);
    focus(&mut a);
    a.list_command(K::First, 0, 1).unwrap();
    a.list_command(K::PageDown, 1, 5).unwrap();
    assert_eq!(a.component().s.current.as_deref(), Some("r6"));
    assert_eq!(a.component().s.selected.len(), 6);
    a.list_command(K::Last, 2, 1).unwrap();
    a.list_command(K::Toggle, 2, 1).unwrap();
    a.list_command(K::Previous, 3, 1).unwrap();
    assert!(a.component().s.selected.contains("r0"));
    assert!(a.component().s.selected.contains("r18"));
    a.list_command(K::PageUp, 0, usize::MAX).unwrap();
    assert_eq!(a.component().s.current.as_deref(), Some("r0"));
}

#[test]
fn empty_and_all_disabled_lists_consume_navigation_without_dispatch() {
    for n in [0, 3] {
        let mut rows = items(n);
        rows.iter_mut().for_each(|r| r.disabled = true);
        let mut a = Application::new(Model {
            rows,
            s: S::default(),
            mode: M::Multiple,
            reject: false,
            disabled: false,
            modal: false,
            calls: 0,
        })
        .unwrap();
        focus(&mut a);
        for command in [
            K::Next,
            K::Previous,
            K::First,
            K::Last,
            K::PageDown,
            K::PageUp,
            K::Toggle,
            K::SelectAll,
        ] {
            assert!(a.list_command(command, 0, 10).unwrap());
        }
        assert_eq!(a.component().calls, 0);
        let mut r = renderer(false);
        r.render(a.view(), 200, 200).unwrap();
        assert!(r.scroll_thumb("list").is_none());
    }
}
#[test]
fn ancestor_clipping_bounds_materialized_rows_and_scrollbar_drag_uses_shared_axis() {
    let list = View::list(
        "list",
        "Rows",
        items(10000),
        S::default(),
        M::Single,
        Msg::Select,
    )
    .layout(Attrs {
        height: Sizing::Px(1000.),
        width: Sizing::Grow(1.),
        ..Default::default()
    });
    let root = View::scroll("outer", "Outer", vec![list]);
    let mut r = renderer(false);
    r.render(&root, 320, 180).unwrap();
    assert!(r.list_visible_rows("list").unwrap() < 8);
    let mut a = app(M::Single, 10000);
    focus(&mut a);
    r.render(a.view(), 320, 180).unwrap();
    let thumb = r.scroll_thumb("list").unwrap();
    let track = r.scroll_track("list").unwrap();
    let x = thumb.x + thumb.w / 2.;
    let y = thumb.y + thumb.h / 2.;
    assert_eq!(r.scroll_pointer(P::Down, x, y).as_deref(), Some("list"));
    assert_eq!(
        r.scroll_pointer(P::Move, x, track.y + track.h).as_deref(),
        Some("list")
    );
    r.scroll_pointer(P::Up, x, track.y + track.h);
    r.render(a.view(), 320, 180).unwrap();
    assert!(r.list_geometry("list").unwrap().visible().contains(&9999));
    assert!(r.list_visible_rows("list").unwrap() < 8);
}

#[test]
fn open_popup_captures_direct_list_adapters_without_changing_selection() {
    let mut a = app(M::Multiple, 20);
    focus(&mut a);
    a.list_command(K::First, 0, 1).unwrap();
    let selection = a.component().s.clone();
    assert!(a.open_context_menu("list", None));
    assert!(!a.list_command(K::Last, 0, 1).unwrap());
    assert!(a.list_pointer(P::Down, Some("list"), Some(7), 0).unwrap());
    assert_eq!(a.component().s, selection);
}
