//! Bounded foreign collection data become the SDK's validated immutable models.
use super::*;
use alloc::collections::BTreeSet;
#[derive(Clone)]
pub(super) struct State {
    pub selection: ListSelection,
    pub expanded: BTreeSet<String>,
    pub sort: Option<TableSort>,
    pub column: Option<String>,
}
impl State {
    pub unsafe fn dispatch(
        &self,
        user: *mut c_void,
        id: u64,
        changed: DunitUiCollectionChangedV1,
    ) -> i32 {
        let mut keys = self.selection.selected.clone();
        keys.extend(self.expanded.iter().cloned());
        keys.extend(self.selection.current.iter().cloned());
        keys.extend(self.selection.anchor.iter().cloned());
        let items: Vec<_> = keys
            .iter()
            .map(|key| DunitUiCollectionItemV1 {
                key: key.as_ptr(),
                key_len: key.len(),
                label: core::ptr::null(),
                label_len: 0,
                parent: core::ptr::null(),
                parent_len: 0,
                flags: if self.selection.selected.contains(key) {
                    DUNIT_UI_ITEM_SELECTED
                } else {
                    0
                } | if self.selection.current.as_ref() == Some(key) {
                    DUNIT_UI_ITEM_CURRENT
                } else {
                    0
                } | if self.selection.anchor.as_ref() == Some(key) {
                    DUNIT_UI_ITEM_ANCHOR
                } else {
                    0
                } | if self.expanded.contains(key) {
                    DUNIT_UI_ITEM_EXPANDED
                } else {
                    0
                },
                reserved: 0,
            })
            .collect();
        let mut config = empty();
        config.items = items.as_ptr();
        config.items_len = items.len();
        if let Some(sort) = &self.sort {
            config.sort = sort.column.as_ptr();
            config.sort_len = sort.column.len();
            if sort.order == SortOrder::Descending {
                config.flags |= DUNIT_UI_COLLECTION_SORT_DESCENDING
            }
        }
        if let Some(column) = &self.column {
            config.column = column.as_ptr();
            config.column_len = column.len()
        }
        changed(user, id, &config)
    }
}
fn empty() -> DunitUiCollectionV1 {
    DunitUiCollectionV1 {
        items: core::ptr::null(),
        items_len: 0,
        columns: core::ptr::null(),
        columns_len: 0,
        cells: core::ptr::null(),
        cells_len: 0,
        flags: 0,
        reserved: 0,
        sort: core::ptr::null(),
        sort_len: 0,
        column: core::ptr::null(),
        column_len: 0,
        rename: core::ptr::null(),
    }
}
unsafe fn slice<'a, T>(pointer: *const T, length: usize) -> dunit::Result<&'a [T]> {
    if length > MAX_VIEW_NODES || (length != 0 && pointer.is_null()) {
        return Err(invalid());
    }
    Ok(if length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(pointer, length)
    })
}
pub(super) unsafe fn view<M: Clone + 'static>(
    node: &DunitUiNodeV1,
    index: u32,
    key: String,
    user: *mut c_void,
    callbacks: &DunitUiCollectionsV1,
    changed: fn(u64, State) -> M,
    renamed: fn(u64, RenameEvent) -> M,
) -> dunit::Result<View<M>> {
    let mut config = empty();
    let result = callbacks.configure.ok_or_else(invalid)?(user, index, &mut config);
    if result != 0 {
        return Err(if result < 0 {
            dunit::Error(result as isize)
        } else {
            invalid()
        });
    }
    if callbacks.changed.is_none()
        || config.reserved != 0
        || config.flags & !(DUNIT_UI_COLLECTION_MULTIPLE | DUNIT_UI_COLLECTION_SORT_DESCENDING) != 0
    {
        return Err(invalid());
    }
    let kind = node.kind & !DUNIT_UI_DISABLED;
    if kind != DUNIT_UI_TABLE
        && (config.columns_len != 0
            || config.cells_len != 0
            || config.sort_len != 0
            || config.column_len != 0
            || config.flags & DUNIT_UI_COLLECTION_SORT_DESCENDING != 0)
    {
        return Err(invalid());
    }
    let mut selection = ListSelection::default();
    let mut expanded = BTreeSet::new();
    let mut items = Vec::new();
    let mut tree = Vec::new();
    let mut row_keys = Vec::new();
    let mut unique = BTreeSet::new();
    for item in slice(config.items, config.items_len)? {
        if item.reserved != 0
            || item.flags
                & !(DUNIT_UI_ITEM_DISABLED
                    | DUNIT_UI_ITEM_SELECTED
                    | DUNIT_UI_ITEM_CURRENT
                    | DUNIT_UI_ITEM_ANCHOR
                    | DUNIT_UI_ITEM_EXPANDED)
                != 0
        {
            return Err(invalid());
        }
        let key = tree::text(item.key, item.key_len)?;
        if key.is_empty() || !unique.insert(key.clone()) {
            return Err(invalid());
        }
        if item.flags & DUNIT_UI_ITEM_SELECTED != 0 {
            selection.selected.insert(key.clone());
        }
        if item.flags & DUNIT_UI_ITEM_CURRENT != 0
            && selection.current.replace(key.clone()).is_some()
        {
            return Err(invalid());
        }
        if item.flags & DUNIT_UI_ITEM_ANCHOR != 0 && selection.anchor.replace(key.clone()).is_some()
        {
            return Err(invalid());
        }
        if item.flags & DUNIT_UI_ITEM_EXPANDED != 0 {
            if kind != DUNIT_UI_TREE {
                return Err(invalid());
            }
            expanded.insert(key.clone());
        }
        let label = tree::text(item.label, item.label_len)?;
        let disabled = item.flags & DUNIT_UI_ITEM_DISABLED != 0;
        items.push(ListItem::new(key.clone(), label.clone()).disabled(disabled));
        row_keys.push((key.clone(), disabled));
        let mut entry = TreeItem::new(key, label).disabled(disabled);
        if item.parent_len > 0 {
            if kind != DUNIT_UI_TREE {
                return Err(invalid());
            }
            entry = entry.under(tree::text(item.parent, item.parent_len)?);
        }
        tree.push(entry);
    }
    let mode = if config.flags & DUNIT_UI_COLLECTION_MULTIPLE != 0 {
        SelectionMode::Multiple
    } else {
        SelectionMode::Single
    };
    if mode == SelectionMode::Single && selection.selected.len() > 1 {
        return Err(invalid());
    }
    let label = tree::text(node.text, node.text_len)?;
    let id = node.message;
    let view = match kind {
        DUNIT_UI_LIST => {
            View::list(key, label, items, selection, mode, |s| s).map_messages(move |s| {
                changed(
                    id,
                    State {
                        selection: s,
                        expanded: BTreeSet::new(),
                        sort: None,
                        column: None,
                    },
                )
            })
        }
        DUNIT_UI_TREE => View::tree_view(
            key,
            label,
            TreeModel::new(tree).map_err(|_| invalid())?,
            TreeState {
                selection,
                expanded,
            },
            mode,
            |s| s,
        )
        .map_messages(move |s| {
            changed(
                id,
                State {
                    selection: s.selection,
                    expanded: s.expanded,
                    sort: None,
                    column: None,
                },
            )
        }),
        DUNIT_UI_TABLE => {
            let mut columns = Vec::new();
            for c in slice(config.columns, config.columns_len)? {
                if c.reserved != 0
                    || c.flags & !(DUNIT_UI_COLUMN_SORTABLE | DUNIT_UI_COLUMN_EDITABLE) != 0
                    || c.width < 0
                {
                    return Err(invalid());
                }
                let mut col = TableColumn::new(
                    tree::text(c.key, c.key_len)?,
                    tree::text(c.label, c.label_len)?,
                )
                .sortable(c.flags & DUNIT_UI_COLUMN_SORTABLE != 0)
                .editable(c.flags & DUNIT_UI_COLUMN_EDITABLE != 0);
                if c.width > 0 {
                    col = col.width(c.width as f32)
                }
                columns.push(col);
            }
            if config.cells_len
                != row_keys
                    .len()
                    .checked_mul(columns.len())
                    .ok_or_else(invalid)?
            {
                return Err(invalid());
            }
            let mut cells = Vec::new();
            for c in slice(config.cells, config.cells_len)? {
                if c.reserved != 0 {
                    return Err(invalid());
                }
                cells.push(match c.kind {
                    DUNIT_UI_CELL_TEXT => TableValue::Text(tree::text(c.text, c.text_len)?),
                    DUNIT_UI_CELL_INTEGER => TableValue::Integer(c.value as i64),
                    DUNIT_UI_CELL_UNSIGNED => TableValue::Unsigned(c.value),
                    _ => return Err(invalid()),
                });
            }
            let mut cells = cells.into_iter();
            let mut rows = Vec::new();
            for (key, disabled) in row_keys {
                rows.push(
                    TableRow::new(key, cells.by_ref().take(columns.len()).collect())
                        .disabled(disabled),
                );
            }
            let sort = if config.sort_len == 0 {
                if config.flags & DUNIT_UI_COLLECTION_SORT_DESCENDING != 0 {
                    return Err(invalid());
                }
                None
            } else {
                Some(TableSort {
                    column: tree::text(config.sort, config.sort_len)?,
                    order: if config.flags & DUNIT_UI_COLLECTION_SORT_DESCENDING != 0 {
                        SortOrder::Descending
                    } else {
                        SortOrder::Ascending
                    },
                })
            };
            let column = if config.column_len == 0 {
                None
            } else {
                Some(tree::text(config.column, config.column_len)?)
            };
            View::table(
                key,
                label,
                TableModel::new(columns, rows).map_err(|_| invalid())?,
                TableState {
                    selection,
                    sort,
                    column,
                },
                mode,
                |s| s,
            )
            .map_messages(move |s| {
                changed(
                    id,
                    State {
                        selection: s.selection,
                        expanded: BTreeSet::new(),
                        sort: s.sort,
                        column: s.column,
                    },
                )
            })
        }
        _ => return Err(invalid()),
    };
    if callbacks.renamed.is_none() {
        if !config.rename.is_null() {
            return Err(invalid());
        }
        return Ok(view);
    }
    let rename = if config.rename.is_null() {
        None
    } else {
        let r = &*config.rename;
        if r.kind != 0 || r.reserved != 0 {
            return Err(invalid());
        }
        let mut state = RenameState::new(
            tree::text(r.row, r.row_len)?,
            tree::text(r.value, r.value_len)?,
        );
        if r.column_len > 0 {
            state = state.column(tree::text(r.column, r.column_len)?)
        }
        Some(state)
    };
    #[derive(Clone)]
    enum Event<M> {
        Change(M),
        Rename(RenameEvent),
    }
    Ok(view
        .map_messages(Event::Change)
        .renaming(rename, Event::Rename)
        .map_messages(move |event| match event {
            Event::Change(m) => m,
            Event::Rename(e) => renamed(id, e),
        }))
}
pub(super) unsafe fn rename_dispatch(
    user: *mut c_void,
    id: u64,
    event: &RenameEvent,
    callback: DunitUiRenamedV1,
) -> i32 {
    let (kind, row, column, value) = match event {
        RenameEvent::Begin { row, column } => (
            DUNIT_UI_RENAME_BEGIN,
            row.as_str(),
            column.as_deref().unwrap_or(""),
            "",
        ),
        RenameEvent::Change(value) => (DUNIT_UI_RENAME_CHANGE, "", "", value.as_str()),
        RenameEvent::Commit => (DUNIT_UI_RENAME_COMMIT, "", "", ""),
        RenameEvent::Cancel => (DUNIT_UI_RENAME_CANCEL, "", "", ""),
    };
    let event = DunitUiRenameV1 {
        kind,
        reserved: 0,
        row: row.as_ptr(),
        row_len: row.len(),
        column: column.as_ptr(),
        column_len: column.len(),
        value: value.as_ptr(),
        value_len: value.len(),
    };
    callback(user, id, &event)
}

