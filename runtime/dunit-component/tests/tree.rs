use dunit_component::{
    Application, Attrs, Component, Font, Key, ListCommand as L, ListSelection, MenuItem,
    Navigation as N, Renderer, SelectionMode as Mode, Sizing, Theme, TreeCommand as K,
    TreeError as E, TreeItem as I, TreeModel as Model, TreeState as State, UiError, View,
};
fn model() -> Model {
    Model::new(vec![
        I::new("root", "Workspace"),
        I::new("a", "Sources").under("root"),
        I::new("a1", "main.rs").under("a"),
        I::new("a2", "ui.rs").under("a"),
        I::new("blocked", "Disabled").under("root").disabled(true),
        I::new("b1", "Hidden disabled child").under("blocked"),
        I::new("b", "Assets").under("root"),
        I::new("b2", "font.ttf").under("b"),
        I::new("other", "Other workspace"),
    ])
    .unwrap()
}
#[derive(Clone)]
enum Msg {
    State(State),
    Reject,
    Disabled,
    Modal(bool),
    Reorder,
    Remove,
}
struct Demo {
    model: Model,
    state: State,
    reject: bool,
    disabled: bool,
    modal: bool,
    calls: usize,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::State(s) => {
                self.calls += 1;
                if !self.reject {
                    self.state = s;
                }
            }
            Msg::Reject => self.reject = true,
            Msg::Disabled => self.disabled = true,
            Msg::Modal(b) => self.modal = b,
            Msg::Reorder => {
                let mut items = self.model.items().to_vec();
                items.reverse();
                self.model = Model::new(items).unwrap();
            }
            Msg::Remove => {
                self.model = Model::new(vec![
                    I::new("root", "Workspace"),
                    I::new("other", "Other workspace"),
                ])
                .unwrap();
                self.state.retain(&self.model);
            }
        }
    }
    fn view(&self) -> View<Msg> {
        View::modal(
            "host",
            View::tree_view(
                "tree",
                "Project",
                self.model.clone(),
                self.state.clone(),
                Mode::Multiple,
                Msg::State,
            )
            .context_menu(vec![MenuItem::action("close", "Close", Msg::Modal(false))])
            .disabled(self.disabled)
            .layout(Attrs {
                width: Sizing::Grow(1.),
                height: Sizing::Grow(1.),
                ..Default::default()
            }),
            self.modal.then(|| {
                View::message_box(
                    "dialog",
                    "Dialog",
                    "Scope",
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
fn app() -> Application<Demo> {
    Application::new(Demo {
        model: model(),
        state: State::default(),
        reject: false,
        disabled: false,
        modal: false,
        calls: 0,
    })
    .unwrap()
}
fn focus(a: &mut Application<Demo>) {
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("tree"));
    a.list_command(L::First, 0, 1).unwrap();
}
fn node<'a>(v: &'a View<Msg>) -> &'a View<Msg> {
    &v.children()[0]
}
fn r(light: bool) -> Renderer {
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
#[test]
fn model_rejects_bad_graphs_and_depth_before_layout() {
    assert_eq!(Model::new(vec![I::new("", "x")]).unwrap_err(), E::EmptyKey);
    assert_eq!(
        Model::new(vec![I::new("a", "A"), I::new("a", "B")]).unwrap_err(),
        E::DuplicateKey
    );
    assert_eq!(
        Model::new(vec![I::new("a", "A").under("missing")]).unwrap_err(),
        E::MissingParent
    );
    for rows in [
        vec![I::new("a", "A").under("a")],
        vec![
            I::new("root", "Root"),
            I::new("a", "A").under("b"),
            I::new("b", "B").under("a"),
        ],
    ] {
        assert_eq!(Model::new(rows).unwrap_err(), E::Cycle);
    }
    let chain: Vec<_> = (0..65)
        .map(|i| {
            if i == 0 {
                I::new("r0", "R")
            } else {
                I::new(format!("r{i}"), "R").under(format!("r{}", i - 1))
            }
        })
        .collect();
    assert_eq!(Model::new(chain.clone()).unwrap_err(), E::DepthLimit);
    Model::new(chain[..64].to_vec()).unwrap();
}
#[test]
fn arbitrary_input_order_keeps_sibling_order_and_disabled_inheritance() {
    let m = Model::new(vec![
        I::new("c", "Child").under("p"),
        I::new("p", "Parent"),
        I::new("d", "Disabled").under("p").disabled(true),
        I::new("e", "Descendant").under("d"),
    ])
    .unwrap();
    let expanded = ["p".into(), "d".into()].into_iter().collect();
    let rows = m.visible(&expanded);
    assert_eq!(
        rows.iter()
            .map(|r| m.items()[r.index].item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["p", "c", "d", "e"]
    );
    assert!(m.item(m.index("e").unwrap()).unwrap().disabled);
    assert_eq!(rows[1].position, 1);
    assert_eq!(rows[1].siblings, 2);
    assert_eq!(rows[3].depth, 2);
}
#[test]
fn right_left_expand_enter_child_return_parent_and_collapse() {
    let mut a = app();
    focus(&mut a);
    a.navigate(N::Right).unwrap();
    assert!(a.component().state.expanded.contains("root"));
    a.navigate(N::Right).unwrap();
    assert_eq!(a.component().state.selection.current.as_deref(), Some("a"));
    a.tree_command(K::Expand).unwrap();
    a.navigate(N::Right).unwrap();
    assert_eq!(a.component().state.selection.current.as_deref(), Some("a1"));
    a.navigate(N::Left).unwrap();
    assert_eq!(a.component().state.selection.current.as_deref(), Some("a"));
    a.navigate(N::Left).unwrap();
    assert!(!a.component().state.expanded.contains("a"));
    assert!(node(a.view()).tree_item("a1").is_none());
}
#[test]
fn ranges_and_select_all_use_visible_enabled_rows_and_one_tab_stop() {
    let mut a = app();
    focus(&mut a);
    a.tree_command(K::Expand).unwrap();
    a.list_command(L::Next, 0, 1).unwrap();
    a.list_command(L::Next, 1, 1).unwrap();
    assert_eq!(a.component().state.selection.current.as_deref(), Some("b"));
    assert_eq!(a.component().state.selection.selected.len(), 2);
    a.key(Key {
        character: 'a',
        modifiers: 2,
    })
    .unwrap();
    assert_eq!(a.component().state.selection.selected.len(), 4);
    assert!(!a.component().state.selection.selected.contains("a1"));
    a.key(Key::plain('\t')).unwrap();
    assert_eq!(a.focused_key(), Some("tree"));
}
#[test]
fn disclosure_collapse_promotes_hidden_selected_current_and_prunes_ranges() {
    let mut a = app();
    focus(&mut a);
    a.tree_command(K::Expand).unwrap();
    a.list_command(L::Next, 0, 1).unwrap();
    a.tree_command(K::Expand).unwrap();
    a.list_command(L::Next, 0, 1).unwrap();
    a.list_command(L::Next, 1, 1).unwrap();
    assert!(a.tree_toggle_row("tree", 1).unwrap());
    assert_eq!(a.component().state.selection.current.as_deref(), Some("a"));
    assert_eq!(
        a.component()
            .state
            .selection
            .selected
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["a"]
    );
    assert!(!a.component().state.expanded.contains("a"));
}
#[test]
fn rejected_proposals_leave_expansion_selection_and_scroll_unchanged() {
    let mut a = app();
    focus(&mut a);
    a.update(Msg::Reject).unwrap();
    let before = a.component().state.clone();
    a.tree_command(K::Expand).unwrap();
    a.tree_toggle_row("tree", 0).unwrap();
    assert_eq!(a.component().state, before);
    assert_eq!(a.component().calls, 3);
}
#[test]
fn disabled_and_scoped_tree_cannot_capture_background_input() {
    let mut a = app();
    focus(&mut a);
    a.update(Msg::Modal(true)).unwrap();
    assert!(!a.tree_toggle_row("tree", 0).unwrap());
    assert!(!a.tree_command(K::Expand).unwrap());
    a.key(Key::plain('\u{1b}')).unwrap();
    assert_eq!(a.focused_key(), Some("tree"));
    assert!(a.open_context_menu("tree", None));
    assert!(!a.tree_command(K::Expand).unwrap());
    assert!(!a.tree_toggle_row("tree", 0).unwrap());
    a.close_popup();
    a.update(Msg::Disabled).unwrap();
    assert!(!a.tree_command(K::Expand).unwrap());
}
#[test]
fn metadata_covers_levels_siblings_expansion_selection_and_disabled() {
    let mut a = app();
    focus(&mut a);
    a.tree_command(K::Expand).unwrap();
    a.list_command(L::Next, 0, 1).unwrap();
    let tree = node(a.view());
    assert_eq!(tree.accessibility_role(), Some("tree"));
    assert_eq!(tree.accessibility_label(), Some("Project"));
    let row = tree.tree_item("a").unwrap();
    assert_eq!(row.role, "treeitem");
    assert_eq!((row.level, row.position, row.siblings), (2, 1, 3));
    assert_eq!(row.expanded, Some(false));
    assert!(row.current && row.selected);
    assert!(tree.tree_item("blocked").unwrap().disabled);
}
#[test]
fn external_model_changes_preserve_keys_and_retain_drops_stale_state() {
    let mut a = app();
    focus(&mut a);
    a.tree_command(K::Expand).unwrap();
    a.list_command(L::Next, 0, 1).unwrap();
    a.tree_command(K::Expand).unwrap();
    a.update(Msg::Reorder).unwrap();
    assert_eq!(a.component().state.selection.current.as_deref(), Some("a"));
    a.update(Msg::Remove).unwrap();
    assert!(a.component().state.selection.current.is_none());
    assert!(!a.component().state.expanded.contains("a"));
    assert!(!a.component().state.expanded.contains("root"));
}
#[test]
fn invalid_expansion_and_hidden_selection_fail_transactional_validation() {
    for state in [
        State {
            expanded: ["missing".into()].into_iter().collect(),
            ..Default::default()
        },
        State {
            expanded: ["a1".into()].into_iter().collect(),
            ..Default::default()
        },
        State {
            selection: ListSelection {
                current: Some("a1".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    ] {
        assert_eq!(
            View::tree_view("t", "T", model(), state, Mode::Single, Msg::State).validate(),
            Err(UiError::InvalidTree)
        );
    }
}
#[test]
fn ten_thousand_descendants_virtualize_reveal_and_collapse_clamps() {
    let mut rows = vec![I::new("root", "Root")];
    rows.extend((0..10000).map(|i| I::new(format!("r{i}"), format!("Node {i}")).under("root")));
    let mut a = Application::new(Demo {
        model: Model::new(rows).unwrap(),
        state: State::default(),
        reject: false,
        disabled: false,
        modal: false,
        calls: 0,
    })
    .unwrap();
    focus(&mut a);
    let mut renderer = r(false);
    renderer.render(a.view(), 320, 220).unwrap();
    assert_eq!(renderer.list_visible_rows("tree"), Some(1));
    a.tree_command(K::Expand).unwrap();
    a.list_command(L::Last, 0, 1).unwrap();
    renderer.render(a.view(), 320, 220).unwrap();
    assert!(renderer
        .list_geometry("tree")
        .unwrap()
        .visible()
        .contains(&10000));
    assert!(renderer.list_visible_rows("tree").unwrap() < 10);
    assert!(a.tree_toggle_row("tree", 0).unwrap());
    renderer.render(a.view(), 320, 220).unwrap();
    assert_eq!(renderer.scroll_offset("tree"), Some(0.));
    assert_eq!(renderer.list_visible_rows("tree"), Some(1));
    assert_eq!(
        a.component().state.selection.current.as_deref(),
        Some("root")
    );
}
#[test]
fn disclosure_hit_geometry_retained_pixels_resize_and_both_themes() {
    for light in [false, true] {
        let mut a = app();
        focus(&mut a);
        let mut renderer = r(light);
        renderer.render(a.view(), 320, 220).unwrap();
        let g = renderer.list_geometry("tree").unwrap();
        assert_eq!(
            renderer.tree_toggle_at("tree", g.viewport.x + 5., g.viewport.y + 10.),
            Some(0)
        );
        assert_eq!(
            renderer.tree_toggle_at("tree", g.viewport.x + 60., g.viewport.y + 10.),
            None
        );
        a.tree_toggle_row("tree", 0).unwrap();
        assert!(renderer
            .render(a.view(), 320, 220)
            .unwrap()
            .damage
            .is_some());
        let mut fresh = r(light);
        fresh.render(a.view(), 320, 220).unwrap();
        assert_eq!(renderer.pixels(), fresh.pixels());
        let g = renderer.list_geometry("tree").unwrap();
        assert_eq!(
            renderer.tree_toggle_at(
                "tree",
                g.viewport.x + 25.,
                g.viewport.y + g.row_height + 10.
            ),
            Some(1)
        );
        assert_eq!(
            renderer.tree_toggle_at("tree", g.viewport.x + 5., g.viewport.y + g.row_height + 10.),
            None
        );
        for (w, h) in [(600, 400), (12, 12), (320, 220)] {
            renderer.render(a.view(), w, h).unwrap();
            assert!(renderer.list_visible_rows("tree").unwrap() <= 8);
            let mut fresh = r(light);
            fresh.render(a.view(), w, h).unwrap();
            assert_eq!(renderer.pixels(), fresh.pixels());
        }
    }
}

#[test]
fn body_selection_does_not_expand_and_disabled_descendants_are_skipped() {
    let mut a = app();
    focus(&mut a);
    a.tree_command(K::Expand).unwrap();
    a.list_pointer(
        dunit_component::PointerEvent::Down,
        Some("tree"),
        Some(1),
        0,
    )
    .unwrap();
    assert_eq!(a.component().state.selection.current.as_deref(), Some("a"));
    assert!(!a.component().state.expanded.contains("a"));
    let mut state = a.component().state.clone();
    state.expanded.insert("blocked".into());
    a.update(Msg::State(state)).unwrap();
    a.list_command(L::Next, 0, 1).unwrap();
    assert_eq!(a.component().state.selection.current.as_deref(), Some("b"));
    assert!(node(a.view()).tree_item("b1").unwrap().disabled);
}
#[test]
fn empty_and_leaf_trees_consume_spatial_navigation_without_expansion_messages() {
    for model in [
        Model::new(vec![]).unwrap(),
        Model::new(vec![I::new("leaf", "Leaf")]).unwrap(),
    ] {
        let mut a = Application::new(Demo {
            model,
            state: State::default(),
            reject: false,
            disabled: false,
            modal: false,
            calls: 0,
        })
        .unwrap();
        a.key(Key::plain('\t')).unwrap();
        a.list_command(L::First, 0, 1).unwrap();
        let calls = a.component().calls;
        for command in [K::Expand, K::Collapse, K::Toggle] {
            assert!(a.tree_command(command).unwrap());
        }
        assert_eq!(a.component().calls, calls);
        assert!(!a.tree_toggle_row("tree", 0).unwrap());
    }
}
