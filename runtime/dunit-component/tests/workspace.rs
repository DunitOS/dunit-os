use dunit_component::{
    Application, Component, DialogAction as D, Font, InputOptions, Key, Navigation as N,
    PointerEvent as P, Renderer, SliderCommand as S, SplitAxis as A, SplitterOptions as O, Tab,
    Theme, UiError, View, WorkspaceCommand as K,
};
const DARK: &str = include_str!("../../../assets/dwm/sdk/dark.dss");
const LIGHT: &str = include_str!("../../../assets/dwm/sdk/light.dss");
const BASE: &str = include_str!("../../../assets/dwm/sdk/base.dss");
#[derive(Clone)]
enum Msg {
    Tab(&'static str),
    Split(i64),
    Open,
    Message,
    Close,
    Reject,
    Disable,
    Text(String),
    Act,
}
struct Model {
    tab: &'static str,
    ratio: i64,
    modal: u8,
    acts: usize,
    reject: bool,
    disabled: bool,
    text: String,
}
impl Component for Model {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Tab(t) => {
                if !self.reject {
                    self.tab = t;
                }
            }
            Msg::Split(v) => {
                if !self.reject {
                    self.ratio = v;
                }
            }
            Msg::Open => self.modal = 1,
            Msg::Message => self.modal = 2,
            Msg::Close => {
                if !self.reject {
                    self.modal = 0;
                }
            }
            Msg::Reject => self.reject = true,
            Msg::Disable => self.disabled = true,
            Msg::Text(t) => self.text = t,
            Msg::Act => self.acts += 1,
        }
    }
    fn view(&self) -> View<Msg> {
        let tabs = View::tabs(
            "tabs",
            "Pages",
            self.tab,
            vec![
                Tab::new(
                    "a",
                    "Editor",
                    Msg::Tab("a"),
                    View::input(
                        "edit",
                        "Text",
                        self.text.clone(),
                        Msg::Text,
                        InputOptions::default(),
                    ),
                ),
                Tab::new(
                    "locked",
                    "Locked",
                    Msg::Tab("locked"),
                    View::button("bad", "Bad", Msg::Act),
                )
                .disabled(true),
                Tab::new(
                    "b",
                    "Preview",
                    Msg::Tab("b"),
                    View::button("preview", "Preview action", Msg::Act),
                ),
            ],
        );
        let split = View::splitter(
            "split",
            "Pane sizes",
            self.ratio,
            O::new(A::Horizontal),
            tabs,
            View::text("side", "Side pane"),
            Msg::Split,
        )
        .disabled(self.disabled);
        let base = View::column(
            "base",
            vec![
                View::toolbar(
                    "tools",
                    "Tools",
                    vec![
                        View::button("open", "Open dialog", Msg::Open),
                        View::toolbar_separator("sep"),
                        View::button("blocked", "Disabled", Msg::Act).disabled(true),
                        View::button("message", "Message", Msg::Message),
                    ],
                ),
                split,
                View::status_bar(
                    "status",
                    "Operation status",
                    format!("Tab {} | ratio {}", self.tab, self.ratio),
                ),
            ],
        );
        let dialog = match self.modal {
            1 => Some(View::dialog(
                "dialog",
                "Settings",
                View::input(
                    "dialog-input",
                    "Name",
                    "Name",
                    Msg::Text,
                    InputOptions::default(),
                ),
                vec![
                    D::new("ok", "OK", Msg::Close),
                    D::new("cancel", "Cancel", Msg::Close),
                ],
                Some("ok"),
                Some(Msg::Close),
            )),
            2 => Some(View::message_box(
                "message-box",
                "Message",
                "A message\nSecond line",
                vec![D::new("ack", "OK", Msg::Close)],
                Some("ack"),
                Some(Msg::Close),
            )),
            _ => None,
        };
        View::modal("host", base, dialog).on_key(Key::plain('q'), Msg::Act)
    }
}
fn app() -> Application<Model> {
    Application::new(Model {
        tab: "a",
        ratio: 500,
        modal: 0,
        acts: 0,
        reject: false,
        disabled: false,
        text: "Hello".into(),
    })
    .unwrap()
}
fn r(profile: &str) -> Renderer {
    let t = Theme::parse(profile, BASE, "").unwrap();
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        t.cascade,
        t.background,
    )
}
fn node<'a>(v: &'a View<Msg>, key: &str) -> Option<&'a View<Msg>> {
    if v.key() == key {
        Some(v)
    } else {
        v.children().iter().find_map(|v| node(v, key))
    }
}
fn click(a: &mut Application<Model>, key: &str) {
    a.pointer(P::Down, Some(key)).unwrap();
    a.pointer(P::Up, Some(key)).unwrap();
}
#[test]
fn tabs_are_controlled_roving_and_only_the_active_page_accepts_input() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("open"));
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("tabs/tab/a"));
    assert!(node(a.view(), "preview").is_none());
    a.navigate(N::Right).unwrap();
    assert_eq!(a.component().tab, "b");
    assert_eq!(a.focused_key(), Some("tabs/tab/b"));
    assert!(node(a.view(), "edit").is_none());
    assert_eq!(
        node(a.view(), "tabs/tab/b").unwrap().accessibility_role(),
        Some("tab")
    );
    assert_eq!(
        node(a.view(), "tabs/tab/b")
            .unwrap()
            .accessibility_selected(),
        Some(true)
    );
    a.workspace_command(K::First).unwrap();
    assert_eq!(a.component().tab, "a");
    a.workspace_command(K::Last).unwrap();
    assert_eq!(a.component().tab, "b");
}
#[test]
fn ctrl_tab_from_a_page_skips_disabled_pages_and_model_can_reject() {
    let mut a = app();
    click(&mut a, "edit");
    a.key(Key {
        character: '\t',
        modifiers: 2,
    })
    .unwrap();
    assert_eq!(a.component().tab, "b");
    a.key(Key {
        character: '\t',
        modifiers: 3,
    })
    .unwrap();
    assert_eq!(a.component().tab, "a");
    a.update(Msg::Reject).unwrap();
    a.navigate(N::Right).unwrap();
    assert_eq!(a.component().tab, "a");
    assert!(node(a.view(), "edit").is_some());
    assert_eq!(
        node(a.view(), "tabs/tab/b")
            .unwrap()
            .accessibility_selected(),
        Some(false)
    );
}
#[test]
fn toolbar_roves_without_activation_and_status_is_passive() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.navigate(N::Right).unwrap();
    assert_eq!(a.focused_key(), Some("message"));
    assert_eq!(a.component().modal, 0);
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("tabs/tab/a"));
    assert_eq!(
        node(a.view(), "tools").unwrap().accessibility_role(),
        Some("toolbar")
    );
    assert_eq!(
        node(a.view(), "status").unwrap().accessibility_role(),
        Some("status")
    );
    assert_eq!(
        node(a.view(), "sep").unwrap().accessibility_role(),
        Some("separator")
    );
    click(&mut a, "status");
    assert_eq!(a.component().acts, 0);
}
#[test]
fn modal_traps_tab_blocks_background_and_restores_the_opener() {
    let mut a = app();
    click(&mut a, "open");
    assert_eq!(a.active_dialog(), Some("dialog"));
    assert_eq!(a.focused_key(), Some("ok"));
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("cancel"));
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("dialog-input"));
    a.key(Key::plain('q')).unwrap();
    assert_eq!(a.component().acts, 0);
    click(&mut a, "preview");
    click(&mut a, "message");
    assert_eq!(a.component().modal, 1);
    assert_eq!(a.focused_key(), Some("dialog-input"));
    a.key(Key::plain('\u{1b}')).unwrap();
    assert_eq!(a.active_dialog(), None);
    assert_eq!(a.focused_key(), Some("open"));
}
#[test]
fn dialog_default_and_rejected_cancel_do_not_leak_shortcuts() {
    let mut a = app();
    click(&mut a, "open");
    click(&mut a, "dialog-input");
    a.key(Key::plain('\n')).unwrap();
    assert_eq!(a.component().modal, 0);
    assert_eq!(a.focused_key(), Some("open"));
    click(&mut a, "open");
    a.update(Msg::Reject).unwrap();
    a.key(Key::plain('\u{1b}')).unwrap();
    assert_eq!(a.active_dialog(), Some("dialog"));
    a.key(Key {
        character: '\t',
        modifiers: 2,
    })
    .unwrap();
    assert_eq!(a.component().tab, "a");
    assert!(a
        .focused_key()
        .is_some_and(|k| k == "ok" || k == "cancel" || k == "dialog-input"));
}
#[test]
fn message_box_is_a_scoped_dialog_and_enter_acknowledges_it() {
    let mut a = app();
    click(&mut a, "message");
    assert_eq!(a.active_dialog(), Some("message-box"));
    assert_eq!(a.focused_key(), Some("ack"));
    assert_eq!(
        node(a.view(), "message-box").unwrap().accessibility_role(),
        Some("dialog")
    );
    a.key(Key::plain('\n')).unwrap();
    assert_eq!(a.focused_key(), Some("message"));
    assert_eq!(a.component().modal, 0);
}
#[test]
fn splitter_drag_keyboard_cancel_and_rejection_are_controlled() {
    let mut a = app();
    let mut r = r(DARK);
    r.render_application(&a, 640, 360).unwrap();
    let g = r.splitter_geometry("split/handle").unwrap();
    let x = g.handle.x + g.handle.w / 2. + 2.;
    a.splitter_pointer(P::Down, Some("split/handle"), Some(("split/handle", x, g)))
        .unwrap();
    assert_eq!(a.component().ratio, 500);
    a.splitter_pointer(
        P::Move,
        None,
        Some(("split/handle", g.start + g.available + 100., g)),
    )
    .unwrap();
    assert_eq!(a.component().ratio, 1000);
    a.key(Key::plain('\u{1b}')).unwrap();
    assert_eq!(a.dragging_splitter_key(), None);
    a.navigate(N::Left).unwrap();
    assert_eq!(a.component().ratio, 990);
    a.splitter_command(S::Home).unwrap();
    assert_eq!(a.component().ratio, 0);
    a.update(Msg::Reject).unwrap();
    a.splitter_command(S::End).unwrap();
    assert_eq!(a.component().ratio, 0);
    a.update(Msg::Disable).unwrap();
    assert!(!a.splitter_command(S::End).unwrap());
}
#[test]
fn splitter_geometry_respects_minima_and_degenerate_rectangles() {
    let area = dunit_component::Rect {
        x: 10.,
        y: 20.,
        w: 400.,
        h: 200.,
    };
    let g = dunit_component::SplitGeometry::new(area, 8., false, 0, 80., None, 60., None);
    assert_eq!(g.first.w, 80.);
    assert_eq!(g.first.w + g.handle.w + g.second.w, 400.);
    let v = dunit_component::SplitGeometry::new(area, 8., true, 700, 0., None, 0., None);
    assert_eq!(v.first.h + v.handle.h + v.second.h, 200.);
    assert_eq!(v.coordinate(100., 50.), 50.);
    let tiny = dunit_component::SplitGeometry::new(
        dunit_component::Rect {
            x: 0.,
            y: 0.,
            w: 1.,
            h: 1.,
        },
        8.,
        false,
        500,
        80.,
        None,
        80.,
        None,
    );
    assert_eq!(tiny.value(0., 0.), None);
}
#[test]
fn pixels_resize_modal_and_split_layout_match_a_fresh_renderer() {
    for profile in [DARK, LIGHT] {
        let mut a = app();
        let mut r = r(profile);
        r.render_application(&a, 640, 360).unwrap();
        let g = r.splitter_geometry("split/handle").unwrap();
        assert_eq!(
            r.hit_key(g.handle.x + g.handle.w / 2., g.handle.y + g.handle.h / 2.),
            Some("split/handle")
        );
        a.update(Msg::Split(700)).unwrap();
        assert!(r.render_application(&a, 640, 360).unwrap().damage.is_some());
        let mut fresh = self::r(profile);
        fresh.render_application(&a, 640, 360).unwrap();
        assert_eq!(fresh.pixels(), r.pixels());
        assert_eq!(r.render_application(&a, 640, 360).unwrap().damage, None);
        click(&mut a, "open");
        r.render_application(&a, 640, 360).unwrap();
        let rect = r.widget_rect("dialog").unwrap();
        assert!(rect.w < 640. && rect.h < 360. && rect.x > 0. && rect.y > 0.);
        assert_eq!(r.hit_key(1., 1.), Some("host/scrim"));
        let mut fresh = self::r(profile);
        fresh.render_application(&a, 640, 360).unwrap();
        assert_eq!(fresh.pixels(), r.pixels());
        r.render_application(&a, 240, 180).unwrap();
        let rect = r.widget_rect("dialog").unwrap();
        assert!(rect.x >= 0. && rect.y >= 0. && rect.x + rect.w <= 240. && rect.y + rect.h <= 180.);
        let action = r.widget_rect("ok").unwrap();
        assert!(action.h > 0. && action.y + action.h <= rect.y + rect.h);
    }
}
#[test]
fn invalid_tab_ranges_defaults_and_multiple_dialogs_fail_validation() {
    assert_eq!(
        View::<()>::toolbar(
            "tools",
            "Tools",
            vec![View::input(
                "input",
                "Search",
                "",
                |_| (),
                InputOptions::default()
            )]
        )
        .validate(),
        Err(UiError::InvalidWorkspace)
    );
    assert_eq!(
        View::<()>::column(
            "root",
            vec![
                View::dialog("a", "A", View::text("body-a", "A"), vec![], None, None),
                View::dialog("b", "B", View::text("body-b", "B"), vec![], None, None)
            ]
        )
        .validate(),
        Err(UiError::InvalidWorkspace)
    );

    assert_eq!(
        View::<()>::tabs(
            "t",
            "Tabs",
            "missing",
            vec![Tab::new("a", "A", (), View::text("body", "A"))]
        )
        .validate(),
        Err(UiError::InvalidWorkspace)
    );
    assert_eq!(
        View::<()>::tabs(
            "t",
            "Tabs",
            "a",
            vec![
                Tab::new("a", "A", (), View::text("a", "A")),
                Tab::new("a", "B", (), View::text("b", "B"))
            ]
        )
        .validate(),
        Err(UiError::InvalidWorkspace)
    );
    assert_eq!(
        View::<()>::dialog(
            "d",
            "Dialog",
            View::text("body", "Body"),
            vec![],
            Some("missing"),
            None
        )
        .validate(),
        Err(UiError::InvalidWorkspace)
    );
    let mut o = O::new(A::Horizontal);
    o.range.max = 1001;
    assert_eq!(
        View::<()>::splitter(
            "s",
            "Split",
            500,
            o,
            View::text("a", "A"),
            View::text("b", "B"),
            |_| ()
        )
        .validate(),
        Err(UiError::InvalidWorkspace)
    );
}

