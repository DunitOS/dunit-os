//! Controlled integer editing. Draft text is transient; the model owns the value.
use crate::{
    Application, Component, InputOptions, Key, PointerEvent, Result, SliderCommand, SliderOptions,
    TextEditor, View,
};
use alloc::string::{String, ToString};
pub use dunit_widgets::spinbox::SpinButton;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpinboxOptions {
    pub range: SliderOptions,
    pub read_only: bool,
}
impl SpinboxOptions {
    pub fn new(min: i64, max: i64, step: i64) -> Self {
        Self {
            range: SliderOptions::new(min, max, step),
            read_only: false,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpinboxRange {
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub value: i64,
    pub read_only: bool,
}
#[derive(Debug, Clone)]
pub(crate) struct SpinboxBinding<M> {
    pub value: i64,
    pub options: SpinboxOptions,
    pub on_change: fn(i64) -> M,
    pub pressed: Option<SpinButton>,
}
impl<M> SpinboxBinding<M> {
    pub(crate) fn draft_value(&self, text: &str) -> i64 {
        text.parse::<i128>()
            .ok()
            .map_or(self.value, |n| self.options.range.snap(n))
    }
}
impl<M> PartialEq for SpinboxBinding<M> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
            && self.options == other.options
            && self.pressed == other.pressed
            && core::ptr::fn_addr_eq(self.on_change, other.on_change)
    }
}
pub(crate) fn valid_draft(text: &str) -> bool {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    digits.bytes().all(|b| b.is_ascii_digit())
}
impl<M> View<M> {
    pub fn spinbox(
        key: impl Into<String>,
        label: impl Into<String>,
        value: i64,
        options: SpinboxOptions,
        on_change: fn(i64) -> M,
    ) -> Self {
        let text = value.to_string();
        let mut view = Self::new(
            key,
            dunit_ui::Kind::Element("Spinbox".into()),
            Some(text.clone()),
            alloc::vec::Vec::new(),
        );
        view.input = Some(crate::input::InputBinding {
            label: label.into(),
            options: InputOptions {
                max_chars: 20,
                read_only: options.read_only,
                ..InputOptions::default()
            },
            editor: TextEditor::new(text),
            on_change: None,
            dragging: false,
            caret_visible: true,
            caret_revision: 0,
        });
        view.spinbox = Some(SpinboxBinding {
            value,
            options,
            on_change,
            pressed: None,
        });
        view
    }
    pub fn spinbox_range(&self) -> Option<SpinboxRange> {
        self.spinbox.as_ref().map(|s| SpinboxRange {
            min: s.options.range.min,
            max: s.options.range.max,
            step: s.options.range.step,
            value: s.value,
            read_only: s.options.read_only,
        })
    }
}
impl<C: Component> Application<C> {
    /// Commit the focused draft, clamping and snapping to the configured grid.
    /// Empty/sign-only drafts revert; unchanged values do not dispatch messages.
    pub fn commit_spinbox(&mut self) -> Result<bool> {
        self.finish_spinbox(None, false)
    }
    pub fn spinbox_step(&mut self, command: SliderCommand) -> Result<bool> {
        self.finish_spinbox(Some(command), false)
    }
    fn finish_spinbox(&mut self, command: Option<SliderCommand>, cancel: bool) -> Result<bool> {
        let mut message = None;
        let mut key = None;
        self.view.visit_mut(&mut |v| {
            if !v.interaction.focused || v.interaction.disabled {
                return;
            }
            let (Some(s), Some(input)) = (&v.spinbox, &v.input) else {
                return;
            };
            key = Some(v.key.clone());
            if cancel || s.options.read_only {
                return;
            }
            let base = s.draft_value(input.editor.text());
            let value = command.map_or(base, |c| s.options.range.command(base, c));
            if value != s.value {
                message = Some((s.on_change)(value));
            }
        });
        let result = if let Some(message) = message {
            self.update(message)
        } else {
            Ok(())
        };
        // Render the actual model value even when the proposal was rejected.
        self.view.visit_mut(&mut |v| {
            if Some(&v.key) != key.as_ref() {
                return;
            }
            if let (Some(s), Some(input)) = (&mut v.spinbox, &mut v.input) {
                input.editor = TextEditor::new(s.value.to_string());
                input.caret_visible = true;
                input.caret_revision = input.caret_revision.wrapping_add(1);
                input.dragging = false;
                s.pressed = None;
                v.interaction.pressed = false;
            }
        });
        result?;
        Ok(key.is_some())
    }
    pub(crate) fn spinbox_key(&mut self, key: Key) -> Result<bool> {
        if key.character == '\t' && key.modifiers & !1 == 0 {
            self.commit_spinbox()?;
            return Ok(false);
        }
        if key.modifiers != 0 {
            return Ok(false);
        }
        match key.character {
            '\n' | '\r' => self.commit_spinbox(),
            '\u{1b}' => self.finish_spinbox(None, true),
            _ => Ok(false),
        }
    }
    pub fn pressed_spinbox_key(&self) -> Option<&str> {
        let mut nodes = alloc::vec::Vec::new();
        self.view.flatten(&mut nodes);
        nodes
            .into_iter()
            .find(|v| {
                !v.interaction.disabled
                    && v.interaction.pressed
                    && v.spinbox.as_ref().is_some_and(|s| s.pressed.is_some())
            })
            .map(|v| v.key.as_str())
    }
    /// Buttons activate on release in the same half. Leaving permanently cancels
    /// capture; text-area events continue through the ordinary selection path.
    pub fn spinbox_pointer(
        &mut self,
        event: PointerEvent,
        target: Option<&str>,
        button: Option<SpinButton>,
    ) -> Result<bool>
    where
        C::Message: Clone,
    {
        if event != PointerEvent::Down && self.dragging_input_key().is_some() {
            return Ok(false);
        }
        let capture = if event == PointerEvent::Down {
            None
        } else {
            self.pressed_spinbox_key().map(String::from)
        };
        let selected = capture.as_deref().or(target);
        let mut enabled = false;
        let mut pressed = None;
        self.view.visit_mut(&mut |v| {
            if Some(v.key.as_str()) == selected && !v.interaction.disabled {
                if let Some(s) = &v.spinbox {
                    enabled = !s.options.read_only;
                    pressed = s.pressed;
                }
            }
        });
        if !enabled || (capture.is_none() && (event != PointerEvent::Down || button.is_none())) {
            return Ok(false);
        }
        let key = String::from(selected.unwrap());
        let activate =
            event == PointerEvent::Up && target == Some(key.as_str()) && button == pressed;
        self.pointer(event, target)?;
        self.view.visit_mut(&mut |v| {
            if v.key != key {
                return;
            }
            if let (Some(s), Some(input)) = (&mut v.spinbox, &mut v.input) {
                input.dragging = false;
                s.pressed = if event == PointerEvent::Down {
                    button
                } else if event == PointerEvent::Move
                    && target == Some(key.as_str())
                    && button == pressed
                {
                    pressed
                } else {
                    None
                };
                v.interaction.pressed = s.pressed.is_some();
            }
        });
        if activate {
            self.spinbox_step(match pressed.unwrap() {
                SpinButton::Increase => SliderCommand::Increase,
                SpinButton::Decrease => SliderCommand::Decrease,
            })?;
        }
        Ok(true)
    }
}
