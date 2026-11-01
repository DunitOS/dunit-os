//! Exercise reusable child views through public dispatch, including two nested
//! message maps with captured parent identities. Rejected controlled proposals
//! still have to reach the intended parent with their exact payload.
use dunit_component::*;
#[derive(Clone, Debug, PartialEq)]
enum Child {
    Text(String),
    Number(i64),
    Pick(String),
    List(ListSelection),
    Tree(TreeState),
    Table(TableState),
    Rename(RenameEvent),
    Action(&'static str),
    Route(EventPhase),
}
struct Parent {
    child: View<Child>,
    events: Vec<(usize, Child)>,
    scope: usize,
    offset: usize,
}
impl Component for Parent {
    type Message = (usize, Child);
    fn update(&mut self, m: Self::Message) {
        self.events.push(m);
    }
    fn view(&self) -> View<Self::Message> {
        let scope = self.scope;
        let offset = self.offset;
        self.child
            .clone()
            .map_messages(move |m| (scope, m))
            .map_messages(move |(n, m)| (n + offset, m))
    }
}
fn app(child: View<Child>) -> Application<Parent> {
    Application::new(Parent {
        child,
        events: vec![],
        scope: 41,
        offset: 7,
    })
    .unwrap()
}
fn focus(a: &mut Application<Parent>, key: &str) {
    a.pointer(PointerEvent::Down, Some(key)).unwrap();
    a.pointer(PointerEvent::Up, Some(key)).unwrap();
    assert_eq!(a.focused_key(), Some(key));
}
fn events(a: &Application<Parent>) -> Vec<Child> {
    assert!(a.component().events.iter().all(|(id, _)| *id == 48));
    a.component()
        .events
        .iter()
        .map(|(_, m)| m.clone())
        .collect()
}
#[test]
fn composed_input_submit_activation_and_root_shortcuts_reach_captured_parent() {
    let mut a = app(View::column(
        "root",
        vec![
            View::input(
                "input",
                "Query",
                "old",
                Child::Text,
                InputOptions::default(),
            )
            .on_submit(Child::Action("submit")),
            View::button("button", "Apply", Child::Action("button")),
        ],
    )
    .on_key(
        Key {
            character: 'k',
            modifiers: 2,
        },
        Child::Action("shortcut"),
    ));
    a.key(Key::plain('\t')).unwrap();
    a.key(Key::plain('x')).unwrap();
    a.key(Key::plain('\r')).unwrap();
    a.key(Key::plain('\t')).unwrap();
    a.key(Key::plain('\r')).unwrap();
    a.key(Key {
        character: 'k',
        modifiers: 2,
    })
    .unwrap();
    assert_eq!(
        events(&a),
        vec![
            Child::Text("oldx".into()),
            Child::Action("submit"),
            Child::Action("button"),
            Child::Action("shortcut")
        ]
    );
}
#[test]
fn composed_slider_spinbox_and_splitter_keep_numeric_callback_payloads() {
    let mut slider = app(View::slider(
        "slider",
        "Volume",
        5,
        SliderOptions::new(0, 10, 1),
        Child::Number,
    ));
    focus(&mut slider, "slider");
    slider.slider_command(SliderCommand::Increase).unwrap();
    assert_eq!(events(&slider), vec![Child::Number(6)]);
    let mut spin = app(View::spinbox(
        "spin",
        "Quantity",
        5,
        SpinboxOptions::new(0, 10, 1),
        Child::Number,
    ));
    focus(&mut spin, "spin");
    spin.edit(EditCommand::SelectAll).unwrap();
    spin.edit(EditCommand::Insert('8')).unwrap();
    assert!(events(&spin).is_empty());
    spin.commit_spinbox().unwrap();
    assert_eq!(events(&spin), vec![Child::Number(8)]);
    let mut split = app(View::splitter(
        "split",
        "Panes",
        500,
        SplitterOptions::new(SplitAxis::Horizontal),
        View::text("left", "Left"),
        View::text("right", "Right"),
        Child::Number,
    ));
    focus(&mut split, "split/handle");
    split.splitter_command(SliderCommand::Home).unwrap();
    assert_eq!(events(&split), vec![Child::Number(0)]);
}
#[test]
fn composed_list_tree_and_table_propose_their_full_controlled_state() {
    let mut list = app(View::list(
        "list",
        "Rows",
        vec![ListItem::new("a", "Alpha"), ListItem::new("b", "Beta")],
        ListSelection::default(),
        SelectionMode::Multiple,
        Child::List,
    ));
    focus(&mut list, "list");
    list.list_command(ListCommand::Last, 0, 1).unwrap();
    match events(&list).as_slice() {
        [Child::List(s)] => {
            assert_eq!(s.current.as_deref(), Some("b"));
            assert!(s.selected.contains("b"));
        }
        other => panic!("{other:?}"),
    }
    let mut tree = app(View::tree_view(
        "tree",
        "Project",
        TreeModel::new(vec![
            TreeItem::new("a", "Root"),
            TreeItem::new("b", "Child").under("a"),
        ])
        .unwrap(),
        TreeState::default(),
        SelectionMode::Single,
        Child::Tree,
    ));
    tree.tree_toggle_row("tree", 0).unwrap();
    match events(&tree).as_slice() {
        [Child::Tree(s)] => assert!(s.expanded.contains("a")),
        other => panic!("{other:?}"),
    }
    let mut table = app(View::table(
        "table",
        "Files",
        TableModel::new(
            vec![
                TableColumn::new("name", "Name"),
                TableColumn::new("size", "Size"),
            ],
            vec![
                TableRow::new("a", vec!["Alpha".into(), 1i64.into()]),
                TableRow::new("b", vec!["Beta".into(), 2i64.into()]),
            ],
        )
        .unwrap(),
        TableState::default(),
        SelectionMode::Single,
        Child::Table,
    ));
    table.table_pointer("table", 1, 1, 0).unwrap();
    table.table_header("table", 0).unwrap();
    match events(&table).as_slice() {
        [Child::Table(a), Child::Table(b)] => {
            assert_eq!(a.selection.current.as_deref(), Some("b"));
            assert_eq!(a.column.as_deref(), Some("size"));
            assert_eq!(b.sort.as_ref().unwrap().column, "name");
            assert_eq!(b.sort.as_ref().unwrap().order, SortOrder::Ascending);
        }
        other => panic!("{other:?}"),
    }
}
fn rename_view(edit: Option<RenameState>) -> View<Child> {
    let mut state = ListSelection::default();
    state.current = Some("a".into());
    state.selected.insert("a".into());
    View::list(
        "rows",
        "Documents",
        vec![ListItem::new("a", "Alpha")],
        state,
        SelectionMode::Single,
        Child::List,
    )
    .renaming(edit, Child::Rename)
}
#[test]
fn composed_rename_begin_generated_input_commit_and_cancel_are_mapped() {
    let mut begin = app(rename_view(None));
    focus(&mut begin, "rows");
    begin.begin_rename().unwrap();
    assert_eq!(
        events(&begin),
        vec![Child::Rename(RenameEvent::Begin {
            row: "a".into(),
            column: None
        })]
    );
    let mut edit = app(rename_view(Some(RenameState::new("a", "Alpha"))));
    assert_ne!(edit.focused_key(), Some("rows"));
    edit.key(Key::plain('z')).unwrap();
    edit.key(Key::plain('\r')).unwrap();
    edit.key(Key::plain('\u{1b}')).unwrap();
    assert_eq!(
        events(&edit),
        vec![
            Child::Rename(RenameEvent::Change("z".into())),
            Child::Rename(RenameEvent::Commit),
            Child::Rename(RenameEvent::Cancel)
        ]
    );
}
#[test]
fn composed_select_nested_menu_and_context_menu_keep_action_and_callback_routing() {
    let mut select = app(View::select(
        "select",
        "Language",
        Some("a"),
        vec![
            SelectOption::new("a", "Alpha"),
            SelectOption::new("b", "Beta"),
        ],
        Child::Pick,
    ));
    select.key(Key::plain('\t')).unwrap();
    select.popup_command(PopupCommand::Down).unwrap();
    select.popup_command(PopupCommand::End).unwrap();
    select.popup_command(PopupCommand::Activate).unwrap();
    assert_eq!(events(&select), vec![Child::Pick("b".into())]);
    let items = || {
        vec![MenuItem::submenu(
            "recent",
            "Recent",
            vec![MenuItem::action(
                "nested",
                "Document",
                Child::Action("nested"),
            )],
        )]
    };
    let mut menu = app(View::menu("menu", "Files", items()));
    menu.key(Key::plain('\t')).unwrap();
    menu.popup_command(PopupCommand::Down).unwrap();
    menu.popup_command(PopupCommand::Right).unwrap();
    menu.popup_command(PopupCommand::Activate).unwrap();
    assert_eq!(events(&menu), vec![Child::Action("nested")]);
    let mut context =
        app(View::button("context", "Context", Child::Action("button")).context_menu(items()));
    assert!(context.open_context_menu("context", None));
    context.popup_command(PopupCommand::Right).unwrap();
    context.popup_command(PopupCommand::Activate).unwrap();
    assert_eq!(events(&context), vec![Child::Action("nested")]);
}
#[test]
fn composed_dialog_default_and_cancel_survive_the_modal_focus_scope() {
    let mut dialog = app(View::dialog(
        "dialog",
        "Confirm",
        View::text("body", "Question"),
        vec![
            DialogAction::new("accept", "Accept", Child::Action("default")),
            DialogAction::new("cancel", "Cancel", Child::Action("cancel-button")),
        ],
        Some("accept"),
        Some(Child::Action("cancel")),
    ));
    dialog.key(Key::plain('\r')).unwrap();
    dialog.key(Key::plain('\u{1b}')).unwrap();
    assert_eq!(
        events(&dialog),
        vec![Child::Action("default"), Child::Action("cancel")]
    );
}
fn route_message(e: UiEvent) -> EventResponse<Child> {
    EventResponse::message(Child::Route(e.phase))
}
fn prevent_message(e: UiEvent) -> EventResponse<Child> {
    EventResponse::message(Child::Route(e.phase)).prevent()
}
#[test]
fn composed_capture_and_bubble_preserve_response_flags_and_phase_payloads() {
    let mut allowed = app(View::button("button", "Apply", Child::Action("button"))
        .on_capture(route_message)
        .on_bubble(route_message));
    allowed.key(Key::plain('\t')).unwrap();
    let before = allowed.component().events.len();
    allowed.key(Key::plain('\r')).unwrap();
    assert_eq!(
        &events(&allowed)[before..],
        &[
            Child::Route(EventPhase::Capture),
            Child::Route(EventPhase::Bubble),
            Child::Action("button")
        ]
    );
    let mut prevented = app(View::button("button", "Apply", Child::Action("button"))
        .on_capture(prevent_message)
        .on_bubble(route_message));
    let out = prevented
        .route_event(Some("button"), UiEventKind::Key(Key::plain('\r')))
        .unwrap();
    assert!(out.default_prevented);
    assert_eq!(
        events(&prevented),
        vec![
            Child::Route(EventPhase::Capture),
            Child::Route(EventPhase::Bubble)
        ]
    );
}