#[cfg(test)]
mod tests {
    use super::*;
    unsafe extern "C" fn configure(user: *mut c_void, _: u32, c: *mut DunitUiCollectionV1) -> i32 {
        *c = *(user as *const DunitUiCollectionV1);
        0
    }
    unsafe extern "C" fn changed(_: *mut c_void, _: u64, _: *const DunitUiCollectionV1) -> i32 {
        0
    }
    fn validate(kind: u32, c: &mut DunitUiCollectionV1) -> bool {
        let callbacks = DunitUiCollectionsV1 {
            abi_version: 1,
            struct_size: core::mem::size_of::<DunitUiCollectionsV1>() as u32,
            configure: Some(configure),
            changed: Some(changed),
            renamed: None,
        };
        let node = DunitUiNodeV1 {
            kind,
            parent: DUNIT_UI_ROOT,
            key: b"x".as_ptr(),
            key_len: 1,
            text: b"Label".as_ptr(),
            text_len: 5,
            message: u64::MAX,
        };
        unsafe {
            view(
                &node,
                0,
                "x".into(),
                c as *mut _ as *mut c_void,
                &callbacks,
                Message::Collection,
                Message::Rename,
            )
        }
        .is_ok_and(|v| v.validate().is_ok())
    }
    #[test]
    fn malformed_foreign_collections_cannot_bypass_the_sdk_model_validator() {
        let mut c = empty();
        assert!(validate(DUNIT_UI_LIST, &mut c));
        c.flags = 4;
        assert!(!validate(DUNIT_UI_LIST, &mut c));
        c = empty();
        c.items_len = 1;
        assert!(!validate(DUNIT_UI_LIST, &mut c));
        c.items_len = MAX_VIEW_NODES + 1;
        assert!(!validate(DUNIT_UI_TREE, &mut c));
        let mut items = vec![DunitUiCollectionItemV1 {
            key: b"a".as_ptr(),
            key_len: 1,
            label: b"Alpha".as_ptr(),
            label_len: 5,
            parent: core::ptr::null(),
            parent_len: 0,
            flags: 0,
            reserved: 0,
        }];
        c = empty();
        c.items = items.as_ptr();
        c.items_len = 1;
        assert!(validate(DUNIT_UI_LIST, &mut c));
        items[0].flags = DUNIT_UI_ITEM_DISABLED | DUNIT_UI_ITEM_SELECTED;
        assert!(!validate(DUNIT_UI_LIST, &mut c));
        items[0].flags = DUNIT_UI_ITEM_EXPANDED;
        assert!(!validate(DUNIT_UI_LIST, &mut c));
        items[0].flags = 0;
        items[0].label = b"\xff".as_ptr();
        items[0].label_len = 1;
        assert!(!validate(DUNIT_UI_LIST, &mut c));
        items[0].label = b"Alpha".as_ptr();
        items[0].label_len = 5;
        items[0].parent = b"missing".as_ptr();
        items[0].parent_len = 7;
        assert!(!validate(DUNIT_UI_TREE, &mut c));
        items[0].parent_len = 0;
        items.push(items[0]);
        c.items = items.as_ptr();
        c.items_len = 2;
        assert!(!validate(DUNIT_UI_LIST, &mut c));
        items[1].key = b"b".as_ptr();
        for item in &mut items {
            item.flags = DUNIT_UI_ITEM_CURRENT;
        }
        assert!(!validate(DUNIT_UI_LIST, &mut c));
        for item in &mut items {
            item.flags = DUNIT_UI_ITEM_SELECTED;
        }
        assert!(!validate(DUNIT_UI_LIST, &mut c));
        c.flags = DUNIT_UI_COLLECTION_MULTIPLE;
        assert!(validate(DUNIT_UI_LIST, &mut c));
        assert!(!validate(DUNIT_UI_TABLE, &mut c));
        let column = DunitUiTableColumnV1 {
            key: b"name".as_ptr(),
            key_len: 4,
            label: b"Name".as_ptr(),
            label_len: 4,
            width: 100,
            flags: DUNIT_UI_COLUMN_SORTABLE,
            reserved: 0,
        };
        c.columns = &column;
        c.columns_len = 1;
        let cells = [
            DunitUiTableCellV1 {
                text: core::ptr::null(),
                text_len: 0,
                value: 2,
                kind: DUNIT_UI_CELL_INTEGER,
                reserved: 0,
            },
            DunitUiTableCellV1 {
                text: core::ptr::null(),
                text_len: 0,
                value: u64::MAX,
                kind: DUNIT_UI_CELL_UNSIGNED,
                reserved: 0,
            },
        ];
        c.cells = cells.as_ptr();
        c.cells_len = 2;
        assert!(validate(DUNIT_UI_TABLE, &mut c));
        let invalid_width = DunitUiTableColumnV1 {
            width: -1,
            ..column
        };
        c.columns = &invalid_width;
        assert!(!validate(DUNIT_UI_TABLE, &mut c));
        let invalid_flags = DunitUiTableColumnV1 { flags: 4, ..column };
        c.columns = &invalid_flags;
        assert!(!validate(DUNIT_UI_TABLE, &mut c));
        c.columns = &column;
        c.sort = b"gone".as_ptr();
        c.sort_len = 4;
        assert!(!validate(DUNIT_UI_TABLE, &mut c));
    }
}
