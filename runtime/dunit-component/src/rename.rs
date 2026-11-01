//! One controlled in-place editor shared by lists, trees and tables.
use crate::{Application, Component, Key, Result, UiError, View};
use alloc::{format, string::String};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameState {
    pub row: String,
    pub column: Option<String>,
    pub value: String,
}
impl RenameState {
    pub fn new(row: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            row: row.into(),
            column: None,
            value: value.into(),
        }
    }
    pub fn column(mut self, column: impl Into<String>) -> Self {
        self.column = Some(column.into());
        self
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameEvent {
    Begin { row: String, column: Option<String> },
    Change(String),
    Commit,
    Cancel,
}
#[derive(Debug, Clone)]
pub(crate) struct RenameBinding<M> {
    pub state: Option<RenameState>,
    pub on_event: crate::callback::Callback<RenameEvent, M>,
}
impl<M> PartialEq for RenameBinding<M> {
    fn eq(&self, b: &Self) -> bool {
        self.state == b.state && self.on_event == b.on_event
    }
}
impl<M> View<M> {
    pub fn renaming(mut self, state: Option<RenameState>, on_event: fn(RenameEvent) -> M) -> Self {
        self.rename = Some(RenameBinding {
            state: state.clone(),
            on_event: crate::callback::Callback::new(on_event),
        });
        if let Some(state) = state {
            let key = format!(
                "{}/rename/{}:{}{}:{}",
                self.key,
                state.row.len(),
                state.row,
                state.column.as_ref().map_or(0, |c| c.len()),
                state.column.as_deref().unwrap_or("")
            );
            let mut editor = Self::new(
                key,
                dunit_ui::Kind::Element("Input".into()),
                Some(state.value.clone()),
                alloc::vec::Vec::new(),
            );
            editor.input = Some(crate::input::InputBinding {
                affinity: dunit_render::CaretAffinity::Downstream,
                visual_goal: None,
                owner: Some(self.key.clone()),
                label: self
                    .list
                    .as_ref()
                    .map_or_else(String::new, |l| l.label.clone()),
                options: crate::InputOptions::default(),
                editor: crate::TextEditor::new(state.value),
                on_change: None,
                dragging: false,
                caret_visible: true,
                caret_revision: 0,
            });
            editor.interaction.disabled = self.interaction.disabled;
            self.children.push(editor);
        }
        self
    }
    pub(crate) fn check_rename(&self) -> Result<()> {
        let Some(rename) = &self.rename else {
            return Ok(());
        };
        let list = self.list.as_ref().ok_or(UiError::InvalidRename)?;
        if let Some(state) = &rename.state {
            if !list.items.iter().any(|r| r.key == state.row && !r.disabled) {
                return Err(UiError::InvalidRename);
            }
            if let Some(table) = &self.table {
                if !state
                    .column
                    .as_deref()
                    .and_then(|k| table.model.column(k))
                    .is_some_and(|c| table.model.columns()[c].editable)
                {
                    return Err(UiError::InvalidRename);
                }
            } else if state.column.is_some() {
                return Err(UiError::InvalidRename);
            }
            if state.value.len() > crate::input::MAX_INPUT_BYTES {
                return Err(UiError::InvalidRename);
            }
        }
        Ok(())
    }
}
impl<C: Component> Application<C> {
    pub(crate) fn sync_rename(&mut self) {
        let mut nodes = alloc::vec::Vec::new();
        self.view.flatten(&mut nodes);
        let next = nodes.iter().find_map(|v| {
            v.input
                .as_ref()?
                .owner
                .as_ref()
                .filter(|_| !v.interaction.disabled && self.in_scope(&v.key))
                .map(|owner| (owner.clone(), v.key.clone()))
        });
        if next == self.rename_focus {
            return;
        }
        let previous = self.rename_focus.take();
        self.rename_focus = next.clone();
        if let Some((_, key)) = next {
            self.close_popup();
            self.dismiss_tooltip();
            self.set_focus(&key);
            self.view.visit_mut(&mut |v| {
                if v.key == key {
                    if let Some(i) = &mut v.input {
                        i.editor.apply(
                            crate::EditCommand::SelectAll,
                            i.options.max_chars,
                            i.options.read_only,
                        );
                    }
                }
            });
        } else if let Some((owner, _)) = previous {
            if self.in_scope(&owner)
                && self
                    .view
                    .find(&owner)
                    .is_some_and(|v| v.focusable() && !v.interaction.disabled)
            {
                self.set_focus(&owner);
            }
        }
    }
    pub fn begin_rename(&mut self) -> Result<bool> {
        if self.popup_state().is_some() {
            return Ok(false);
        }
        let Some(key) = self.focused_key() else {
            return Ok(false);
        };
        let Some(v) = self.view.find(key).filter(|v| !v.interaction.disabled) else {
            return Ok(false);
        };
        let Some(rename) = &v.rename else {
            return Ok(false);
        };
        let Some(list) = &v.list else {
            return Ok(false);
        };
        let Some(row) = &list.selection.current else {
            return Ok(true);
        };
        let column = v.table.as_ref().map(|t| {
            t.column
                .clone()
                .unwrap_or_else(|| t.model.columns()[0].key.clone())
        });
        if let Some(t) = &v.table {
            if !column
                .as_deref()
                .and_then(|c| t.model.column(c))
                .is_some_and(|c| t.model.columns()[c].editable)
            {
                return Ok(true);
            }
        }
        let message = rename.on_event.call(RenameEvent::Begin {
            row: row.clone(),
            column,
        });
        self.update(message)?;
        Ok(true)
    }
    pub(crate) fn rename_key(&mut self, key: Key) -> Result<bool> {
        let Some(owner) = self
            .focused_input_key()
            .and_then(|k| self.view.find(k)?.input.as_ref()?.owner.clone())
        else {
            return Ok(false);
        };
        let event = match key.character {
            '\r' | '\n' | '\t' if key.modifiers & !1 == 0 => RenameEvent::Commit,
            '\u{1b}' if key.modifiers == 0 => RenameEvent::Cancel,
            _ => return Ok(false),
        };
        let message = self
            .view
            .find(&owner)
            .and_then(|v| v.rename.as_ref())
            .ok_or(UiError::InvalidRename)?
            .on_event
            .call(event);
        self.update(message)?;
        Ok(true)
    }
    pub(crate) fn rename_change(&self, owner: &str, value: String) -> Result<C::Message> {
        Ok(self
            .view
            .find(owner)
            .and_then(|v| v.rename.as_ref())
            .ok_or(UiError::InvalidRename)?
            .on_event
            .call(RenameEvent::Change(value)))
    }
}
