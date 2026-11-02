//! Negotiated native input lifecycle; cooked legacy key calls remain immediate.
use crate::{Application, Component, Key, Result};
use alloc::string::String;
#[derive(Clone)]
pub(crate) struct KeyboardPress {
    scan: i32,
    target: String,
    focus: Option<String>,
    kind: dunit_ui::Kind,
    cancelled: bool,
}
impl<C: Component> Application<C> {
    /// Whether this application's window currently owns keyboard focus.
    pub fn window_focused(&self) -> bool {
        self.window_focused
    }
    /// Focus changes cancel transient gestures without committing model edits.
    /// A valid stable focus owner is restored when the window regains focus.
    pub fn window_focus(&mut self, focused: bool) -> bool {
        if self.window_focused == focused {
            return false;
        }
        if !focused {
            self.suspended_focus = self.focused_key().map(String::from);
            self.window_focused = false;
            self.keyboard_press = None;
            self.held_activation_scans.fill(false);
            self.cancel_pointer_interaction();
            self.close_popup();
            self.dismiss_tooltip();
            self.view.visit_mut(&mut |v| {
                v.interaction.focused = false;
                v.interaction.hovered = false;
                if let Some(i) = &mut v.input {
                    i.dragging = false;
                    i.caret_visible = false;
                }
            });
        } else {
            self.window_focused = true;
            let target = self.suspended_focus.take().filter(|key| {
                self.in_scope(key)
                    && self
                        .view
                        .find(key)
                        .is_some_and(|v| v.focusable() && !v.interaction.disabled)
            });
            if let Some(target) = target {
                self.set_focus(&target);
            } else {
                self.sync_dialog(None);
            }
        }
        true
    }
    pub(crate) fn keyboard_activation_key(&self) -> Option<&str> {
        self.keyboard_press
            .as_ref()
            .filter(|p| !p.cancelled)
            .map(|p| p.target.as_str())
    }
    /// Cancel a pending activation but consume repeated makes until its break.
    pub fn cancel_keyboard_activation(&mut self) -> bool {
        let Some(press) = &mut self.keyboard_press else {
            return false;
        };
        press.cancelled = true;
        let key = press.target.clone();
        let mut changed = false;
        self.view.visit_mut(&mut |v| {
            if v.key == key {
                changed |= v.interaction.pressed;
                v.interaction.pressed = false;
            }
        });
        changed
    }
    pub(crate) fn reconcile_keyboard_press(&mut self) {
        let valid = self.keyboard_press.as_ref().is_none_or(|p| {
            self.window_focused
                && !p.cancelled
                && self.focused_key() == p.focus.as_deref()
                && self.in_scope(&p.target)
                && self.view.find(&p.target).is_some_and(|v| {
                    v.kind == p.kind && !v.interaction.disabled && v.interaction.pressed
                })
        });
        if !valid {
            self.cancel_keyboard_activation();
        }
    }
    /// Arm a push/checkable control on native make, activate once on break.
    /// Returns false for other keys/controls so normal native defaults continue.
    pub fn key_press(&mut self, scancode: i32, key: Key) -> Result<bool> {
        if !self.window_focused {
            return Ok(true);
        }
        let Some(scan) = usize::try_from(scancode)
            .ok()
            .filter(|&scan| scan < self.held_activation_scans.len())
        else {
            return Ok(false);
        };
        // Physical held state outlives the single current visual activation.
        // A second activation key must not erase a cancelled first key's
        // tombstone and let its later repeat look like a fresh press.
        if self.held_activation_scans[scan] {
            return Ok(true);
        }
        if key.character == '\u{1b}' {
            self.cancel_keyboard_activation();
            return Ok(false);
        }
        if key.modifiers != 0 || !matches!(key.character, ' ' | '\r' | '\n') {
            return Ok(false);
        }
        if let Some(popup) = self.popup_state() {
            let target = popup.owner.clone();
            if let Some(v) = self.view.find(&target) {
                self.held_activation_scans[scan] = true;
                self.keyboard_press = Some(KeyboardPress {
                    scan: scancode,
                    target,
                    focus: self.focused_key().map(String::from),
                    kind: v.kind.clone(),
                    cancelled: true,
                });
            }
            return Ok(false);
        }
        let focus = self.focused_key().map(String::from);
        let target = focus
            .as_deref()
            .and_then(|k| self.view.find(k))
            .filter(|v| v.activation.is_some() || v.popup.is_some())
            .map(|v| v.key.clone());
        let target = target.or_else(|| {
            if key.character == ' ' {
                return None;
            }
            let input = focus.as_deref().and_then(|k| self.view.find(k));
            if input.is_some_and(|v| {
                v.rename.as_ref().is_some()
                    || v.spinbox.is_some()
                    || v.submit.is_some()
                    || v.input
                        .as_ref()
                        .is_some_and(|i| i.owner.is_some() || i.editor.is_multiline())
            }) {
                return None;
            }
            self.active_dialog()
                .and_then(|k| self.view.find(k))
                .and_then(|v| v.dialog.as_ref())
                .and_then(|d| d.default.clone())
        });
        let Some(target) = target else {
            return Ok(false);
        };
        let Some(view) = self
            .view
            .find(&target)
            .filter(|v| !v.interaction.disabled && (v.activation.is_some() || v.popup.is_some()))
        else {
            return Ok(false);
        };
        let kind = view.kind.clone();
        self.cancel_keyboard_activation();
        self.held_activation_scans[scan] = true;
        self.keyboard_press = Some(KeyboardPress {
            scan: scancode,
            target: target.clone(),
            focus,
            kind,
            cancelled: false,
        });
        self.view.visit_mut(&mut |v| {
            if v.key == target {
                v.interaction.pressed = true;
            }
        });
        Ok(true)
    }
    /// A stray release, a cancelled press, or a rebuilt/disabled target cannot
    /// activate a control. Modifiers at release do not change the original key.
    pub fn key_release(&mut self, scancode: i32, _modifiers: u8) -> Result<bool>
    where
        C::Message: Clone,
    {
        let was_held = usize::try_from(scancode)
            .ok()
            .and_then(|scan| self.held_activation_scans.get_mut(scan))
            .is_some_and(|held| core::mem::take(held));
        if !self
            .keyboard_press
            .as_ref()
            .is_some_and(|p| p.scan == scancode)
        {
            return Ok(was_held);
        }
        let press = self.keyboard_press.take().unwrap();
        let target = self.view.find(&press.target).filter(|v| {
            self.window_focused
                && !press.cancelled
                && self.focused_key() == press.focus.as_deref()
                && self.in_scope(&press.target)
                && v.kind == press.kind
                && !v.interaction.disabled
                && v.interaction.pressed
        });
        let popup = target.is_some_and(|v| v.popup.is_some());
        let message = target.and_then(|v| v.activation_message());
        self.view.visit_mut(&mut |v| {
            if v.key == press.target {
                v.interaction.pressed = false;
            }
        });
        if popup {
            self.trigger_popup(&press.target);
        } else if let Some(message) = message {
            self.update(message)?;
        }
        Ok(true)
    }
}
