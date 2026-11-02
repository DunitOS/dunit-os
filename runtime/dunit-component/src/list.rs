//! Controlled virtual list descriptions and input. No native dependencies.
use crate::{Application, Component, Key, PointerEvent, Result, UiError, View};
use alloc::{string::String, vec::Vec};
pub use dunit_widgets::selection::{ListGeometry, ListItem, ListSelection, SelectionMode};
#[derive(Debug, Clone)]
pub(crate) enum SelectionHandler<M> {
    List(crate::callback::Callback<ListSelection, M>),
    Tree(crate::callback::Callback<crate::TreeState, M>),
    Table(crate::callback::Callback<crate::TableState, M>),
}
impl<M> PartialEq for SelectionHandler<M> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::List(a), Self::List(b)) => a == b,
            (Self::Tree(a), Self::Tree(b)) => a == b,
            (Self::Table(a), Self::Table(b)) => a == b,
            _ => false,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct ListBinding<M> {
    pub label: String,
    pub items: Vec<ListItem>,
    pub selection: ListSelection,
    pub mode: SelectionMode,
    pub on_change: SelectionHandler<M>,
}
impl<M> PartialEq for ListBinding<M> {
    fn eq(&self, o: &Self) -> bool {
        self.label == o.label
            && self.items == o.items
            && self.selection == o.selection
            && self.mode == o.mode
            && self.on_change == o.on_change
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListCommand {
    Previous,
    Next,
    First,
    Last,
    PageUp,
    PageDown,
    Toggle,
    SelectAll,
}
impl<M> View<M> {
    pub(crate) fn collection_message(&self, selection: ListSelection) -> crate::Result<M> {
        let l = self.list.as_ref().ok_or(UiError::InvalidList)?;
        Ok(match &l.on_change {
            crate::list::SelectionHandler::List(f) => f.call(selection),
            crate::list::SelectionHandler::Table(f) => {
                let t = self.table.as_ref().ok_or(UiError::InvalidTable)?;
                f.call(crate::TableState {
                    selection,
                    sort: t.sort.clone(),
                    column: t.column.clone(),
                })
            }
            crate::list::SelectionHandler::Tree(f) => f.call(crate::TreeState {
                selection,
                expanded: self
                    .tree
                    .as_ref()
                    .ok_or(UiError::InvalidTree)?
                    .expanded
                    .clone(),
            }),
        })
    }

    pub fn list(
        key: impl Into<String>,
        label: impl Into<String>,
        items: Vec<ListItem>,
        selection: ListSelection,
        mode: SelectionMode,
        on_change: fn(ListSelection) -> M,
    ) -> Self {
        let mut v = Self::new(
            key,
            dunit_ui::Kind::Element("List".into()),
            None,
            Vec::new(),
        );
        v.list = Some(ListBinding {
            label: label.into(),
            items,
            selection,
            mode,
            on_change: SelectionHandler::List(crate::callback::Callback::new(on_change)),
        });
        v
    }
    pub fn list_selection(&self) -> Option<&ListSelection> {
        self.list.as_ref().map(|l| &l.selection)
    }
    pub fn list_items(&self) -> Option<&[ListItem]> {
        self.list.as_ref().map(|l| l.items.as_slice())
    }
    pub fn accessibility_multiselectable(&self) -> Option<bool> {
        self.list
            .as_ref()
            .map(|l| l.mode == SelectionMode::Multiple)
    }
    pub(crate) fn check_list(&self) -> Result<()> {
        let Some(l) = &self.list else { return Ok(()) };
        let mut keys = alloc::collections::BTreeSet::new();
        if l.items
            .iter()
            .any(|i| i.key.is_empty() || !keys.insert(&i.key))
        {
            return Err(UiError::InvalidList);
        }
        let mut clean = l.selection.clone();
        clean.retain(&l.items);
        if clean != l.selection
            || (l.mode == SelectionMode::Single && l.selection.selected.len() > 1)
        {
            return Err(UiError::InvalidList);
        }
        Ok(())
    }
}
impl<C: Component> Application<C> {
    pub fn list_command(
        &mut self,
        command: ListCommand,
        modifiers: u8,
        page: usize,
    ) -> Result<bool> {
        if self.popup_state().is_some() || modifiers & !3 != 0 {
            return Ok(false);
        }
        self.dismiss_tooltip();
        let mut message = None;
        let mut consumed = false;
        self.view.visit_mut(&mut |v| {
            if !v.interaction.focused || v.interaction.disabled {
                return;
            }
            let Some(l) = &v.list else { return };
            consumed = true;
            let enabled: Vec<_> = l
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| !i.disabled)
                .map(|(i, _)| i)
                .collect();
            let current = l
                .selection
                .current
                .as_ref()
                .and_then(|k| enabled.iter().position(|&i| &l.items[i].key == k));
            let next = match command {
                ListCommand::SelectAll => Some(l.selection.select_all(&l.items, l.mode)),
                _ => {
                    let index = match command {
                        ListCommand::First => enabled.first().copied(),
                        ListCommand::Last => enabled.last().copied(),
                        ListCommand::Previous | ListCommand::PageUp => current
                            .map(|i| {
                                i.saturating_sub(if command == ListCommand::PageUp {
                                    page.max(1)
                                } else {
                                    1
                                })
                            })
                            .or_else(|| enabled.len().checked_sub(1))
                            .and_then(|i| enabled.get(i).copied()),
                        ListCommand::Next | ListCommand::PageDown => current
                            .map(|i| {
                                i.saturating_add(if command == ListCommand::PageDown {
                                    page.max(1)
                                } else {
                                    1
                                })
                                .min(enabled.len().saturating_sub(1))
                            })
                            .or(Some(0))
                            .and_then(|i| enabled.get(i).copied()),
                        ListCommand::Toggle => current
                            .and_then(|i| enabled.get(i).copied())
                            .or_else(|| enabled.first().copied()),
                        ListCommand::SelectAll => unreachable!(),
                    };
                    index.map(|i| {
                        l.selection.choose(
                            &l.items,
                            l.mode,
                            i,
                            modifiers & 1 != 0,
                            modifiers & 2 != 0,
                            command == ListCommand::Toggle,
                        )
                    })
                }
            };
            if let Some(next) = next.filter(|s| s != &l.selection) {
                message = Some(v.collection_message(next));
            }
        });
        if let Some(m) = message {
            self.update(m?)?;
        }
        Ok(consumed)
    }
    pub(crate) fn list_key(&mut self, key: Key) -> Result<bool> {
        if key.character == ' ' {
            return self.list_command(ListCommand::Toggle, key.modifiers, 1);
        }
        if key.modifiers == 2 && matches!(key.character, 'a' | 'A' | '\u{1}') {
            return self.list_command(ListCommand::SelectAll, 2, 1);
        }
        Ok(false)
    }
    /// Pointer selection supports Shift range and Ctrl additive/toggle selection.
    pub fn list_pointer(
        &mut self,
        event: PointerEvent,
        target: Option<&str>,
        row: Option<usize>,
        modifiers: u8,
    ) -> Result<bool>
    where
        C::Message: Clone,
    {
        if self.popup_state().is_some() {
            return self.pointer(event, target);
        }
        let Some(key) = target.filter(|k| self.in_scope(k)).map(String::from) else {
            return Ok(false);
        };
        if !self
            .view
            .find(&key)
            .is_some_and(|v| v.list.is_some() && !v.interaction.disabled)
        {
            return Ok(false);
        }
        self.pointer(event, Some(&key))?;
        if event == PointerEvent::Down {
            self.dismiss_tooltip();
            let mut message = None;
            if let Some(v) = self.view.find(&key) {
                let l = v.list.as_ref().ok_or(UiError::InvalidList)?;
                if let Some(i) = row {
                    let next = l.selection.choose(
                        &l.items,
                        l.mode,
                        i,
                        modifiers & 1 != 0,
                        modifiers & 2 != 0,
                        modifiers & 2 != 0,
                    );
                    if next != l.selection {
                        message = Some(v.collection_message(next));
                    }
                }
            }
            if let Some(m) = message {
                self.update(m?)?;
            }
        }
        Ok(true)
    }
}
