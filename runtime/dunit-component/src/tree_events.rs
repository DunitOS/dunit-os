use crate::{Application, Component, PointerEvent, Result, TreeCommand, View};
use alloc::string::String;
impl<M> View<M> {
    fn tree_proposal(&self, row: usize, command: TreeCommand) -> Option<crate::TreeState> {
        let tree = self.tree.as_ref()?;
        let list = self.list.as_ref()?;
        let target = tree.rows.get(row)?;
        if list.items[row].disabled {
            return None;
        }
        let mut state = self.tree_state()?;
        let key = &list.items[row].key;
        let expanding = match command {
            TreeCommand::Expand => true,
            TreeCommand::Collapse => false,
            TreeCommand::Toggle => !state.expanded.contains(key),
        };
        if target.expanded.is_some()
            && (command == TreeCommand::Toggle || state.expanded.contains(key) != expanding)
        {
            if expanding {
                state.expanded.insert(key.clone());
            } else {
                state.expanded.remove(key);
            }
            state.retain(&tree.model);
            return Some(state);
        }
        let next = if command == TreeCommand::Expand {
            tree.model.children(target.index).iter().find_map(|&i| {
                tree.rows
                    .iter()
                    .position(|r| r.index == i)
                    .filter(|&p| !list.items[p].disabled)
            })
        } else if command == TreeCommand::Collapse {
            tree.model
                .parent(target.index)
                .and_then(|i| tree.rows.iter().position(|r| r.index == i))
                .filter(|&i| !list.items[i].disabled)
        } else {
            None
        };
        if let Some(i) = next {
            state.selection = list
                .selection
                .choose(&list.items, list.mode, i, false, false, false);
        }
        Some(state)
    }
}
impl<C: Component> Application<C> {
    pub fn tree_command(&mut self, command: TreeCommand) -> Result<bool> {
        if self.popup_state().is_some() {
            return Ok(false);
        }
        let mut message = None;
        let mut consumed = false;
        self.view.visit_mut(&mut |v| {
            if !v.interaction.focused || v.interaction.disabled || v.tree.is_none() {
                return;
            }
            consumed = true;
            let list = v.list.as_ref().unwrap();
            if let Some(row) = list
                .selection
                .current
                .as_ref()
                .and_then(|k| list.items.iter().position(|i| &i.key == k))
            {
                if let Some(state) = v
                    .tree_proposal(row, command)
                    .filter(|s| Some(s) != v.tree_state().as_ref())
                {
                    if let crate::list::SelectionHandler::Tree(f) = &list.on_change {
                        message = Some(f(state));
                    }
                }
            }
        });
        if consumed {
            self.dismiss_tooltip();
        }
        if let Some(m) = message {
            self.update(m)?;
        }
        Ok(consumed)
    }
    /// Toggle one visible disclosure, keeping selection except hidden descendants.
    pub fn tree_toggle_row(&mut self, key: &str, row: usize) -> Result<bool>
    where
        C::Message: Clone,
    {
        if self.popup_state().is_some() || !self.in_scope(key) {
            return Ok(false);
        }
        let Some(v) = self.view.find(key).filter(|v| !v.interaction.disabled) else {
            return Ok(false);
        };
        let Some(t) = &v.tree else { return Ok(false) };
        if !t.rows.get(row).is_some_and(|r| r.expanded.is_some())
            || v.list.as_ref().unwrap().items[row].disabled
        {
            return Ok(false);
        }
        let row_key = v.list.as_ref().unwrap().items[row].key.clone();
        let key = String::from(key);
        self.pointer(PointerEvent::Down, Some(&key))?;
        let mut message = None;
        if let Some(v) = self.view.find(&key) {
            let list = v.list.as_ref();
            if list.is_some_and(|l| l.items.get(row).is_some_and(|i| i.key == row_key)) {
                if let Some(state) = v
                    .tree_proposal(row, TreeCommand::Toggle)
                    .filter(|s| Some(s) != v.tree_state().as_ref())
                {
                    if let crate::list::SelectionHandler::Tree(f) = &list.unwrap().on_change {
                        message = Some(f(state));
                    }
                }
            }
        }
        if let Some(m) = message {
            self.update(m)?;
        }
        Ok(true)
    }
}
