use dunit_component::{
    Application, Attrs, Component, Font, InputOptions, Key, MenuItem as I, PointerEvent as P,
    PopupCommand as K, PopupHit, Renderer, SelectOption as O, Sizing, Theme, UiError, View,
};
const DARK: &str = include_str!("../../../assets/dwm/sdk/dark.dss");
const LIGHT: &str = include_str!("../../../assets/dwm/sdk/light.dss");
const BASE: &str = include_str!("../../../assets/dwm/sdk/base.dss");
#[derive(Clone)]
enum Msg {
    Pick(String),
    Action(&'static str),
    Reject,
    Disable,
    Change,
    Hide,
    Text(String),
    Noop,
}
struct Model {
    selected: String,
    actions: Vec<&'static str>,
    calls: usize,
    reject: bool,
    disabled: bool,
    changed: bool,
    hidden: bool,
    text: String,
}
fn files() -> Vec<I<Msg>> {
    vec![
        I::action("open", "Open", Msg::Action("open")).mnemonic('o'),
        I::separator("sep"),
        I::action("locked", "Locked", Msg::Action("bad")).disabled(true),
        I::submenu(
            "recent",
            "Recent",
            vec![
                I::action("one", "One", Msg::Action("one")),
                I::action("no", "No", Msg::Action("bad")).disabled(true),
                I::action("two", "Two", Msg::Action("two")),
            ],
        ),
        I::action("last", "Last", Msg::Action("last")),
    ]
}
impl Component for Model {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Pick(s) => {
                self.calls += 1;
                if !self.reject {
                    self.selected = s;
                }
            }
            Msg::Action(s) => self.actions.push(s),
            Msg::Reject => self.reject = true,
            Msg::Disable => self.disabled = true,
            Msg::Change => self.changed = true,
            Msg::Hide => self.hidden = true,
            Msg::Text(t) => self.text = t,
            Msg::Noop => {}
        }
    }
    fn view(&self) -> View<Msg> {
        let mut children = vec![View::menu_bar(
            "bar",
            vec![
                View::menu("file", "File", files()).menu_mnemonic('f'),
                View::menu(
                    "edit",
                    "Edit",
                    vec![I::action("copy", "Copy", Msg::Action("copy"))],
                )
                .menu_mnemonic('e'),
            ],
        )];
        if !self.hidden {
            let mut opts = vec![
                O::new("a", "Alpha"),
                O::new("locked", "Locked").disabled(true),
                O::new("b", "Beta"),
                O::new("c", "Charlie"),
            ];
            if self.changed {
                opts.push(O::new("d", "Delta"));
            }
            children.push(
                View::select("pick", "Language", Some(&self.selected), opts, Msg::Pick)
                    .disabled(self.disabled)
                    .layout(Attrs {
                        width: Sizing::Px(200.),
                        height: Sizing::Px(32.),
                        ..Default::default()
                    }),
            );
        }
        children.extend([
            View::button("help", "Help", Msg::Action("help"))
                .tooltip("Delayed help that wraps in a narrow viewport")
                .context_menu(vec![
                    I::action("inspect", "Inspect", Msg::Action("inspect")),
                    I::submenu(
                        "more",
                        "More",
                        vec![I::action("detail", "Detail", Msg::Action("detail"))],
                    ),
                ]),
            View::input(
                "input",
                "Input",
                self.text.clone(),
                Msg::Text,
                InputOptions::default(),
            )
            .context_menu(vec![I::action("ctx", "Context", Msg::Action("ctx"))]),
            View::button("apply", "Apply", Msg::Action("apply")),
        ]);
        View::column("root", children)
    }
}
fn app() -> Application<Model> {
    Application::new(Model {
        selected: "a".into(),
        actions: vec![],
        calls: 0,
        reject: false,
        disabled: false,
        changed: false,
        hidden: false,
        text: "Text".into(),
    })
    .unwrap()
}
fn renderer(profile: &str, overlay: &str) -> Renderer {
    let t = Theme::parse(profile, BASE, overlay).unwrap();
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        t.cascade,
        t.background,
    )
}
fn click(a: &mut Application<Model>, key: &str) {
    a.pointer(P::Down, Some(key)).unwrap();
    a.pointer(P::Up, Some(key)).unwrap();
}
fn hit(owner: &str, path: &[usize]) -> Option<PopupHit> {
    Some(PopupHit::Item {
        owner: owner.into(),
        path: path.to_vec(),
    })
}
#[test]
fn validation_and_accessibility_have_no_ambiguous_or_invalid_choices() {
    assert_eq!(
        View::<()>::select(
            "s",
            "Select",
            Some("missing"),
            vec![O::new("a", "A")],
            |_| ()
        )
        .validate(),
        Err(UiError::InvalidPopup)
    );
    assert_eq!(
        View::<()>::select(
            "s",
            "Select",
            None,
            vec![O::new("a", "A"), O::new("a", "B")],
            |_| ()
        )
        .validate(),
        Err(UiError::InvalidPopup)
    );
    assert_eq!(
        View::<()>::menu(
            "m",
            "Menu",
            vec![I::action("a", "A", ()), I::action("a", "B", ())]
        )
        .validate(),
        Err(UiError::InvalidPopup)
    );
    assert_eq!(
        View::<()>::menu_bar("b", vec![View::button("wrong", "Wrong", ())]).validate(),
        Err(UiError::InvalidPopup)
    );
    let a = app();
    assert_eq!(a.view().children()[0].accessibility_role(), Some("menubar"));
    let p = &a.view().children()[1];
    assert_eq!(p.accessibility_role(), Some("combobox"));
    assert_eq!(p.accessibility_has_popup(), Some("listbox"));
    assert_eq!(p.accessibility_label(), Some("Language"));
    assert_eq!(a.accessibility_expanded("pick"), Some(false));
    assert_eq!(
        a.view().children()[2].accessibility_description(),
        Some("Delayed help that wraps in a narrow viewport")
    );
}
#[test]
fn select_is_controlled_skips_disabled_and_closes_without_duplicate_callbacks() {
    let mut a = app();
    click(&mut a, "pick");
    assert_eq!(a.accessibility_expanded("pick"), Some(true));
    assert_eq!(a.popup_items(&[]).unwrap()[0].role, "option");
    a.popup_command(K::Down).unwrap();
    assert_eq!(a.popup_state().unwrap().path, vec![2]);
    a.popup_command(K::Activate).unwrap();
    assert_eq!(a.component().selected, "b");
    assert_eq!(a.component().calls, 1);
    assert_eq!(a.focused_key(), Some("pick"));
    click(&mut a, "pick");
    a.popup_command(K::Activate).unwrap();
    assert_eq!(a.component().calls, 1);
    a.update(Msg::Reject).unwrap();
    click(&mut a, "pick");
    a.popup_command(K::End).unwrap();
    a.popup_command(K::Activate).unwrap();
    assert_eq!(a.component().selected, "b");
    assert_eq!(a.view().children()[1].selected_option(), Some("b"));
    assert_eq!(a.component().calls, 2);
}
#[test]
fn menus_support_separators_nested_navigation_and_focus_restore() {
    let mut a = app();
    a.key(Key {
        character: 'f',
        modifiers: 4,
    })
    .unwrap();
    assert_eq!(a.popup_state().unwrap().owner, "file");
    a.popup_command(K::Down).unwrap();
    assert_eq!(a.popup_state().unwrap().path, vec![3]);
    a.popup_command(K::Right).unwrap();
    assert_eq!(a.popup_state().unwrap().path, vec![3, 0]);
    a.popup_command(K::Down).unwrap();
    assert_eq!(a.popup_state().unwrap().path, vec![3, 2]);
    a.popup_command(K::Escape).unwrap();
    assert_eq!(a.popup_state().unwrap().path, vec![3]);
    a.popup_command(K::Right).unwrap();
    a.popup_command(K::End).unwrap();
    a.popup_command(K::Activate).unwrap();
    assert_eq!(a.component().actions, vec!["two"]);
    assert_eq!(a.focused_key(), Some("file"));
}
#[test]
fn menu_bar_f10_arrows_hover_and_mnemonics_share_the_same_controller() {
    let mut a = app();
    a.popup_command(K::MenuBar).unwrap();
    assert_eq!(a.focused_key(), Some("file"));
    assert!(a.popup_state().is_none());
    a.popup_command(K::Right).unwrap();
    assert_eq!(a.focused_key(), Some("edit"));
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("pick"));
    a.key(Key {
        character: '\t',
        modifiers: 1,
    })
    .unwrap();
    assert_eq!(a.focused_key(), Some("edit"));
    a.key(Key::plain('\n')).unwrap();
    assert_eq!(a.popup_state().unwrap().owner, "edit");
    a.popup_command(K::Left).unwrap();
    assert_eq!(a.popup_state().unwrap().owner, "file");
    a.popup_pointer(P::Move, None, Some("edit")).unwrap();
    assert_eq!(a.popup_state().unwrap().owner, "edit");
    a.popup_command(K::Escape).unwrap();
    a.key(Key {
        character: 'f',
        modifiers: 4,
    })
    .unwrap();
    a.key(Key::plain('o')).unwrap();
    assert_eq!(a.component().actions, vec!["open"]);
}
#[test]
fn pointer_requires_matching_press_release_and_outside_click_has_no_click_through() {
    let mut a = app();
    click(&mut a, "file");
    a.popup_pointer(P::Down, hit("file", &[0]), None).unwrap();
    a.popup_pointer(P::Move, hit("file", &[4]), None).unwrap();
    a.popup_pointer(P::Up, hit("file", &[0]), None).unwrap();
    assert!(a.component().actions.is_empty());
    a.popup_pointer(P::Down, hit("file", &[2]), None).unwrap();
    a.popup_pointer(P::Up, hit("file", &[2]), None).unwrap();
    assert!(a.component().actions.is_empty());
    a.popup_pointer(P::Down, None, Some("apply")).unwrap();
    assert!(a.popup_state().is_none());
    a.pointer(P::Up, Some("apply")).unwrap();
    assert!(a.component().actions.is_empty());
    click(&mut a, "apply");
    assert_eq!(a.component().actions, vec!["apply"]);
    click(&mut a, "file");
    a.popup_pointer(P::Down, hit("file", &[0]), None).unwrap();
    a.popup_pointer(P::Up, hit("file", &[0]), None).unwrap();
    assert_eq!(a.component().actions, vec!["apply", "open"]);
}
#[test]
fn pointer_hover_opens_submenus_and_leave_cancels_a_press() {
    let mut a = app();
    click(&mut a, "file");
    a.popup_pointer(P::Move, hit("file", &[3]), None).unwrap();
    assert_eq!(a.popup_state().unwrap().path, vec![3, 0]);
    a.popup_pointer(P::Down, hit("file", &[3, 0]), None)
        .unwrap();
    a.popup_pointer(P::Leave, None, None).unwrap();
    a.popup_pointer(P::Up, hit("file", &[3, 0]), None).unwrap();
    assert!(a.component().actions.is_empty());
    a.popup_pointer(P::Down, hit("file", &[3, 2]), None)
        .unwrap();
    a.popup_pointer(P::Up, hit("file", &[3, 2]), None).unwrap();
    assert_eq!(a.component().actions, vec!["two"]);
}
#[test]
fn context_menus_restore_prior_focus_and_capture_keyboard_before_input() {
    let mut a = app();
    click(&mut a, "input");
    assert!(a.open_context_menu("input", Some((290., 230.))));
    assert_eq!(a.popup_state().unwrap().point, Some((290., 230.)));
    a.key(Key::plain('x')).unwrap();
    assert_eq!(a.component().text, "Text");
    a.popup_command(K::Activate).unwrap();
    assert_eq!(a.component().actions, vec!["ctx"]);
    assert_eq!(a.focused_key(), Some("input"));
    assert!(a.open_context_menu("help", None));
    a.popup_command(K::Escape).unwrap();
    assert_eq!(a.focused_key(), Some("input"));
    assert!(!a.open_context_menu("none", None));
    assert!(!a.open_context_menu("help", Some((f32::NAN, 0.))));
}
#[test]
fn rebuild_removal_disable_and_options_changes_cancel_stale_popup_state() {
    for change in [Msg::Disable, Msg::Change, Msg::Hide] {
        let mut a = app();
        click(&mut a, "pick");
        a.popup_pointer(P::Down, hit("pick", &[2]), None).unwrap();
        a.update(change).unwrap();
        assert!(a.popup_state().is_none());
        a.popup_pointer(P::Up, hit("pick", &[2]), None).unwrap();
        assert_eq!(a.component().calls, 0);
    }
    let mut a = app();
    click(&mut a, "pick");
    a.popup_command(K::Down).unwrap();
    a.update(Msg::Noop).unwrap();
    assert_eq!(a.popup_state().unwrap().path, vec![2]);
    a.key(Key::plain('\t')).unwrap();
    assert!(a.popup_state().is_none());
    assert_eq!(a.focused_key(), Some("help"));
}
#[test]
fn overlay_pixels_damage_and_closing_match_a_fresh_scene_in_both_themes() {
    for profile in [DARK, LIGHT] {
        let mut a = app();
        let mut r = renderer(profile, "");
        r.render_application(&a, 360, 280).unwrap();
        let base = r.pixels().to_vec();
        click(&mut a, "pick");
        assert!(r.render_application(&a, 360, 280).unwrap().damage.is_some());
        assert_ne!(base, r.pixels());
        let row = r.popup_item_rect("pick", &[2]).unwrap();
        assert_eq!(
            r.popup_hit(row.x + row.w / 2., row.y + row.h / 2.),
            hit("pick", &[2])
        );
        let mut fresh = renderer(profile, "");
        fresh.render_application(&a, 360, 280).unwrap();
        assert_eq!(fresh.pixels(), r.pixels());
        assert_eq!(r.render_application(&a, 360, 280).unwrap().damage, None);
        a.popup_command(K::Escape).unwrap();
        r.render_application(&a, 360, 280).unwrap();
        let mut fresh = renderer(profile, "");
        fresh.render_application(&a, 360, 280).unwrap();
        assert_eq!(fresh.pixels(), r.pixels());
        assert!(a.open_context_menu("help", Some((359., 279.))));
        r.render_application(&a, 360, 280).unwrap();
        let row = r.popup_item_rect("help", &[0]).unwrap();
        assert!(row.x >= 0. && row.y >= 0. && row.x + row.w <= 360. && row.y + row.h <= 280.);
        r.render_application(&a, 150, 100).unwrap();
        let row = r.popup_item_rect("help", &[0]).unwrap();
        assert!(row.x >= 0. && row.y >= 0. && row.x + row.w <= 150. && row.y + row.h <= 100.);
    }
}
#[test]
fn tall_select_scrolls_to_the_active_option_and_clips_outside_hits() {
    struct Many;
    impl Component for Many {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self) -> View<()> {
            View::select(
                "many",
                "Many",
                Some("0"),
                (0..100)
                    .map(|n| O::new(n.to_string(), format!("Option {n}")))
                    .collect(),
                |_| (),
            )
        }
    }
    let mut a = Application::new(Many).unwrap();
    a.key(Key::plain('\t')).unwrap();
    a.popup_command(K::Down).unwrap();
    a.popup_command(K::End).unwrap();
    let mut r = renderer(DARK, "");
    r.render_application(&a, 240, 120).unwrap();
    let rect = r.popup_item_rect("many", &[99]).unwrap();
    assert!(rect.y >= 0. && rect.y + rect.h <= 120.);
    assert!(r.popup_item_rect("many", &[0]).is_none());
    assert!(r.popup_hit(-1., -1.).is_none());
    a.popup_command(K::Home).unwrap();
    r.render_application(&a, 240, 120).unwrap();
    assert!(r.popup_item_rect("many", &[0]).is_some());
}
#[test]
fn tooltips_are_delayed_noninteractive_wrapped_and_cancelled_by_input() {
    let mut a = app();
    let mut r = renderer(DARK, "Tooltip#help {tooltip-max-width: 140;}");
    r.render_application(&a, 360, 280).unwrap();
    let before = r.pixels().to_vec();
    assert!(!a.tooltip_pointer(Some("help"), 100));
    let delay = r.tooltip_delay_ms("help").unwrap();
    assert_eq!(delay, 600);
    assert!(!a.sample_tooltip(699, delay));
    assert!(a.sample_tooltip(700, delay));
    assert_eq!(a.visible_tooltip(), Some("help"));
    r.render_application(&a, 360, 280).unwrap();
    assert_ne!(before, r.pixels());
    assert!(r.popup_hit(100., 150.).is_none());
    let mut fresh = renderer(DARK, "Tooltip#help {tooltip-max-width: 140;}");
    fresh.render_application(&a, 360, 280).unwrap();
    assert_eq!(fresh.pixels(), r.pixels());
    a.key(Key::plain('\t')).unwrap();
    assert!(a.visible_tooltip().is_none());
    assert!(a.tooltip_candidate().is_none());
    a.tooltip_pointer(Some("help"), 800);
    assert!(!a.tooltip_pointer(Some("help"), 900));
    assert_eq!(a.tooltip_candidate(), Some(("help", 800)));
    assert!(!a.tooltip_pointer(None, 950));
    assert!(!a.sample_tooltip(2000, delay));
    a.tooltip_pointer(Some("help"), 2100);
    a.sample_tooltip(2700, delay);
    click(&mut a, "file");
    assert!(a.visible_tooltip().is_none());
    assert!(a.tooltip_candidate().is_none());
}
#[test]
fn placement_and_theme_properties_are_validated() {
    use dunit_widgets::overlay::{place, Placement};
    let bounds = dunit_component::Rect {
        x: 0.,
        y: 0.,
        w: 100.,
        h: 80.,
    };
    let anchor = dunit_component::Rect {
        x: 90.,
        y: 70.,
        w: 10.,
        h: 10.,
    };
    let below = place(anchor, 60., 40., bounds, Placement::Below, 0.);
    assert_eq!(
        below,
        dunit_component::Rect {
            x: 40.,
            y: 30.,
            w: 60.,
            h: 40.
        }
    );
    let side = place(anchor, 60., 40., bounds, Placement::Side, 0.);
    assert_eq!(side.x, 30.);
    for overlay in [
        "Popup {menu-min-width: -1;}",
        "MenuItem {menu-row-height: red;}",
        "Button {tooltip-delay: -1ms;}",
        "Tooltip {tooltip-max-width: -1;}",
    ] {
        assert!(Theme::parse(DARK, BASE, overlay).is_err());
    }
}

