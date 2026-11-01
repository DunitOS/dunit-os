//! Controlled virtual grid, reusing collection row selection and scrolling.
use crate::{ListSelection, SelectionMode, UiError, View};
use alloc::{string::String, vec::Vec};
pub use dunit_widgets::table::{
    SortOrder, TableColumn, TableError, TableModel, TableRow, TableSort, TableValue,
};
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableState {
    pub selection: ListSelection,
    pub sort: Option<TableSort>,
    pub column: Option<String>,
}
impl TableState {
    pub fn retain(&mut self, model: &TableModel) {
        let items: Vec<_> = model
            .rows()
            .iter()
            .map(|r| crate::ListItem::new(&r.key, "").disabled(r.disabled))
            .collect();
        self.selection.retain(&items);
        if self
            .column
            .as_ref()
            .is_some_and(|k| model.column(k).is_none())
        {
            self.column = None;
        }
        if self.sort.as_ref().is_some_and(|s| {
            model
                .column(&s.column)
                .is_none_or(|i| !model.columns()[i].sortable)
        }) {
            self.sort = None;
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TableBinding {
    pub model: TableModel,
    pub order: Vec<usize>,
    pub sort: Option<TableSort>,
    pub column: Option<String>,
    pub valid: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableCommand {
    PreviousColumn,
    NextColumn,
    SortAscending,
    SortDescending,
    ToggleSort,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCellState<'a> {
    pub role: &'static str,
    pub row: &'a str,
    pub column: &'a str,
    pub label: String,
    pub row_index: usize,
    pub column_index: usize,
    pub selected: bool,
    pub current: bool,
    pub disabled: bool,
    pub editable: bool,
}
impl<M> View<M> {
    pub fn table(
        key: impl Into<String>,
        label: impl Into<String>,
        model: TableModel,
        state: TableState,
        mode: SelectionMode,
        on_change: fn(TableState) -> M,
    ) -> Self {
        let order = model.order(state.sort.as_ref());
        let valid = order.is_some();
        let order = order.unwrap_or_default();
        let items = order
            .iter()
            .map(|&i| {
                let r = &model.rows()[i];
                crate::ListItem::new(&r.key, r.cells[0].text()).disabled(r.disabled)
            })
            .collect();
        let mut v = Self::new(
            key,
            dunit_ui::Kind::Element("Table".into()),
            None,
            Vec::new(),
        );
        v.scroll_axes = crate::ScrollAxes::Both;
        v.list = Some(crate::list::ListBinding {
            label: label.into(),
            items,
            selection: state.selection,
            mode,
            on_change: crate::list::SelectionHandler::Table(crate::callback::Callback::new(
                on_change,
            )),
        });
        v.table = Some(TableBinding {
            model,
            order,
            sort: state.sort,
            column: state.column,
            valid,
        });
        v
    }
    pub fn table_state(&self) -> Option<TableState> {
        let t = self.table.as_ref()?;
        Some(TableState {
            selection: self.list.as_ref()?.selection.clone(),
            sort: t.sort.clone(),
            column: t.column.clone(),
        })
    }
    pub fn table_cell(&self, row: &str, column: &str) -> Option<TableCellState<'_>> {
        let t = self.table.as_ref()?;
        let l = self.list.as_ref()?;
        let r = t.order.iter().position(|&i| t.model.rows()[i].key == row)?;
        let c = t.model.column(column)?;
        let row = &t.model.rows()[t.order[r]];
        let current = t.column.as_deref().map_or(c == 0, |k| k == column);
        Some(TableCellState {
            role: "gridcell",
            row: &row.key,
            column: &t.model.columns()[c].key,
            label: row.cells[c].text(),
            row_index: r + 1,
            column_index: c + 1,
            selected: l.selection.selected.contains(&row.key),
            current: current && l.selection.current.as_ref() == Some(&row.key),
            disabled: self.interaction.disabled || row.disabled,
            editable: t.model.columns()[c].editable,
        })
    }
    pub(crate) fn check_table(&self) -> crate::Result<()> {
        let Some(t) = &self.table else { return Ok(()) };
        let state = self.table_state().ok_or(UiError::InvalidTable)?;
        let mut clean = state.clone();
        clean.retain(&t.model);
        if !t.valid || clean != state {
            return Err(UiError::InvalidTable);
        }
        Ok(())
    }
}
impl<C: crate::Component> crate::Application<C> {
    pub fn table_command(&mut self, command: TableCommand) -> crate::Result<bool> {
        if self.popup_state().is_some() {
            return Ok(false);
        }
        let mut message = None;
        let mut consumed = false;
        self.view.visit_mut(&mut |v| {
            if !v.interaction.focused || v.interaction.disabled {
                return;
            }
            let Some(t) = &v.table else {
                return;
            };
            consumed = true;
            let mut state = v.table_state().unwrap();
            let c = t
                .column
                .as_deref()
                .and_then(|k| t.model.column(k))
                .unwrap_or(0);
            match command {
                TableCommand::PreviousColumn => {
                    state.column = Some(t.model.columns()[c.saturating_sub(1)].key.clone())
                }
                TableCommand::NextColumn => {
                    state.column = Some(
                        t.model.columns()[(c + 1).min(t.model.columns().len() - 1)]
                            .key
                            .clone(),
                    )
                }
                TableCommand::SortAscending
                | TableCommand::SortDescending
                | TableCommand::ToggleSort => {
                    if t.model.columns()[c].sortable {
                        state.sort = Some(TableSort {
                            column: t.model.columns()[c].key.clone(),
                            order: if command == TableCommand::SortAscending
                                || (command == TableCommand::ToggleSort
                                    && !state.sort.as_ref().is_some_and(|s| {
                                        s.column == t.model.columns()[c].key
                                            && s.order == SortOrder::Ascending
                                    }))
                            {
                                SortOrder::Ascending
                            } else {
                                SortOrder::Descending
                            },
                        });
                    }
                }
            }
            if Some(&state) != v.table_state().as_ref() {
                if let crate::list::SelectionHandler::Table(f) = &v.list.as_ref().unwrap().on_change
                {
                    message = Some(f.call(state));
                }
            }
        });
        if let Some(m) = message {
            self.update(m)?;
        }
        Ok(consumed)
    }
    pub fn table_header(&mut self, key: &str, column: usize) -> crate::Result<bool>
    where
        C::Message: Clone,
    {
        if self.popup_state().is_some() || !self.in_scope(key) {
            return Ok(false);
        }
        let Some(v) = self.view.find(key).filter(|v| !v.interaction.disabled) else {
            return Ok(false);
        };
        let Some(column_key) = v
            .table
            .as_ref()
            .and_then(|t| t.model.columns().get(column))
            .filter(|c| c.sortable)
            .map(|c| c.key.clone())
        else {
            return Ok(false);
        };
        let key = String::from(key);
        self.commit_spinbox()?;
        if self.popup_state().is_some() || !self.in_scope(&key) {
            return Ok(true);
        }
        let Some(v) = self.view.find(&key).filter(|v| !v.interaction.disabled) else {
            return Ok(true);
        };
        let Some(t) = &v.table else { return Ok(true) };
        let Some(c) = t
            .model
            .column(&column_key)
            .map(|i| &t.model.columns()[i])
            .filter(|c| c.sortable)
        else {
            return Ok(true);
        };
        let mut state = v.table_state().unwrap();
        state.column = Some(c.key.clone());
        state.sort = Some(TableSort {
            column: c.key.clone(),
            order: if state
                .sort
                .as_ref()
                .is_some_and(|s| s.column == c.key && s.order == SortOrder::Ascending)
            {
                SortOrder::Descending
            } else {
                SortOrder::Ascending
            },
        });
        let message = match &v.list.as_ref().unwrap().on_change {
            crate::list::SelectionHandler::Table(f) => f.call(state),
            _ => return Err(UiError::InvalidTable),
        };
        self.set_focus(&key);
        self.update(message)?;
        Ok(true)
    }
    pub fn table_pointer(
        &mut self,
        key: &str,
        row: usize,
        column: usize,
        modifiers: u8,
    ) -> crate::Result<bool>
    where
        C::Message: Clone,
    {
        if self.popup_state().is_some() || !self.in_scope(key) || modifiers & !3 != 0 {
            return Ok(false);
        }
        let Some(v) = self.view.find(key).filter(|v| !v.interaction.disabled) else {
            return Ok(false);
        };
        let Some(t) = &v.table else { return Ok(false) };
        let Some(column_key) = t.model.columns().get(column).map(|c| c.key.clone()) else {
            return Ok(false);
        };
        let Some(row_key) = v
            .list
            .as_ref()
            .unwrap()
            .items
            .get(row)
            .filter(|r| !r.disabled)
            .map(|r| r.key.clone())
        else {
            return Ok(true);
        };
        // The pointer indices came from the previous rendered frame. Committing
        // another editor can replace/reorder the model; keep their identities.
        self.commit_spinbox()?;
        if self.popup_state().is_some() || !self.in_scope(key) {
            return Ok(true);
        }
        let Some(v) = self.view.find(key).filter(|v| !v.interaction.disabled) else {
            return Ok(true);
        };
        let Some(t) = &v.table else { return Ok(true) };
        let l = v.list.as_ref().unwrap();
        let Some(c) = t.model.column(&column_key).map(|i| &t.model.columns()[i]) else {
            return Ok(true);
        };
        let Some(row) = l.items.iter().position(|r| r.key == row_key && !r.disabled) else {
            return Ok(true);
        };
        let mut state = v.table_state().unwrap();
        state.column = Some(c.key.clone());
        state.selection = l.selection.choose(
            &l.items,
            l.mode,
            row,
            modifiers & 1 != 0,
            modifiers & 2 != 0,
            modifiers & 2 != 0,
        );
        let message = if Some(&state) != v.table_state().as_ref() {
            match &l.on_change {
                crate::list::SelectionHandler::Table(f) => Some(f.call(state)),
                _ => return Err(UiError::InvalidTable),
            }
        } else {
            None
        };
        self.set_focus(key);
        if let Some(m) = message {
            self.update(m)?;
        }
        Ok(true)
    }
}
