//! Controlled text fields and editing dispatch.
use crate::{Application, Component, Key, PointerEvent, Result, View};
use alloc::string::String;
pub use dunit_widgets::text_input::{EditCommand, TextEditor, MAX_INPUT_BYTES};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputOptions {
    pub placeholder: String,
    pub password: bool,
    /// Wrap multiline visual rows without changing the controlled text.
    pub soft_wrap: bool,
    pub read_only: bool,
    /// Unicode scalar count, additionally bounded by MAX_INPUT_BYTES.
    pub max_chars: usize,
}
impl Default for InputOptions {
    fn default() -> Self {
        Self {
            placeholder: String::new(),
            password: false,
            soft_wrap: false,
            read_only: false,
            max_chars: MAX_INPUT_BYTES,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct InputBinding<M> {
    pub affinity: dunit_render::CaretAffinity,
    pub visual_goal: Option<f32>,
    pub owner: Option<String>,
    pub label: String,
    pub options: InputOptions,
    pub editor: TextEditor,
    /// None for a spinbox: numeric drafts commit through its integer callback.
    pub on_change: Option<crate::callback::Callback<String, M>>,
    pub dragging: bool,
    pub caret_visible: bool,
    pub caret_revision: u64,
}
impl<M> PartialEq for InputBinding<M> {
    fn eq(&self, other: &Self) -> bool {
        self.affinity == other.affinity
            && self.visual_goal == other.visual_goal
            && self.owner == other.owner
            && self.label == other.label
            && self.options == other.options
            && self.editor == other.editor
            && self.on_change == other.on_change
            && self.dragging == other.dragging
            && self.caret_visible == other.caret_visible
            && self.caret_revision == other.caret_revision
    }
}
impl<M> View<M> {
    pub(crate) fn focusable(&self) -> bool {
        self.list.is_some()
            || self.split.is_some()
            || self.popup.is_some()
            || self.activation.is_some()
            || self.slider.is_some()
            || self.input.is_some()
            || self.scroll_label.is_some()
    }
    pub fn text_editor(&self) -> Option<&TextEditor> {
        self.input.as_ref().map(|i| &i.editor)
    }
}
impl<C: Component> Application<C> {
    /// Dispatch a command to the focused field. Returns consumed, even for a
    /// rejected edit, so text never falls through to application shortcuts.
    pub fn edit(&mut self, command: EditCommand) -> Result<bool> {
        match command {
            EditCommand::Up { extend: false } => {
                if self.spinbox_step(crate::SliderCommand::Increase)? {
                    return Ok(true);
                }
            }
            EditCommand::Down { extend: false } => {
                if self.spinbox_step(crate::SliderCommand::Decrease)? {
                    return Ok(true);
                }
            }
            _ => {}
        }
        let mut message = None;
        let mut previous = None;
        let mut rename_change = None;
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
            let old_visual = (input.affinity, input.visual_goal);
            input.affinity = dunit_render::CaretAffinity::Downstream;
            input.visual_goal = None;
            input
                .editor
                .apply(command, input.options.max_chars, input.options.read_only);
            input.caret_visible = true;
            if old.text() != input.editor.text() {
                if let Some(owner) = &input.owner {
                    rename_change = Some((owner.clone(), String::from(input.editor.text())));
                    previous = Some((v.key.clone(), old, old_visual));
                } else if let Some(on_change) = &input.on_change {
                    message = Some(on_change.call(input.editor.text().into()));
                    previous = Some((v.key.clone(), old, old_visual));
                } else if !crate::spinbox::valid_draft(input.editor.text()) {
                    input.editor = old;
                }
            }
        });
        if let Some((owner, value)) = rename_change {
            message = Some(self.rename_change(&owner, value)?);
        }
        if let Some(message) = message {
            let result = self.update(message);
            if let Some((key, old, old_visual)) = previous {
                self.view.visit_mut(&mut |v| {
                    if v.key == key
                        && !v.interaction.disabled
                        && (result.is_err() || v.text.as_deref() == Some(old.text()))
                    {
                        if let Some(input) = &mut v.input {
                            if input.editor.is_multiline() == old.is_multiline() {
                                input.editor = old.clone();
                                input.affinity = old_visual.0;
                                input.visual_goal = old_visual.1;
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
    /// A scrollbar changes viewport/focus without initiating text selection.
    pub fn scrollbar_pointer(&mut self, event: PointerEvent, key: &str) -> Result<bool>
    where
        C::Message: Clone,
    {
        self.dispatch_input(
            Some(key),
            crate::UiEventKind::Pointer {
                event,
                position: None,
                button: 0,
                modifiers: 0,
            },
            |app| app.scrollbar_pointer_default(event, key),
        )
    }
    fn scrollbar_pointer_default(&mut self, event: PointerEvent, key: &str) -> Result<bool>
    where
        C::Message: Clone,
    {
        let changed = self.pointer(event, Some(key))?;
        self.view.visit_mut(&mut |v| {
            if let Some(input) = &mut v.input {
                input.dragging = false;
            }
        });
        Ok(changed)
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
        self.dispatch_input(
            target,
            crate::UiEventKind::Pointer {
                event,
                position: None,
                button: 0,
                modifiers: 0,
            },
            |app| app.pointer_at_default(event, target, position),
        )
    }
    fn pointer_at_default(
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
                        input.affinity = dunit_render::CaretAffinity::Downstream;
                        input.visual_goal = None;
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
    /// Extend an existing text capture without synthesizing pointer routing.
    pub fn drag_selection(&mut self, key: &str, index: usize) -> bool {
        let mut changed = false;
        self.view.visit_mut(&mut |v| {
            if v.key != key || !v.interaction.focused || v.interaction.disabled {
                return;
            }
            if let Some(input) = &mut v.input {
                if input.dragging {
                    input.affinity = dunit_render::CaretAffinity::Downstream;
                    input.visual_goal = None;
                    input.editor.place(index, true);
                    input.caret_revision = input.caret_revision.wrapping_add(1);
                    input.caret_visible = true;
                    changed = true;
                }
            }
        });
        changed
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

impl<C: Component> Application<C>
where
    C::Message: Clone,
{
    pub(crate) fn submit_key(&mut self, key: Key) -> Result<bool> {
        if key.modifiers != 0 || !matches!(key.character, '\r' | '\n') {
            return Ok(false);
        }
        let message = self
            .focused_key()
            .and_then(|k| self.view.find(k))
            .and_then(|v| v.submit.clone());
        if let Some(m) = message {
            self.update(m)?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl<C: Component> Application<C> {
    /// Visual navigation changes selection/caret only, preserving model/history.
    pub fn place_visual_input(
        &mut self,
        key: &str,
        target: dunit_render::TextPosition,
        extend: bool,
        goal: Option<f32>,
    ) -> bool {
        let mut changed = false;
        self.view.visit_mut(&mut |v| {
            if v.key != key || !v.interaction.focused || v.interaction.disabled {
                return;
            }
            if let Some(input) = &mut v.input {
                input.editor.place(target.index, extend);
                input.affinity = if input.editor.cursor() == target.index {
                    target.affinity
                } else {
                    dunit_render::CaretAffinity::Downstream
                };
                input.visual_goal = goal.filter(|x| x.is_finite() && *x >= 0.);
                input.caret_revision = input.caret_revision.wrapping_add(1);
                input.caret_visible = true;
                changed = true;
            }
        });
        changed
    }
    /// Resolve the visual side of a wrap boundary after a pointer gesture.
    pub fn set_input_affinity(&mut self, key: &str, position: dunit_render::TextPosition) -> bool {
        let mut changed = false;
        self.view.visit_mut(&mut |v| {
            if v.key != key || !v.interaction.focused || v.interaction.disabled {
                return;
            }
            if let Some(input) = &mut v.input {
                if input.editor.cursor() == position.index {
                    changed = input.affinity != position.affinity || input.visual_goal.is_some();
                    input.affinity = position.affinity;
                    input.visual_goal = None;
                    if changed {
                        input.caret_revision = input.caret_revision.wrapping_add(1);
                        input.caret_visible = true;
                    }
                }
            }
        });
        changed
    }
}