#[test]
fn empty_and_all_disabled_submenus_remain_visible_without_dispatching() {
    struct Empty(u32);
    impl Component for Empty {
        type Message = ();
        fn update(&mut self, _: ()) {
            self.0 += 1;
        }
        fn view(&self) -> View<()> {
            View::menu(
                "menu",
                "Menu",
                vec![
                    I::submenu(
                        "sub",
                        "Submenu",
                        vec![I::action("locked", "Locked", ()).disabled(true)],
                    ),
                    I::submenu("empty", "Empty", vec![]),
                ],
            )
        }
    }
    let mut a = Application::new(Empty(0)).unwrap();
    a.key(Key::plain('\t')).unwrap();
    a.key(Key::plain('\n')).unwrap();
    a.popup_command(K::Right).unwrap();
    let mut r = renderer(DARK, "");
    r.render_application(&a, 300, 200).unwrap();
    assert!(r.popup_item_rect("menu", &[0, 0]).is_some());
    a.popup_command(K::Activate).unwrap();
    assert_eq!(a.component().0, 0);
    assert!(a.popup_state().is_some());
    a.popup_command(K::Escape).unwrap();
    a.popup_command(K::Down).unwrap();
    a.popup_command(K::Right).unwrap();
    r.render_application(&a, 300, 200).unwrap();
    a.popup_command(K::Activate).unwrap();
    assert_eq!(a.component().0, 0);
    a.popup_command(K::Escape).unwrap();
    a.popup_command(K::Escape).unwrap();
    assert!(a.popup_state().is_none());
    assert_eq!(a.focused_key(), Some("menu"));
}

