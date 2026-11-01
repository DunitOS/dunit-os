//! Workspace focus scopes and controlled navigation.
use crate::workspace::WorkspaceRole as R;
use crate::{Application, Component, Key, Navigation, Result, View, WorkspaceCommand as K};
use alloc::{string::String, vec::Vec};
impl<C: Component> Application<C> {
    pub fn active_dialog(&self) -> Option<&str> {
        self.dialog_focus.key.as_deref()
    }
    pub(crate) fn in_scope(&self, key: &str) -> bool {
        self.dialog_focus
            .key
            .as_deref()
            .is_none_or(|scope| self.view.find(scope).is_some_and(|v| v.find(key).is_some()))
    }
    pub(crate) fn sync_dialog(&mut self, previous_focus: Option<String>) {
        let mut nodes = Vec::new();
        self.view.flatten(&mut nodes);
        let next = nodes
            .into_iter()
            .find(|v| v.dialog.is_some())
            .map(|v| v.key.clone());
        let changed = next != self.dialog_focus.key;
        if changed {
            self.close_popup();
            self.dismiss_tooltip();
            if self.dialog_focus.key.is_none() && next.is_some() {
                self.dialog_focus.restore = previous_focus;
            }
            self.dialog_focus.key = next;
        }
        if let Some(key) = self.dialog_focus.key.clone() {
            let focused = self.focused_key().map(String::from);
            if !changed && focused.as_deref().is_some_and(|key| self.in_scope(key)) {
                return;
            }
            let root = self.view.find(&key).unwrap();
            let mut nodes = Vec::new();
            root.flatten(&mut nodes);
            let target = root
                .dialog
                .as_ref()
                .and_then(|d| d.default.as_ref())
                .filter(|key| {
                    root.find(key)
                        .is_some_and(|v| v.focusable() && !v.interaction.disabled)
                })
                .cloned()
                .or_else(|| {
                    nodes
                        .into_iter()
                        .find(|v| v.focusable() && !v.interaction.disabled)
                        .map(|v| v.key.clone())
                });
            self.set_focus(target.as_deref().unwrap_or(""));
        } else if changed {
            let target = self.dialog_focus.restore.take().filter(|key| {
                self.view
                    .find(key)
                    .is_some_and(|v| v.focusable() && !v.interaction.disabled)
            });
            self.set_focus(target.as_deref().unwrap_or(""));
        }
    }
    pub(crate) fn workspace_roving(&self, key: &str) -> Option<&View<C::Message>> {
        self.view
            .ancestor_with(key, &|v| matches!(v.workspace, Some(R::Toolbar(_))))
    }
    pub fn blocked_pointer(&mut self, target: Option<&str>) -> bool {
        if self.dialog_focus.key.is_none() || target.is_some_and(|key| self.in_scope(key)) {
            return false;
        }
        self.view.visit_mut(&mut |v| {
            v.interaction.pressed = false;
            if let Some(i) = &mut v.input {
                i.dragging = false;
            }
        });
        true
    }
}
impl<C: Component> Application<C>
where
    C::Message: Clone,
{
    pub fn workspace_command(&mut self, command: K) -> Result<bool> {
        let focused = self.focused_key().map(String::from);
        if matches!(command, K::First | K::Last) {
            if let Some(bar) = focused
                .as_deref()
                .and_then(|key| self.workspace_roving(key))
            {
                let items: Vec<_> = bar
                    .children
                    .iter()
                    .filter(|v| v.focusable() && !v.interaction.disabled)
                    .collect();
                if let Some(item) = if command == K::First {
                    items.first()
                } else {
                    items.last()
                } {
                    let key = item.key.clone();
                    self.set_focus(&key);
                    return Ok(true);
                }
            }
        }

        let group = focused
            .as_deref()
            .and_then(|key| {
                self.view
                    .ancestor_with(key, &|v| matches!(v.workspace, Some(R::Tabs { .. })))
            })
            .filter(|v| self.in_scope(&v.key))
            .or_else(|| {
                let mut nodes = Vec::new();
                self.view.flatten(&mut nodes);
                nodes
                    .into_iter()
                    .find(|v| matches!(v.workspace, Some(R::Tabs { .. })) && self.in_scope(&v.key))
            });
        let Some(group) = group else {
            return Ok(false);
        };
        if matches!(command, K::First | K::Last)
            && !focused.as_deref().is_some_and(|key| {
                self.view
                    .find(key)
                    .is_some_and(|v| matches!(v.workspace, Some(R::Tab { .. })))
            })
        {
            return Ok(false);
        }
        let headers = &group.children[0].children;
        let enabled: Vec<_> = headers.iter().filter(|v| !v.interaction.disabled).collect();
        if enabled.is_empty() {
            return Ok(true);
        }
        let current = enabled
            .iter()
            .position(|v| v.interaction.focused)
            .or_else(|| enabled.iter().position(|v| v.interaction.checked));
        let n = match command {
            K::First => 0,
            K::Last => enabled.len() - 1,
            K::NextTab => current.map_or(0, |n| (n + 1) % enabled.len()),
            K::PreviousTab => current.map_or(enabled.len() - 1, |n| {
                (n + enabled.len() - 1) % enabled.len()
            }),
        };
        let key = enabled[n].key.clone();
        let message = enabled[n].activation_message();
        self.commit_spinbox()?;
        self.set_focus(&key);
        if let Some(message) = message {
            self.update(message)?;
        }
        Ok(true)
    }
    pub(crate) fn workspace_navigation(&mut self, direction: Navigation) -> Result<bool> {
        let Some(key) = self.focused_key().map(String::from) else {
            return Ok(false);
        };
        if self.view.find(&key).is_some_and(|v| v.split.is_some()) {
            return self.splitter_command(match direction {
                Navigation::Left | Navigation::Up => crate::SliderCommand::Decrease,
                Navigation::Right | Navigation::Down => crate::SliderCommand::Increase,
            });
        }
        if let Some(bar) = self.workspace_roving(&key) {
            if matches!(direction, Navigation::Left | Navigation::Right) {
                let items: Vec<_> = bar
                    .children
                    .iter()
                    .filter(|v| v.focusable() && !v.interaction.disabled)
                    .collect();
                if items.is_empty() {
                    return Ok(true);
                }
                let current = items.iter().position(|v| v.key == key).unwrap_or(0);
                let next = if direction == Navigation::Right {
                    (current + 1) % items.len()
                } else {
                    (current + items.len() - 1) % items.len()
                };
                let target = items[next].key.clone();
                self.set_focus(&target);
                return Ok(true);
            }
        }
        if self
            .view
            .find(&key)
            .is_some_and(|v| matches!(v.workspace, Some(R::Tab { .. })))
        {
            return match direction {
                Navigation::Left => self.workspace_command(K::PreviousTab),
                Navigation::Right => self.workspace_command(K::NextTab),
                _ => Ok(true),
            };
        }
        Ok(false)
    }
    pub(crate) fn workspace_key(&mut self, key: Key) -> Result<bool> {
        if key.character == '\t' && matches!(key.modifiers, 2 | 3) {
            return self.workspace_command(if key.modifiers == 2 {
                K::NextTab
            } else {
                K::PreviousTab
            });
        }
        if key.character == '\u{1b}' && key.modifiers == 0 {
            if let Some(handle) = self.dragging_splitter_key().map(String::from) {
                self.view.visit_mut(&mut |v| {
                    if v.key == handle {
                        v.interaction.pressed = false;
                    }
                });
                return Ok(true);
            }
            if let Some(scope) = self.dialog_focus.key.clone() {
                let message = self
                    .view
                    .find(&scope)
                    .and_then(|v| v.dialog.as_ref())
                    .and_then(|d| d.cancel.clone());
                if let Some(message) = message {
                    self.update(message)?;
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(crate) fn dialog_default(&mut self, key: Key) -> Result<bool> {
        if key.modifiers != 0 || !matches!(key.character, '\n' | '\r') {
            return Ok(false);
        }
        let Some(scope) = self.dialog_focus.key.as_deref() else {
            return Ok(false);
        };
        if self
            .focused_key()
            .and_then(|key| self.view.find(key))
            .is_some_and(|v| {
                v.activation.is_some() || v.input.as_ref().is_some_and(|i| i.editor.is_multiline())
            })
        {
            return Ok(false);
        }
        let message = self
            .view
            .find(scope)
            .and_then(|v| v.dialog.as_ref())
            .and_then(|d| d.default.as_ref())
            .and_then(|key| self.view.find(key))
            .filter(|v| !v.interaction.disabled)
            .and_then(View::activation_message);
        if let Some(message) = message {
            self.update(message)?;
        }
        Ok(true)
    }
}
