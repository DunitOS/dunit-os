use super::*;
use core::ptr;
fn node(kind: u32, parent: u32, key: &str, text: &str, message: u64) -> DunitUiNodeV1 {
    DunitUiNodeV1 {
        kind,
        parent,
        key: key.as_ptr(),
        key_len: key.len(),
        text: text.as_ptr(),
        text_len: text.len(),
        message,
    }
}
fn item(key: &str, label: &str, parent: &str, flags: u32) -> DunitUiCollectionItemV1 {
    DunitUiCollectionItemV1 {
        key: key.as_ptr(),
        key_len: key.len(),
        label: label.as_ptr(),
        label_len: label.len(),
        parent: parent.as_ptr(),
        parent_len: parent.len(),
        flags,
        reserved: 0,
    }
}
#[derive(Clone)]
struct State {
    list: [u32; 3],
    tree: [u32; 2],
    rows: [u32; 2],
    column: bool,
    sort: bool,
    sort_column: bool,
    descending: bool,
    selected: bool,
    dialog: bool,
    changes: usize,
    split: i64,
    split_changes: usize,
    status: String,
    rename: Option<RenameState>,
    label: String,
    commits: usize,
}
impl Default for State {
    fn default() -> Self {
        Self {
            list: [0, 0, DUNIT_UI_ITEM_DISABLED],
            tree: [0; 2],
            rows: [0; 2],
            column: false,
            sort: false,
            sort_column: false,
            descending: false,
            selected: false,
            dialog: false,
            changes: 0,
            split: 50,
            split_changes: 0,
            status: String::new(),
            rename: None,
            label: "Alpha".into(),
            commits: 0,
        }
    }
}
fn nodes(s: &mut State) -> Vec<DunitUiNodeV1> {
    s.status = alloc::format!("Changes {} / split {}", s.changes, s.split);
    let mut n = vec![
        node(DUNIT_UI_MODAL, DUNIT_UI_ROOT, "host", "", 0),
        node(DUNIT_UI_COLUMN, 0, "body", "", 0),
        node(
            DUNIT_UI_TEXT,
            1,
            "title",
            "C SDK workspace + collections",
            0,
        ),
        node(DUNIT_UI_LIST, 1, "list", "Project list", 1),
        node(DUNIT_UI_TREE, 1, "tree", "Project tree", 2),
        node(DUNIT_UI_TABLE, 1, "table", "Files", 3),
        node(DUNIT_UI_TABS, 1, "tabs", "Pages", 0),
        node(
            if s.selected {
                DUNIT_UI_TAB
            } else {
                DUNIT_UI_TAB_SELECTED
            },
            6,
            "overview",
            "Overview",
            10,
        ),
        node(DUNIT_UI_TEXT, 7, "first", "The SDK owns tabs and focus", 0),
        node(
            if s.selected {
                DUNIT_UI_TAB_SELECTED
            } else {
                DUNIT_UI_TAB
            },
            6,
            "details",
            "Details",
            11,
        ),
        node(DUNIT_UI_SCROLL_BOTH, 9, "details-scroll", "Details", 0),
        node(DUNIT_UI_TOOLBAR, 1, "bar", "Actions", 0),
        node(DUNIT_UI_BUTTON, 11, "open", "About", 12),
        node(DUNIT_UI_TOOLBAR_SEPARATOR, 11, "separator", "", 0),
        node(DUNIT_UI_BUTTON, 11, "quit", "Quit", 14),
        node(DUNIT_UI_SPLITTER, 1, "split", "Panes", 9),
        node(DUNIT_UI_TEXT, 15, "left", "Left pane", 0),
        node(DUNIT_UI_TEXT, 15, "right", "Right pane", 0),
        node(DUNIT_UI_STATUS_BAR, 1, "status", &s.status, 0),
    ];
    if s.dialog {
        n.extend([
            node(
                DUNIT_UI_MESSAGE_BOX,
                0,
                "about",
                "One SDK, two languages",
                0,
            ),
            node(
                DUNIT_UI_MESSAGE_BODY,
                19,
                "message",
                "Same widgets. Same theme.\nCompiled by TCC inside Dunit.",
                0,
            ),
            node(DUNIT_UI_DIALOG_ACTION, 19, "dismiss", "Close", 13),
            node(DUNIT_UI_DIALOG_DEFAULT, 19, "default", "dismiss", 0),
            node(DUNIT_UI_DIALOG_CANCEL, 19, "cancel", "", 13),
        ]);
    }
    n
}
unsafe extern "C" fn build(user: *mut c_void, out: *mut DunitUiNodeV1, cap: u32) -> i32 {
    let n = nodes(&mut *(user as *mut State));
    assert!(n.len() <= cap as usize);
    ptr::copy_nonoverlapping(n.as_ptr(), out, n.len());
    n.len() as i32
}
unsafe extern "C" fn update(user: *mut c_void, id: u64) -> i32 {
    let s = &mut *(user as *mut State);
    match id {
        10 | 11 => s.selected = id == 11,
        12 => s.dialog = true,
        13 => s.dialog = false,
        14 => return 1,
        _ => panic!("unexpected action {id}"),
    };
    0
}
// Configure spans are copied before the next callback; these thread-local arrays
// emulate ordinary C-owned model storage, not runtime state or dispatch.
struct Data {
    items: Vec<DunitUiCollectionItemV1>,
    columns: Vec<DunitUiTableColumnV1>,
    cells: Vec<DunitUiTableCellV1>,
}
std::thread_local! {static DATA:core::cell::RefCell<Option<Data>>=const{core::cell::RefCell::new(None)};}
unsafe extern "C" fn configure(user: *mut c_void, index: u32, c: *mut DunitUiCollectionV1) -> i32 {
    let s = &*(user as *mut State);
    let c = &mut *c;
    let mut data = Data {
        items: Vec::new(),
        columns: Vec::new(),
        cells: Vec::new(),
    };
    match index {
        3 => {
            data.items = vec![
                item("a", &s.label, "", s.list[0]),
                item("b", "Beta", "", s.list[1]),
                item("c", "Disabled", "", s.list[2]),
            ];
            c.flags = DUNIT_UI_COLLECTION_MULTIPLE;
            RENAME.with(|slot| {
                *slot.borrow_mut() = s.rename.as_ref().map(|r| DunitUiRenameV1 {
                    kind: 0,
                    reserved: 0,
                    row: r.row.as_ptr(),
                    row_len: r.row.len(),
                    column: ptr::null(),
                    column_len: 0,
                    value: r.value.as_ptr(),
                    value_len: r.value.len(),
                });
                c.rename = slot
                    .borrow()
                    .as_ref()
                    .map_or(ptr::null(), |r| r as *const _);
            });
        }
        4 => {
            data.items = vec![
                item("folder", "Folder", "", s.tree[0]),
                item("child", "Child", "folder", s.tree[1]),
            ]
        }
        5 => {
            data.items = vec![
                item("small", "", "", s.rows[0]),
                item("large", "", "", s.rows[1]),
            ];
            for (key, label, width) in [("name", "Name", 260), ("size", "Size", 160)] {
                data.columns.push(DunitUiTableColumnV1 {
                    key: key.as_ptr(),
                    key_len: key.len(),
                    label: label.as_ptr(),
                    label_len: label.len(),
                    width,
                    flags: DUNIT_UI_COLUMN_SORTABLE,
                    reserved: 0,
                });
            }
            for (text, value, kind) in [
                ("Small", 0, DUNIT_UI_CELL_TEXT),
                ("", 2, DUNIT_UI_CELL_INTEGER),
                ("Large", 0, DUNIT_UI_CELL_TEXT),
                ("", 100, DUNIT_UI_CELL_UNSIGNED),
            ] {
                data.cells.push(DunitUiTableCellV1 {
                    text: text.as_ptr(),
                    text_len: text.len(),
                    value,
                    kind,
                    reserved: 0,
                });
            }
            if s.sort {
                c.sort = if s.sort_column {
                    b"size".as_ptr()
                } else {
                    b"name".as_ptr()
                };
                c.sort_len = 4;
            }
            if s.column {
                c.column = b"size".as_ptr();
                c.column_len = 4;
            }
            if s.descending {
                c.flags |= DUNIT_UI_COLLECTION_SORT_DESCENDING;
            }
        }
        _ => return invalid().0 as i32,
    }
    c.items = data.items.as_ptr();
    c.items_len = data.items.len();
    c.columns = data.columns.as_ptr();
    c.columns_len = data.columns.len();
    c.cells = data.cells.as_ptr();
    c.cells_len = data.cells.len();
    DATA.with(|slot| *slot.borrow_mut() = Some(data));
    0
}
unsafe extern "C" fn changed(user: *mut c_void, id: u64, c: *const DunitUiCollectionV1) -> i32 {
    let s = &mut *(user as *mut State);
    let c = &*c;
    let (target, keys): (&mut [u32], &[&str]) = match id {
        1 => (&mut s.list, &["a", "b", "c"]),
        2 => (&mut s.tree, &["folder", "child"]),
        3 => (&mut s.rows, &["small", "large"]),
        _ => panic!("unexpected collection id"),
    };
    for f in target.iter_mut() {
        *f &= DUNIT_UI_ITEM_DISABLED;
    }
    for i in core::slice::from_raw_parts(c.items, c.items_len) {
        let key = tree::text(i.key, i.key_len).unwrap();
        let pos = keys.iter().position(|k| *k == key).unwrap();
        target[pos] |= i.flags;
    }
    if id == 3 {
        s.column = tree::text(c.column, c.column_len).unwrap() == "size";
        s.sort = c.sort_len != 0;
        s.sort_column = tree::text(c.sort, c.sort_len).unwrap() == "size";
        s.descending = c.flags & DUNIT_UI_COLLECTION_SORT_DESCENDING != 0;
    }
    s.changes += 1;
    0
}
std::thread_local! {static RENAME:core::cell::RefCell<Option<DunitUiRenameV1>>=const{core::cell::RefCell::new(None)};}
unsafe extern "C" fn renamed(user: *mut c_void, id: u64, event: *const DunitUiRenameV1) -> i32 {
    assert_eq!(id, 1);
    let s = &mut *(user as *mut State);
    let e = &*event;
    match e.kind {
        DUNIT_UI_RENAME_BEGIN => {
            assert_eq!(tree::text(e.row, e.row_len).unwrap(), "a");
            s.rename = Some(RenameState::new("a", s.label.clone()));
        }
        DUNIT_UI_RENAME_CHANGE => {
            s.rename.as_mut().unwrap().value = tree::text(e.value, e.value_len).unwrap()
        }
        DUNIT_UI_RENAME_COMMIT => {
            s.label = s.rename.take().unwrap().value;
            s.commits += 1
        }
        DUNIT_UI_RENAME_CANCEL => s.rename = None,
        _ => panic!("invalid rename event"),
    };
    0
}
unsafe extern "C" fn number_config(user: *mut c_void, index: u32, n: *mut DunitUiNumberV1) -> i32 {
    assert_eq!(index, 15);
    let n = &mut *n;
    n.min = 0;
    n.max = 100;
    n.step = 10;
    n.page_step = 20;
    n.value = (*(user as *const State)).split;
    0
}
unsafe extern "C" fn number_changed(user: *mut c_void, id: u64, v: i64) -> i32 {
    assert_eq!(id, 9);
    let s = &mut *(user as *mut State);
    s.split = v;
    s.split_changes += 1;
    0
}
fn model(s: &mut State) -> Model {
    unsafe {
        Model::new_extended(
            DunitUiCallbacksV1 {
                abi_version: DUNIT_UI_ABI_VERSION,
                struct_size: core::mem::size_of::<DunitUiCallbacksV1>() as u32,
                user: s as *mut _ as *mut c_void,
                build: Some(build),
                update: Some(update),
            },
            None,
            None,
            Some(DunitUiNumbersV1 {
                abi_version: 1,
                struct_size: 24,
                configure: Some(number_config),
                changed: Some(number_changed),
            }),
            Some(DunitUiCollectionsV1 {
                abi_version: 1,
                struct_size: 32,
                configure: Some(configure),
                changed: Some(changed),
                renamed: Some(renamed),
            }),
        )
        .unwrap()
    }
}
fn selection(flags: &[u32], keys: &[&str]) -> ListSelection {
    let mut s = ListSelection::default();
    for (f, k) in flags.iter().zip(keys) {
        if f & DUNIT_UI_ITEM_SELECTED != 0 {
            s.selected.insert((*k).into());
        }
        if f & DUNIT_UI_ITEM_CURRENT != 0 {
            s.current = Some((*k).into());
        }
        if f & DUNIT_UI_ITEM_ANCHOR != 0 {
            s.anchor = Some((*k).into());
        }
    }
    s
}
struct Reference(State);
impl Component for Reference {
    type Message = Message;
    fn update(&mut self, m: Message) {
        let u = &mut self.0 as *mut _ as *mut c_void;
        unsafe {
            match m {
                Message::Rename(id, event) => {
                    collection::rename_dispatch(u, id, &event, renamed);
                }
                Message::Action(id) => {
                    update(u, id);
                }
                Message::Collection(id, s) => {
                    s.dispatch(u, id, changed);
                }
                Message::Number(id, v) => {
                    number_changed(u, id, v);
                }
                _ => panic!("unexpected text event"),
            }
        }
    }
    fn view(&self) -> View<Message> {
        let s = &self.0;
        let list = View::list(
            "list",
            "Project list",
            vec![
                ListItem::new("a", &s.label),
                ListItem::new("b", "Beta"),
                ListItem::new("c", "Disabled").disabled(true),
            ],
            selection(&s.list, &["a", "b", "c"]),
            SelectionMode::Multiple,
            |s| s,
        )
        .map_messages(|s| {
            Message::Collection(
                1,
                collection::State {
                    selection: s,
                    expanded: Default::default(),
                    sort: None,
                    column: None,
                },
            )
        });
        let list = list.renaming(s.rename.clone(), |e| Message::Rename(1, e));
        let tree = View::tree_view(
            "tree",
            "Project tree",
            TreeModel::new(vec![
                TreeItem::new("folder", "Folder"),
                TreeItem::new("child", "Child").under("folder"),
            ])
            .unwrap(),
            TreeState {
                selection: selection(&s.tree, &["folder", "child"]),
                expanded: if s.tree[0] & DUNIT_UI_ITEM_EXPANDED != 0 {
                    [String::from("folder")].into()
                } else {
                    Default::default()
                },
            },
            SelectionMode::Single,
            |s| s,
        )
        .map_messages(|s| {
            Message::Collection(
                2,
                collection::State {
                    selection: s.selection,
                    expanded: s.expanded,
                    sort: None,
                    column: None,
                },
            )
        });
        let table = View::table(
            "table",
            "Files",
            TableModel::new(
                vec![
                    TableColumn::new("name", "Name").width(260.),
                    TableColumn::new("size", "Size").width(160.),
                ],
                vec![
                    TableRow::new("small", vec!["Small".into(), TableValue::Integer(2)]),
                    TableRow::new("large", vec!["Large".into(), TableValue::Unsigned(100)]),
                ],
            )
            .unwrap(),
            TableState {
                selection: selection(&s.rows, &["small", "large"]),
                sort: if s.sort {
                    Some(TableSort {
                        column: if s.sort_column { "size" } else { "name" }.into(),
                        order: if s.descending {
                            SortOrder::Descending
                        } else {
                            SortOrder::Ascending
                        },
                    })
                } else {
                    None
                },
                column: if s.column { Some("size".into()) } else { None },
            },
            SelectionMode::Single,
            |s| s,
        )
        .map_messages(|s| {
            Message::Collection(
                3,
                collection::State {
                    selection: s.selection,
                    expanded: Default::default(),
                    sort: s.sort,
                    column: s.column,
                },
            )
        });
        let tabs = View::tabs(
            "tabs",
            "Pages",
            if s.selected { "details" } else { "overview" },
            vec![
                Tab::new(
                    "overview",
                    "Overview",
                    Message::Action(10),
                    View::text("first", "The SDK owns tabs and focus"),
                ),
                Tab::new(
                    "details",
                    "Details",
                    Message::Action(11),
                    View::scroll("details-scroll", "Details", vec![]).scroll_axes(ScrollAxes::Both),
                ),
            ],
        );
        let split = View::splitter(
            "split",
            "Panes",
            s.split,
            SplitterOptions {
                axis: SplitAxis::Horizontal,
                range: SliderOptions {
                    min: 0,
                    max: 100,
                    step: 10,
                    page_step: 20,
                },
            },
            View::text("left", "Left pane"),
            View::text("right", "Right pane"),
            |v| Message::Number(9, v),
        );
        View::modal(
            "host",
            View::column(
                "body",
                vec![
                    View::text("title", "C SDK workspace + collections"),
                    list,
                    tree,
                    table,
                    tabs,
                    View::toolbar(
                        "bar",
                        "Actions",
                        vec![
                            View::button("open", "About", Message::Action(12)),
                            View::toolbar_separator("separator"),
                            View::button("quit", "Quit", Message::Action(14)),
                        ],
                    ),
                    split,
                    View::status_bar(
                        "status",
                        "status",
                        alloc::format!("Changes {} / split {}", s.changes, s.split),
                    ),
                ],
            ),
            if s.dialog {
                Some(View::message_box(
                    "about",
                    "One SDK, two languages",
                    "Same widgets. Same theme.\nCompiled by TCC inside Dunit.",
                    vec![DialogAction::new("dismiss", "Close", Message::Action(13))],
                    Some("dismiss"),
                    Some(Message::Action(13)),
                ))
            } else {
                None
            },
        )
    }
}
fn sequence<C: Component<Message = Message>>(a: &mut Application<C>, phase: usize) {
    match phase {
        1 => {
            a.key(Key::plain('\t')).unwrap();
            a.list_command(ListCommand::Next, 0, 1).unwrap();
            a.list_command(ListCommand::Next, 0, 1).unwrap();
            a.list_command(ListCommand::Previous, 1, 1).unwrap();
            a.key(Key::plain('\t')).unwrap();
            a.list_command(ListCommand::Next, 0, 1).unwrap();
            a.tree_command(TreeCommand::Expand).unwrap();
            a.list_command(ListCommand::Next, 0, 1).unwrap();
            a.key(Key::plain('\t')).unwrap();
            a.list_command(ListCommand::Next, 0, 1).unwrap();
            a.table_command(TableCommand::NextColumn).unwrap();
            a.table_command(TableCommand::ToggleSort).unwrap();
            a.table_command(TableCommand::ToggleSort).unwrap();
        }
        2 => {
            a.key(Key::plain('\t')).unwrap();
            a.workspace_command(WorkspaceCommand::NextTab).unwrap();
            a.key(Key::plain('\t')).unwrap();
            a.key(Key::plain('\t')).unwrap();
            a.key(Key::plain('\r')).unwrap();
        }
        3 => {
            a.key(Key::plain('\u{1b}')).unwrap();
            a.key(Key::plain('\t')).unwrap();
            assert_eq!(a.focused_key(), Some("split/handle"));
            a.splitter_command(SliderCommand::Increase).unwrap();
        }
        4 => {
            a.key(Key::plain('\t')).unwrap();
            assert_eq!(a.focused_key(), Some("list"));
            assert!(a.begin_rename().unwrap());
            for c in "renamed".chars() {
                a.key(Key::plain(c)).unwrap();
            }
            a.key(Key::plain('\r')).unwrap();
        }
        _ => unreachable!(),
    }
}
#[test]
fn collections_workspace_modal_and_splitter_match_direct_sdk_pixels() {
    let mut s = State::default();
    let mut adapted = Application::new(model(&mut s)).unwrap();
    let mut direct = Application::new(Reference(State::default())).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let style = std::fs::read_to_string(root.join("assets/tcc/workspace.dss")).unwrap();
    for (phase, name) in ["initial", "collections", "dialog", "split", "renamed"]
        .iter()
        .enumerate()
    {
        if phase > 0 {
            sequence(&mut adapted, phase);
            sequence(&mut direct, phase);
        }
        let mut expected = super::checkable_tests::renderer_with_style(&style);
        let mut actual = super::checkable_tests::renderer_with_style(&style);
        expected.render_application(&direct, 640, 620).unwrap();
        actual.render_application(&adapted, 640, 620).unwrap();
        assert_eq!(expected.pixels(), actual.pixels(), "phase {name}");
        if let Some(out) = std::env::var_os("DUNIT_C_GOLDEN_DIR") {
            if phase == 0 {
                let title = expected.widget_rect("title").unwrap();
                let quit = expected.widget_rect("quit").unwrap();
                let geometry = alloc::format!(
                    "{{\"title\":[{},{}],\"quit\":[{},{}]}}",
                    title.x + 12.,
                    title.y + title.h / 2.,
                    quit.x + quit.w / 2.,
                    quit.y + quit.h / 2.
                );
                std::fs::write(
                    std::path::Path::new(&out).join("c-ui-workspace-geometry.json"),
                    geometry,
                )
                .unwrap();
            }

            let rgb: Vec<u8> = expected
                .pixels()
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect();
            std::fs::write(
                std::path::Path::new(&out).join(alloc::format!("c-ui-workspace-{name}.rgb")),
                rgb,
            )
            .unwrap();
        }
    }
    assert_eq!(s.commits, 1);
    assert_eq!(s.label, "renamed");
    assert_eq!(s.changes, 10);
    assert_eq!(s.split, 60);
    assert_eq!(s.split_changes, 1);
    assert!(s.selected && !s.dialog && s.column && s.sort && s.descending);
    assert_eq!(s.list[0] & DUNIT_UI_ITEM_SELECTED, DUNIT_UI_ITEM_SELECTED);
    assert_eq!(s.tree[0] & DUNIT_UI_ITEM_EXPANDED, DUNIT_UI_ITEM_EXPANDED);
}
#[test]
fn workspace_descriptor_cardinality_and_parents_are_checked() {
    let mut s = State::default();
    let config = model(&mut s);
    let mut n = nodes(&mut s);
    n[9].kind = DUNIT_UI_TAB_SELECTED;
    assert!(unsafe {
        tree::convert_extended(
            &n,
            &mut s as *mut _ as *mut c_void,
            None,
            None,
            config.numbers.as_ref(),
            config.collections.as_ref(),
        )
    }
    .is_err());
    let malformed = vec![
        node(DUNIT_UI_TABS, DUNIT_UI_ROOT, "tabs", "Tabs", 0),
        node(DUNIT_UI_TEXT, 0, "ignored", "Must not disappear", 0),
    ];
    assert!(unsafe { tree::convert(&malformed) }.is_err());
    let mut n = vec![node(DUNIT_UI_DIALOG, DUNIT_UI_ROOT, "d", "Title", 0)];
    assert!(unsafe { tree::convert(&n) }.is_err());
    n.push(node(DUNIT_UI_TEXT, 0, "body", "Body", 0));
    assert!(unsafe { tree::convert(&n) }.is_ok());
    n.push(node(DUNIT_UI_TEXT, 0, "extra", "Extra", 0));
    assert!(unsafe { tree::convert(&n) }.is_err());
}