#[test]
fn nested_splitters_use_the_final_parent_pane_rectangles() {
    struct Nested;
    impl Component for Nested {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self) -> View<()> {
            View::splitter(
                "outer",
                "Outer",
                700,
                O::new(A::Horizontal),
                View::splitter(
                    "inner",
                    "Inner",
                    250,
                    O::new(A::Vertical),
                    View::text("top", "Top"),
                    View::text("bottom", "Bottom"),
                    |_| (),
                ),
                View::text("right", "Right"),
                |_| (),
            )
        }
    }
    let a = Application::new(Nested).unwrap();
    let mut r = r(DARK);
    r.render_application(&a, 640, 360).unwrap();
    let outer = r.splitter_geometry("outer/handle").unwrap();
    let inner = r.splitter_geometry("inner/handle").unwrap();
    assert_eq!(inner.handle.w, outer.first.w);
    assert_eq!(inner.first.y, outer.first.y);
    assert_eq!(r.widget_rect("top").unwrap(), inner.first);
    assert_eq!(r.widget_rect("bottom").unwrap(), inner.second);
    assert_eq!(
        r.hit_key(
            inner.handle.x + inner.handle.w / 2.,
            inner.handle.y + inner.handle.h / 2.
        ),
        Some("inner/handle")
    );
}
#[test]
fn layout_subtree_rearrangement_rejects_invalid_inputs_atomically() {
    let tree = View::<()>::column("root", vec![View::text("child", "Text")])
        .tree()
        .unwrap();
    let mut layout = dunit_ui::layout(&tree, 100., 80.);
    let id = tree.by_name("child").unwrap();
    let before = layout.rect(id);
    assert!(!layout.relayout_subtree(
        &tree,
        id,
        dunit_component::Rect {
            x: f32::NAN,
            ..before
        },
        &|_| (0., 0.)
    ));
    assert_eq!(layout.rect(id), before);
    assert!(!layout.relayout_subtree(&tree, dunit_ui::NodeId(999), before, &|_| (0., 0.)));
    assert_eq!(layout.rect(id), before);
    assert!(dunit_ui::layout::measure_subtree(&tree, id, 100., 80., &|_| (20., 12.)).is_some());
}

