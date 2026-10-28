//! Control interaction and stable-key reconciliation, independent of native I/O.
use crate::{Application, Component, Key, Result, View};
use alloc::{collections::BTreeMap, vec::Vec};

/// The renderer supplies the topmost stable key, or None outside the content.
/// A leave permanently cancels a press, even if the pointer later returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEvent {
    Move,
    Down,
    Up,
    Leave,
}

impl<M> View<M> {
    pub(crate) fn reconcile(&mut self, old: &Self) {
        let mut nodes = Vec::new();
        old.flatten(&mut nodes);
        let states: BTreeMap<_, _> = nodes
            .into_iter()
            .filter(|v| v.focusable())
            .map(|v| (v.key.as_str(), v))
            .collect();
        self.restore(&states);
    }
    fn restore(&mut self, states: &BTreeMap<&str, &Self>) {
        if self.focusable() && !self.interaction.disabled {
            if let Some(old) = states.get(self.key.as_str()) {
                if old.kind == self.kind
                    && old.radio_group == self.radio_group
                    && !old.interaction.disabled
                {
                    // Checked is model-owned. Restore only transient interaction.
                    self.interaction.hovered = old.interaction.hovered;
                    self.interaction.focused = old.interaction.focused;
                    self.interaction.pressed = old.interaction.pressed;
                    if let (Some(input), Some(previous)) = (&mut self.input, &old.input) {
                        input.editor = previous.editor.clone();
                        input.editor.sync(self.text.as_deref().unwrap_or(""));
                        input.dragging = previous.dragging;
                    }
                }
            }
        }
        for child in &mut self.children {
            child.restore(states);
        }
    }
    pub(crate) fn visit_mut(&mut self, visit: &mut impl FnMut(&mut Self)) {
        visit(self);
        for child in &mut self.children {
            child.visit_mut(visit);
        }
    }
}

impl<C: Component> Application<C> {
    /// Route a pointer event using Renderer::hit_key. Only enabled controls
    /// participate. Use pointer_at for text cursor coordinates. Returns whether state changed or a message was dispatched.
    pub fn pointer(&mut self, event: PointerEvent, target: Option<&str>) -> Result<bool>
    where
        C::Message: Clone,
    {
        let mut message = None;
        let mut changed = false;
        self.view.visit_mut(&mut |v| {
            if !v.focusable() || v.interaction.disabled {
                return;
            }
            let old = v.interaction;
            let hit = event != PointerEvent::Leave && target == Some(v.key.as_str());
            v.interaction.hovered = hit;
            match event {
                PointerEvent::Down => {
                    v.interaction.focused = hit;
                    v.interaction.pressed = hit && v.activation.is_some();
                    if let Some(input) = &mut v.input {
                        input.dragging = hit;
                        changed |= hit && !input.caret_visible;
                        input.caret_visible = true;
                    }
                }
                PointerEvent::Up => {
                    if hit && v.interaction.pressed {
                        message = v.activation_message();
                    }
                    v.interaction.pressed = false;
                    if let Some(input) = &mut v.input {
                        input.dragging = false;
                    }
                }
                PointerEvent::Move | PointerEvent::Leave => {
                    if !hit {
                        v.interaction.pressed = false;
                    }
                    if event == PointerEvent::Leave {
                        if let Some(input) = &mut v.input {
                            input.dragging = false;
                        }
                    }
                }
            }
            changed |= old != v.interaction;
        });
        if let Some(message) = message {
            self.update(message)?;
            changed = true;
        }
        Ok(changed)
    }

    pub(crate) fn widget_key(&mut self, key: Key) -> Result<bool>
    where
        C::Message: Clone,
    {
        // Shift is bit 0 in the native keyboard modifier contract. Other
        // modifiers belong to application shortcuts, not button activation.
        if key.character == '\t' && key.modifiers & !1 == 0 {
            return Ok(self.tab_focus(key.modifiers == 1));
        }
        if key.modifiers != 0 {
            return Ok(false);
        }
        if key.character == '\u{1b}' {
            let mut cancelled = false;
            self.view.visit_mut(&mut |v| {
                cancelled |= v.interaction.pressed;
                v.interaction.pressed = false;
            });
            return Ok(cancelled);
        }
        if !matches!(key.character, ' ' | '\n' | '\r') {
            return Ok(false);
        }
        let mut message = None;
        let mut consumed = false;
        self.view.visit_mut(&mut |v| {
            if v.interaction.focused && !v.interaction.disabled && v.activation.is_some() {
                consumed = true;
                message = v.activation_message();
            }
            v.interaction.pressed = false;
        });
        if let Some(message) = message {
            self.update(message)?;
            return Ok(true);
        }
        Ok(consumed)
    }

    /// Stable identity of the currently focused button, if any.
    pub fn focused_key(&self) -> Option<&str> {
        let mut nodes = Vec::new();
        self.view.flatten(&mut nodes);
        nodes
            .into_iter()
            .find(|v| v.interaction.focused)
            .map(|v| v.key.as_str())
    }
}
