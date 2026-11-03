use dunit_component::{
    Application, Attrs, Component, EditCommand as E, Font, Key, ListCommand as L, RenameEvent as R,
    RenameState, Renderer, SelectionMode, Sizing, SortOrder, TableColumn as C, TableCommand as K,
    TableModel as Model, TableRow as Row, TableSort, TableState as State, TableValue as V, Theme,
    UiError, View,
};
#[derive(Clone)]
enum Msg {
    State(State),
    Rename(R),
    Reject,
    Disable,
    Modal(bool),
    Remove,
}
struct Demo {
    model: Model,
    state: State,
    edit: Option<RenameState>,
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
            Msg::Rename(e) => {
                self.calls += 1;
                match e {
                    R::Begin { row, column } => {
                        let r = self.model.rows().iter().find(|r| r.key == row).unwrap();
                        let c = column
                            .as_deref()
                            .and_then(|k| self.model.column(k))
                            .unwrap_or(0);
                        self.edit = Some(RenameState {
                            row,
                            column,
                            value: r.cells[c].text(),
                        });
                    }
                    R::Change(v) => {
                        if !self.reject {
                            self.edit.as_mut().unwrap().value = v;
                        }
                    }
                    R::Commit => {
                        if !self.reject {
                            let e = self.edit.take().unwrap();
                            let mut rows = self.model.rows().to_vec();
                            let r = rows.iter_mut().find(|r| r.key == e.row).unwrap();
                            let c = self.model.column(e.column.as_deref().unwrap()).unwrap();
                            r.cells[c] = V::Text(e.value);
                            self.model = Model::new(self.model.columns().to_vec(), rows).unwrap();
                        }
                    }
                    R::Cancel => self.edit = None,
                }
            }
            Msg::Reject => self.reject = true,
            Msg::Disable => self.disabled = true,
            Msg::Modal(b) => self.modal = b,
            Msg::Remove => {
                self.model = Model::new(self.model.columns().to_vec(), vec![]).unwrap();
                self.state.retain(&self.model);
                self.edit = None;
            }
        }
    }
    fn view(&self) -> View<Msg> {
        View::modal(
            "host",
            View::table(
                "table",
                "Documents",
                self.model.clone(),
                self.state.clone(),
                SelectionMode::Multiple,
                Msg::State,
            )
            .renaming(self.edit.clone(), Msg::Rename)
            .disabled(self.disabled)
            .layout(Attrs {
                width: Sizing::Grow(1.),
                height: Sizing::Grow(1.),
                ..Default::default()
            }),
            self.modal.then(|| {
                View::message_box(
                    "dialog",
                    "Scope",
                    "Modal",
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
fn app(n: usize) -> Application<Demo> {
    Application::new(Demo {
        model: Model::new(
            vec![
                C::new("name", "Name").width(240.).editable(true),
                C::new("size", "Size").width(180.),
                C::new("type", "Type").width(180.).sortable(false),
            ],
            (0..n)
                .map(|i| {
                    Row::new(
                        format!("r{i}"),
                        vec![
                            format!("Document {i}").into(),
                            (n as i64 - i as i64).into(),
                            "File".into(),
                        ],
                    )
                    .disabled(i == 2)
                })
                .collect(),
        )
        .unwrap(),
        state: State::default(),
        edit: None,
        reject: false,
        disabled: false,
        modal: false,
        calls: 0,
    })
    .unwrap()
}
fn focus(a: &mut Application<Demo>) {
    a.key(Key::plain('\t')).unwrap();
    a.list_command(L::First, 0, 1).unwrap();
    assert_eq!(a.focused_key(), Some("table"));
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
fn node(a: &Application<Demo>) -> &View<Msg> {
    &a.view().children()[0]
}
#[test]
fn validates_model_keys_widths_and_cell_counts() {
    assert!(Model::new(vec![], vec![]).is_err());
    for width in [0., -1., f32::NAN, f32::INFINITY] {
        assert!(Model::new(vec![C::new("a", "A").width(width)], vec![]).is_err());
    }
    assert!(Model::new(vec![C::new("a", "A"), C::new("a", "A")], vec![]).is_err());
    assert!(Model::new(vec![C::new("a", "A")], vec![Row::new("x", vec![])]).is_err());
    assert!(Model::new(
        vec![C::new("a", "A")],
        vec![
            Row::new("x", vec!["A".into()]),
            Row::new("x", vec!["B".into()])
        ]
    )
    .is_err());
}
#[test]
fn stable_typed_sort_handles_i64_extremes_and_equal_values() {
    let m = Model::new(
        vec![C::new("v", "Value")],
        vec![
            Row::new("a", vec![V::Integer(i64::MAX)]),
            Row::new("b", vec![V::Integer(i64::MIN)]),
            Row::new("c", vec![V::Integer(i64::MAX)]),
        ],
    )
    .unwrap();
    assert_eq!(
        m.order(Some(&TableSort {
            column: "v".into(),
            order: SortOrder::Ascending
        })),
        Some(vec![1, 0, 2])
    );
    assert_eq!(
        m.order(Some(&TableSort {
            column: "v".into(),
            order: SortOrder::Descending
        })),
        Some(vec![0, 2, 1])
    );
}
#[test]
fn sort_preserves_key_selection_and_reorders_navigation() {
    let mut a = app(6);
    focus(&mut a);
    a.table_header("table", 1).unwrap();
    assert_eq!(a.component().state.selection.current.as_deref(), Some("r0"));
    assert_eq!(node(&a).list_items().unwrap()[0].key, "r5");
    a.list_command(L::First, 0, 1).unwrap();
    assert_eq!(a.component().state.selection.current.as_deref(), Some("r5"));
    a.table_header("table", 1).unwrap();
    assert_eq!(node(&a).list_items().unwrap()[0].key, "r0");
}
#[test]
fn columns_keyboard_sort_and_nonsortable_header() {
    let mut a = app(8);
    focus(&mut a);
    a.table_command(K::NextColumn).unwrap();
    assert_eq!(a.component().state.column.as_deref(), Some("size"));
    a.table_command(K::ToggleSort).unwrap();
    assert_eq!(
        a.component().state.sort.as_ref().unwrap().order,
        SortOrder::Ascending
    );
    a.table_command(K::ToggleSort).unwrap();
    assert_eq!(
        a.component().state.sort.as_ref().unwrap().order,
        SortOrder::Descending
    );
    a.table_command(K::NextColumn).unwrap();
    let before = a.component().state.sort.clone();
    assert!(!a.table_header("table", 2).unwrap());
    a.table_command(K::SortAscending).unwrap();
    assert_eq!(a.component().state.sort, before);
}
#[test]
fn pointer_selects_row_and_column_in_one_proposal_and_skips_disabled() {
    let mut a = app(8);
    a.table_pointer("table", 1, 1, 0).unwrap();
    assert_eq!(a.component().calls, 1);
    assert_eq!(a.component().state.column.as_deref(), Some("size"));
    assert_eq!(a.component().state.selection.current.as_deref(), Some("r1"));
    a.table_pointer("table", 3, 0, 1).unwrap();
    assert_eq!(a.component().state.selection.selected.len(), 2);
    let before = a.component().state.clone();
    a.table_pointer("table", 2, 0, 0).unwrap();
    assert_eq!(a.component().state, before);
}
#[test]
fn grid_cell_metadata_follows_sort_and_column() {
    let mut a = app(8);
    focus(&mut a);
    a.table_command(K::NextColumn).unwrap();
    let c = node(&a).table_cell("r0", "size").unwrap();
    assert_eq!(
        (c.role, c.label.as_str(), c.row_index, c.column_index),
        ("gridcell", "8", 1, 2)
    );
    assert!(c.current && c.selected && !c.editable);
    assert_eq!(node(&a).accessibility_role(), Some("grid"));
}
#[test]
fn rejects_invalid_sort_column_and_stale_selection() {
    let a = app(5);
    let model = a.component().model.clone();
    for s in [
        State {
            column: Some("bad".into()),
            ..Default::default()
        },
        State {
            sort: Some(TableSort {
                column: "type".into(),
                order: SortOrder::Ascending,
            }),
            ..Default::default()
        },
    ] {
        assert_eq!(
            View::table(
                "t",
                "T",
                model.clone(),
                s,
                SelectionMode::Single,
                Msg::State
            )
            .validate(),
            Err(UiError::InvalidTable)
        );
    }
}
#[test]
fn rename_selects_text_edits_commits_and_restores_collection_focus() {
    let mut a = app(5);
    focus(&mut a);
    assert!(a.begin_rename().unwrap());
    assert!(a.focused_key().unwrap().contains("/rename/"));
    let mut renderer = r(false);
    renderer.render(a.view(), 320, 220).unwrap();
    assert!(a.key(Key::plain('x')).unwrap());
    assert_eq!(a.component().edit.as_ref().unwrap().value, "x");
    a.key(Key::plain('\r')).unwrap();
    assert!(a.component().edit.is_none());
    assert_eq!(a.component().model.rows()[0].cells[0].text(), "x");
    assert_eq!(a.focused_key(), Some("table"));
}
#[test]
fn rejected_draft_preserves_text_and_cancel_discards_it() {
    let mut a = app(5);
    focus(&mut a);
    a.begin_rename().unwrap();
    a.update(Msg::Reject).unwrap();
    let before = a.component().edit.clone();
    a.key(Key::plain('x')).unwrap();
    assert_eq!(a.component().edit, before);
    a.key(Key::plain('\r')).unwrap();
    assert!(a.component().edit.is_some());
    a.key(Key::plain('\u{1b}')).unwrap();
    assert!(a.component().edit.is_none());
    assert_eq!(a.focused_key(), Some("table"));
}
#[test]
fn noneditable_column_cannot_begin_editor_and_disable_stops_it() {
    let mut a = app(5);
    focus(&mut a);
    a.table_command(K::NextColumn).unwrap();
    assert!(a.begin_rename().unwrap());
    assert!(a.component().edit.is_none());
    a.table_command(K::PreviousColumn).unwrap();
    a.begin_rename().unwrap();
    a.update(Msg::Disable).unwrap();
    assert!(a.focused_input_key().is_none());
    assert!(!a.edit(E::Insert('x')).unwrap());
}
#[test]
fn header_and_cell_geometry_scroll_x_and_virtual_row_bound() {
    for light in [false, true] {
        let mut a = app(10000);
        focus(&mut a);
        let mut renderer = r(light);
        renderer.render(a.view(), 300, 200).unwrap();
        let g = renderer.list_geometry("table").unwrap();
        assert_eq!(
            renderer.table_header_at("table", g.viewport.x + 10., g.viewport.y - 10.),
            Some(0)
        );
        assert_eq!(
            renderer.table_cell_at("table", g.viewport.x + 10., g.viewport.y + 10.),
            Some((0, 0))
        );
        assert!(renderer.list_visible_rows("table").unwrap() < 8);
        a.table_command(K::NextColumn).unwrap();
        a.table_command(K::NextColumn).unwrap();
        renderer.render(a.view(), 300, 200).unwrap();
        assert!(renderer.scroll_offset_x("table").unwrap() > 0.);
        a.list_command(L::Last, 0, 1).unwrap();
        renderer.render(a.view(), 300, 200).unwrap();
        assert!(renderer
            .list_geometry("table")
            .unwrap()
            .visible()
            .contains(&9999));
        assert!(renderer.list_visible_rows("table").unwrap() < 8);
        a.update(Msg::Remove).unwrap();
        renderer.render(a.view(), 300, 200).unwrap();
        assert_eq!(renderer.scroll_offset("table"), Some(0.));
    }
}
#[test]
fn modal_and_rejection_do_not_leak_sort_or_cell_input() {
    let mut a = app(8);
    focus(&mut a);
    a.update(Msg::Modal(true)).unwrap();
    assert!(!a.table_header("table", 0).unwrap());
    assert!(!a.table_pointer("table", 0, 0, 0).unwrap());
    assert!(!a.table_command(K::NextColumn).unwrap());
    a.key(Key::plain('\u{1b}')).unwrap();
    a.update(Msg::Reject).unwrap();
    a.table_header("table", 1).unwrap();
    assert!(a.component().state.sort.is_none());
}
#[test]
fn retained_cells_headers_and_editor_pixels_match_fresh_after_resize() {
    for light in [false, true] {
        let mut a = app(10);
        focus(&mut a);
        let mut renderer = r(light);
        renderer.render(a.view(), 500, 300).unwrap();
        a.table_header("table", 0).unwrap();
        renderer.render(a.view(), 500, 300).unwrap();
        for (w, h) in [(500, 300), (12, 12), (500, 300)] {
            renderer.render(a.view(), w, h).unwrap();
            let mut fresh = r(light);
            fresh.render(a.view(), w, h).unwrap();
            assert_eq!(renderer.pixels(), fresh.pixels());
        }
        a.begin_rename().unwrap();
        renderer.render(a.view(), 500, 300).unwrap();
        let mut fresh = r(light);
        fresh.render(a.view(), 500, 300).unwrap();
        assert_eq!(renderer.pixels(), fresh.pixels());
    }
}

#[derive(Clone, Copy)]
enum Rewrite {
    Reorder,
    RemoveRow,
    RemoveColumn,
    Disable,
    Modal,
}
#[derive(Clone)]
enum RewriteMsg {
    Value(i64),
    State(State),
    Close,
}
struct RebuiltTable {
    rewrite: Rewrite,
    value: i64,
    state: State,
    calls: usize,
}
impl Component for RebuiltTable {
    type Message = RewriteMsg;
    fn update(&mut self, m: RewriteMsg) {
        match m {
            RewriteMsg::Value(v) => self.value = v,
            RewriteMsg::State(s) => {
                self.calls += 1;
                self.state = s;
            }
            RewriteMsg::Close => self.value = 0,
        }
    }
    fn view(&self) -> View<RewriteMsg> {
        let changed = self.value != 0;
        let mut columns = vec![C::new("name", "Name"), C::new("size", "Size")];
        let mut rows = vec![
            Row::new("a", vec!["A".into(), 1i64.into()]),
            Row::new("b", vec!["B".into(), 2i64.into()]),
        ];
        if changed {
            match self.rewrite {
                Rewrite::Reorder => {
                    columns.reverse();
                    rows.reverse();
                    for row in &mut rows {
                        row.cells.reverse();
                    }
                }
                Rewrite::RemoveRow => {
                    rows.remove(0);
                }
                Rewrite::RemoveColumn => {
                    columns.remove(0);
                    for row in &mut rows {
                        row.cells.remove(0);
                    }
                }
                Rewrite::Disable | Rewrite::Modal => {}
            }
        }
        let model = Model::new(columns, rows).unwrap();
        let mut state = self.state.clone();
        state.retain(&model);
        View::modal(
            "host",
            View::column(
                "body",
                vec![
                    View::spinbox(
                        "spin",
                        "Rebuild",
                        self.value,
                        dunit_component::SpinboxOptions::new(0, 10, 1),
                        RewriteMsg::Value,
                    ),
                    View::table(
                        "table",
                        "Data",
                        model,
                        state,
                        SelectionMode::Single,
                        RewriteMsg::State,
                    )
                    .disabled(changed && matches!(self.rewrite, Rewrite::Disable)),
                ],
            ),
            (changed && matches!(self.rewrite, Rewrite::Modal)).then(|| {
                View::message_box(
                    "modal",
                    "Changed",
                    "Confirm",
                    vec![dunit_component::DialogAction::new(
                        "close",
                        "Close",
                        RewriteMsg::Close,
                    )],
                    Some("close"),
                    Some(RewriteMsg::Close),
                )
            }),
        )
    }
}
fn pending_rebuild(rewrite: Rewrite) -> Application<RebuiltTable> {
    let mut app = Application::new(RebuiltTable {
        rewrite,
        value: 0,
        state: State::default(),
        calls: 0,
    })
    .unwrap();
    app.key(Key::plain('\t')).unwrap();
    assert_eq!(app.focused_key(), Some("spin"));
    app.edit(E::SelectAll).unwrap();
    app.key(Key::plain('1')).unwrap();
    assert_eq!(app.component().value, 0);
    app
}
#[test]
fn pointer_preserves_cell_identity_when_spinbox_commit_rebuilds_table() {
    let mut app = pending_rebuild(Rewrite::Reorder);
    assert!(app.table_pointer("table", 0, 0, 0).unwrap());
    assert_eq!(app.component().value, 1);
    assert_eq!(app.component().calls, 1);
    assert_eq!(
        app.component().state.selection.current.as_deref(),
        Some("a")
    );
    assert_eq!(app.component().state.column.as_deref(), Some("name"));
}
#[test]
fn header_preserves_column_identity_when_spinbox_commit_reorders_columns() {
    let mut app = pending_rebuild(Rewrite::Reorder);
    assert!(app.table_header("table", 0).unwrap());
    assert_eq!(app.component().state.sort.as_ref().unwrap().column, "name");
    assert_eq!(app.component().calls, 1);
}
#[test]
fn stale_cell_click_cannot_select_replacement_or_escape_new_modal_scope() {
    for rewrite in [
        Rewrite::RemoveRow,
        Rewrite::RemoveColumn,
        Rewrite::Disable,
        Rewrite::Modal,
    ] {
        let mut app = pending_rebuild(rewrite);
        assert!(app.table_pointer("table", 0, 0, 0).unwrap());
        assert_eq!(app.component().value, 1);
        assert_eq!(app.component().calls, 0);
        assert_ne!(app.focused_key(), Some("table"));
    }
    for rewrite in [Rewrite::RemoveColumn, Rewrite::Disable, Rewrite::Modal] {
        let mut app = pending_rebuild(rewrite);
        assert!(app.table_header("table", 0).unwrap());
        assert_eq!(app.component().calls, 0);
        assert_ne!(app.focused_key(), Some("table"));
    }
}

#[test]
fn clipboard_exports_the_active_column_or_whole_row_as_tsv() {
    let mut a = app(4);
    focus(&mut a);
    assert_eq!(
        a.collection_clipboard_text().as_deref(),
        Some("Document 0\t4\tFile")
    );
    a.table_command(K::NextColumn).unwrap();
    assert_eq!(a.collection_clipboard_text().as_deref(), Some("4"));
    let mut state = a.component().state.clone();
    state.column = Some("name".into());
    a.update(Msg::State(state)).unwrap();
    let calls = a.component().calls;
    assert_eq!(a.collection_clipboard_text().as_deref(), Some("Document 0"));
    assert_eq!(a.component().calls, calls);
}