#[test]
fn modal_scope_blocks_background_menus_and_parent_tooltips() {
    struct Menus(bool);
    impl Component for Menus {
        type Message = Msg;
        fn update(&mut self, m: Msg) {
            match m {
                Msg::Open => self.0 = true,
                Msg::Close => self.0 = false,
                _ => {}
            }
        }
        fn view(&self) -> View<Msg> {
            let base = View::column(
                "base",
                vec![
                    View::menu_bar(
                        "bar",
                        vec![View::menu("file", "File", vec![]).menu_mnemonic('f')],
                    ),
                    View::button("open", "Open", Msg::Open),
                ],
            );
            let dialog = self.0.then(|| {
                View::dialog(
                    "d",
                    "Dialog",
                    View::menu("inside", "Inside", vec![]).menu_mnemonic('d'),
                    vec![D::new("ok", "OK", Msg::Close)],
                    Some("ok"),
                    Some(Msg::Close),
                )
            });
            View::modal("host", base, dialog).tooltip("Background tooltip")
        }
    }
    let mut a = Application::new(Menus(false)).unwrap();
    a.pointer(P::Down, Some("open")).unwrap();
    a.pointer(P::Up, Some("open")).unwrap();
    a.key(Key {
        character: 'f',
        modifiers: 4,
    })
    .unwrap();
    assert!(a.popup_state().is_none());
    a.tooltip_pointer(Some("ok"), 0);
    assert!(a.tooltip_candidate().is_none());
    a.key(Key {
        character: 'd',
        modifiers: 4,
    })
    .unwrap();
    assert_eq!(a.popup_state().unwrap().owner, "inside");
    a.key(Key::plain('\u{1b}')).unwrap();
    assert!(a.popup_state().is_none());
    assert_eq!(a.active_dialog(), Some("d"));
    a.key(Key::plain('\u{1b}')).unwrap();
    assert_eq!(a.active_dialog(), None);
    assert_eq!(a.focused_key(), Some("open"));
}
#[test]
fn toolbar_home_end_remember_focus_and_dialog_replacement_restore_base() {
    let mut a = app();
    a.key(Key::plain('\t')).unwrap();
    a.workspace_command(K::Last).unwrap();
    assert_eq!(a.focused_key(), Some("message"));
    a.key(Key::plain('\t')).unwrap();
    a.key(Key {
        character: '\t',
        modifiers: 1,
    })
    .unwrap();
    assert_eq!(a.focused_key(), Some("message"));
    a.workspace_command(K::First).unwrap();
    assert_eq!(a.focused_key(), Some("open"));
    a.key(Key::plain('\n')).unwrap();
    a.update(Msg::Message).unwrap();
    assert_eq!(a.active_dialog(), Some("message-box"));
    a.key(Key::plain('\n')).unwrap();
    assert_eq!(a.focused_key(), Some("open"));
}