#[test]
fn tooltip_rebuilds_cancel_removed_disabled_or_changed_anchors() {
    struct Tips {
        show: bool,
        disabled: bool,
        text: String,
    }
    impl Component for Tips {
        type Message = Msg;
        fn update(&mut self, m: Msg) {
            match m {
                Msg::Hide => self.show = false,
                Msg::Disable => self.disabled = true,
                Msg::Change => self.text = "New text".into(),
                _ => {}
            }
        }
        fn view(&self) -> View<Msg> {
            View::column(
                "root",
                if self.show {
                    vec![View::button("tip", "Help", Msg::Noop)
                        .tooltip(self.text.clone())
                        .disabled(self.disabled)]
                } else {
                    vec![]
                },
            )
        }
    }
    for change in [Msg::Hide, Msg::Disable, Msg::Change] {
        let mut a = Application::new(Tips {
            show: true,
            disabled: false,
            text: "Help text".into(),
        })
        .unwrap();
        a.tooltip_pointer(Some("tip"), 100);
        assert!(a.sample_tooltip(700, 600));
        a.update(Msg::Noop).unwrap();
        assert_eq!(a.visible_tooltip(), Some("tip"));
        a.update(change).unwrap();
        assert!(a.visible_tooltip().is_none());
        assert!(a.tooltip_candidate().is_none());
    }
    assert_eq!(
        View::<()>::button("none", "Help", ())
            .tooltip("   ")
            .accessibility_description(),
        None
    );
    assert_eq!(
        View::<()>::menu_bar(
            "bar",
            vec![
                View::menu("a", "A", vec![]).menu_mnemonic('f'),
                View::menu("b", "B", vec![]).menu_mnemonic('f')
            ]
        )
        .validate(),
        Err(UiError::InvalidPopup)
    );
}

#[test]
fn held_pointer_does_not_rearm_tooltips_after_a_control_press_cancels() {
    let mut a = app();
    a.pointer(P::Down, Some("help")).unwrap();
    a.pointer(P::Move, None).unwrap();
    assert!(!a.view().children()[2].interaction().pressed);
    a.tooltip_pointer(Some("help"), 100);
    assert!(a.tooltip_candidate().is_none());
    assert!(!a.sample_tooltip(1000, 600));
    a.pointer(P::Up, Some("help")).unwrap();
    assert!(a.component().actions.is_empty());
    a.tooltip_pointer(Some("help"), 1100);
    assert_eq!(a.tooltip_candidate(), Some(("help", 1100)));
    assert!(a.sample_tooltip(1700, 600));
    a.pointer(P::Leave, None).unwrap();
    assert!(a.visible_tooltip().is_none());
}
