//! Controlled text fields and editing dispatch.
use crate::{Application, Component, Key, PointerEvent, Result, View};
use alloc::string::String;
pub use dunit_widgets::text_input::{EditCommand, TextEditor, MAX_INPUT_BYTES};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputOptions {
    pub placeholder: String,
    pub password: bool,
    pub read_only: bool,
    /// Unicode scalar count, additionally bounded by MAX_INPUT_BYTES.
    pub max_chars: usize,
}
impl Default for InputOptions {
    fn default() -> Self {
        Self {
            placeholder: String::new(),
            password: false,
            read_only: false,
            max_chars: MAX_INPUT_BYTES,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct InputBinding<M> {
    pub label: String,
    pub options: InputOptions,
    pub editor: TextEditor,
    pub on_change: fn(String) -> M,
    pub dragging: bool,
    pub caret_visible: bool,
    pub caret_revision: u64,
}
impl<M> PartialEq for InputBinding<M> {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label
            && self.options == other.options
            && self.editor == other.editor
            && core::ptr::fn_addr_eq(self.on_change, other.on_change)
            && self.dragging == other.dragging
            && self.caret_visible == other.caret_visible
            && self.caret_revision == other.caret_revision
    }
}
impl<M> View<M> {
    pub(crate) fn focusable(&self) -> bool {
        self.activation.is_some() || self.input.is_some() || self.scroll_label.is_some()
    }
    pub fn text_editor(&self) -> Option<&TextEditor> {
        self.input.as_ref().map(|i| &i.editor)
    }
}
impl<C: Component> Application<C> {
    /// Dispatch a command to the focused field. Returns consumed, even for a
    /// rejected edit, so text never falls through to application shortcuts.
    pub fn edit(&mut self, command: EditCommand) -> Result<bool> {
        let mut message = None;
        let mut previous = None;
        let mut consumed = false;
        self.view.visit_mut(&mut |v| {
            if !v.interaction.focused || v.interaction.disabled {
                return;
            }
            let Some(input) = &mut v.input else {
                return;
            };
            consumed = true;
            input.caret_revision = input.caret_revision.wrapping_add(1);
            let old = input.editor.clone();
            input
                .editor
                .apply(command, input.options.max_chars, input.options.read_only);
            input.caret_visible = true;
            if old.text() != input.editor.text() {
                message = Some((input.on_change)(input.editor.text().into()));
                previous = Some((v.key.clone(), old));
            }
        });
        if let Some(message) = message {
            let result = self.update(message);
            if let Some((key, old)) = previous {
                self.view.visit_mut(&mut |v| {
                    if v.key == key
                        && !v.interaction.disabled
                        && (result.is_err() || v.text.as_deref() == Some(old.text()))
                    {
                        if let Some(input) = &mut v.input {
                            if input.editor.is_multiline() == old.is_multiline() {
                                input.editor = old.clone();
                            }
                        }
                    }
                });
            }
            result?;
        }
        Ok(consumed)
    }
    pub(crate) fn text_key(&mut self, key: Key) -> Result<bool> {
        let has_input = self.focused_input_key().is_some();
        if !has_input {
            return Ok(false);
        }
        let ctrl = key.modifiers & 2 != 0;
        let shift = key.modifiers & 1 != 0;
        if key.modifiers & !3 != 0 {
            return Ok(false);
        }
        let mut multiline = false;
        self.view.visit_mut(&mut |v| {
            if v.interaction.focused {
                multiline = v.input.as_ref().is_some_and(|i| i.editor.is_multiline());
            }
        });
        let command = if ctrl {
            match key.character {
                'a' | 'A' | '\u{1}' => Some(EditCommand::SelectAll),
                'z' | 'Z' | '\u{1a}' if shift => Some(EditCommand::Redo),
                'z' | 'Z' | '\u{1a}' => Some(EditCommand::Undo),
                'y' | 'Y' | '\u{19}' => Some(EditCommand::Redo),
                '\u{8}' => Some(EditCommand::Backspace { word: true }),
                '\u{7f}' => Some(EditCommand::Delete { word: true }),
                _ => None,
            }
        } else {
            match key.character {
                '\n' | '\r' if multiline => Some(EditCommand::Insert('\n')),
                '\u{8}' => Some(EditCommand::Backspace { word: false }),
                '\u{7f}' => Some(EditCommand::Delete { word: false }),
                c if !c.is_control() => Some(EditCommand::Insert(c)),
                _ => None,
            }
        };
        if let Some(command) = command {
            return self.edit(command);
        }
        Ok(false)
    }
    pub fn focused_input_key(&self) -> Option<&str> {
        let mut nodes = alloc::vec::Vec::new();
        self.view.flatten(&mut nodes);
        nodes
            .into_iter()
            .find(|v| v.input.is_some() && v.interaction.focused && !v.interaction.disabled)
            .map(|v| v.key.as_str())
    }
    pub fn dragging_input_key(&self) -> Option<&str> {
        let mut nodes = alloc::vec::Vec::new();
        self.view.flatten(&mut nodes);
        nodes
            .into_iter()
            .find(|v| v.input.as_ref().is_some_and(|i| i.dragging) && !v.interaction.disabled)
            .map(|v| v.key.as_str())
    }
    /// `position` comes from Renderer::input_index, including captured dragging.
    pub fn pointer_at(
        &mut self,
        event: PointerEvent,
        target: Option<&str>,
        position: Option<(&str, usize)>,
    ) -> Result<bool>
    where
        C::Message: Clone,
    {
        let mut changed = self.pointer(event, target)?;
        if let Some((key, index)) = position {
            if event != PointerEvent::Leave {
                self.view.visit_mut(&mut |v| {
                    if v.key != key || !v.interaction.focused || v.interaction.disabled {
                        return;
                    }
                    if let Some(input) = &mut v.input {
                        input.caret_revision = input.caret_revision.wrapping_add(1);
                        input.editor.place(index, event != PointerEvent::Down);
                        // Revision-only activity must schedule a render to reveal the caret.
                        changed = true;
                        input.caret_visible = true;
                    }
                });
            }
        }
        Ok(changed)
    }
    pub fn blink_caret(&mut self) -> bool {
        let mut changed = false;
        self.view.visit_mut(&mut |v| {
            if v.interaction.focused && !v.interaction.disabled {
                if let Some(input) = &mut v.input {
                    input.caret_visible = !input.caret_visible;
                    changed = true;
                }
            }
        });
        changed
    }
}
