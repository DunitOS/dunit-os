//! Timed multi-click selection. Native adapters supply monotonic time and DSS
//! policy; editing/history and the authoritative model are never changed here.
use crate::{Application, Component, PointerEvent, Result, TextEditor};
use alloc::string::String;
use core::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextClickConfig {
    pub interval_ms: u64,
    pub distance: f32,
}
impl Default for TextClickConfig {
    fn default() -> Self {
        Self {
            interval_ms: 0,
            distance: 0.,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextClickSample {
    pub time_ms: u64,
    pub x: f32,
    pub y: f32,
    pub config: TextClickConfig,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSelectionUnit {
    Character,
    Word,
    Line,
}
#[derive(Clone, Debug)]
struct Press {
    key: String,
    text: String,
    sample: TextClickSample,
    count: u8,
}
#[derive(Clone, Debug)]
struct Capture {
    fixed_anchor: Option<usize>,
    key: String,
    text: String,
    unit: TextSelectionUnit,
    anchor: Range<usize>,
}
/// A click streak and one active selection capture, scoped to a stable field
/// identity and exact text. No background timer or synthetic click is required.
#[derive(Clone, Debug, Default)]
pub struct TextClickState {
    last: Option<Press>,
    capture: Option<Capture>,
}
impl TextClickState {
    /// Cancel on keyboard input, another control, window leave, resize or focus
    /// loss. Releasing a selection ends capture but retains the click streak.
    pub fn cancel(&mut self) {
        self.last = None;
        self.capture = None;
    }
    pub fn continues_click(&self,key:&str,text:&str,sample:TextClickSample)->bool{
        self.last.as_ref().is_some_and(|p|p.key==key&&p.text==text&&p.sample.config==sample.config&&sample.config.interval_ms>0&&sample.time_ms>=p.sample.time_ms&&sample.time_ms-p.sample.time_ms<=sample.config.interval_ms&&Self::nearby(p.sample,sample))
    }
    fn nearby(a: TextClickSample, b: TextClickSample) -> bool {
        if ![a.x, a.y, b.x, b.y, b.config.distance]
            .iter()
            .all(|n| n.is_finite())
            || b.config.distance < 0.
        {
            return false;
        }
        let dx = a.x as f64 - b.x as f64;
        let dy = a.y as f64 - b.y as f64;
        dx * dx + dy * dy <= (b.config.distance as f64) * (b.config.distance as f64)
    }
    fn press(&mut self, key: &str, text: &str, sample: TextClickSample) -> TextSelectionUnit {
        let count = self
            .last
            .as_ref()
            .filter(|_|self.continues_click(key,text,sample))
            .map_or(1, |p| p.count % 3 + 1);
        self.last = Some(Press {
            key: key.into(),
            text: text.into(),
            sample,
            count,
        });
        match count {
            2 => TextSelectionUnit::Word,
            3 => TextSelectionUnit::Line,
            _ => TextSelectionUnit::Character,
        }
    }
    fn movement(&mut self, sample: TextClickSample) {
        if self
            .last
            .as_ref()
            .is_some_and(|p| !Self::nearby(p.sample, sample))
        {
            self.last = None;
        }
    }
}
fn range(editor: &TextEditor, index: usize, unit: TextSelectionUnit) -> Range<usize> {
    // Public placement normalizes every untrusted offset to a UTF-8 boundary.
    // A fresh probe carries only text, never the potentially large undo stack.
    let mut probe = TextEditor::new(editor.text());
    probe.place(index, false);
    let index = probe.cursor();
    match unit {
        TextSelectionUnit::Character => index..index,
        TextSelectionUnit::Word => {
            probe.select_word(index);
            probe.selection()
        }
        TextSelectionUnit::Line => {
            let text = probe.text();
            let start = text[..index].rfind('\n').map_or(0, |n| n + 1);
            // Include LF so replacing a selected line removes its separator.
            let end = text[index..]
                .find('\n')
                .map_or(text.len(), |n| index + n + 1);
            start..end
        }
    }
}
fn place_range(editor: &mut TextEditor, anchor: usize, cursor: usize) {
    editor.place(anchor, false);
    editor.place(cursor, true);
}
impl<C: Component> Application<C> {
    /// Native pointer routing plus character/word/logical-line selection. A
    /// direct host caller supplies the same painted byte index and clock sample.
    pub fn pointer_text_at(
        &mut self,
        event: PointerEvent,
        target: Option<&str>,
        position: Option<(&str, usize)>,
        clicks: &mut TextClickState,
        sample: TextClickSample,
    ) -> Result<bool>
    where
        C::Message: Clone,
    {
        self.pointer_text_at_with_modifiers(event, target, position, clicks, sample, 0)
    }
    /// Native Shift extends the existing anchor; Ctrl selects a word and
    /// Ctrl+Shift extends the existing anchor to that word boundary.
    pub fn pointer_text_at_with_modifiers(
        &mut self,
        event: PointerEvent,
        target: Option<&str>,
        position: Option<(&str, usize)>,
        clicks: &mut TextClickState,
        sample: TextClickSample,
        modifiers: u8,
    ) -> Result<bool>
    where
        C::Message: Clone,
    {
        if modifiers & !15 != 0 {
            return Err(crate::UiError::InvalidInput);
        }
        let kind = crate::UiEventKind::Pointer {
            event,
            position: Some((sample.x, sample.y)),
            button: 0,
            modifiers,
        };
        self.dispatch_input(target, kind, |app| {
            app.pointer_text_at_default(event, target, position, clicks, sample, modifiers)
        })
    }
    fn pointer_text_at_default(
        &mut self,
        event: PointerEvent,
        target: Option<&str>,
        position: Option<(&str, usize)>,
        clicks: &mut TextClickState,
        sample: TextClickSample,
        modifiers: u8,
    ) -> Result<bool>
    where
        C::Message: Clone,
    {
        let was_dragging = self.dragging_input_key().map(String::from);
        let mut changed = self.pointer_at(event, target, None)?;
        if event == PointerEvent::Leave {
            clicks.cancel();
            return Ok(changed);
        }
        clicks.movement(sample);
        let Some((key, index)) = position else {
            if event == PointerEvent::Down {
                clicks.cancel();
            }
            if event == PointerEvent::Up {
                clicks.capture = None;
            }
            return Ok(changed);
        };
        if self.popup_state().is_some() || !self.in_scope(key) {
            clicks.cancel();
            return Ok(changed);
        }
        let valid = self.view.find(key).is_some_and(|v| {
            v.interaction.focused
                && !v.interaction.disabled
                && v.input.as_ref().is_some_and(|i| match event {
                    PointerEvent::Down | PointerEvent::Move => i.dragging,
                    PointerEvent::Up => was_dragging.as_deref() == Some(key),
                    PointerEvent::Leave => false,
                })
        });
        if !valid {
            clicks.cancel();
            return Ok(changed);
        }
        if event == PointerEvent::Down {
            self.view.visit_mut(&mut |v| {
                if v.key != key {
                    return;
                }
                if let Some(i) = &mut v.input {
                    if modifiers & 3 != 0 {
                        clicks.cancel();
                    }
                    let unit = if modifiers & 2 != 0 {
                        TextSelectionUnit::Word
                    } else if modifiers & 1 != 0 {
                        TextSelectionUnit::Character
                    } else {
                        clicks.press(key, i.editor.text(), sample)
                    };
                    let hit = range(&i.editor, index, unit);
                    let fixed_anchor = (modifiers & 1 != 0).then(|| i.editor.anchor());
                    let anchor = if modifiers & 1 != 0 {
                        let original = i.editor.anchor();
                        let endpoint = if index < original { hit.start } else { hit.end };
                        place_range(&mut i.editor, original, endpoint);
                        original..original
                    } else {
                        place_range(&mut i.editor, hit.start, hit.end);
                        hit
                    };
                    clicks.capture = Some(Capture {
                        fixed_anchor,
                        key: key.into(),
                        text: i.editor.text().into(),
                        unit,
                        anchor,
                    });
                    i.affinity = dunit_render::CaretAffinity::Downstream;
                    i.visual_goal = None;
                    i.caret_revision = i.caret_revision.wrapping_add(1);
                    i.caret_visible = true;
                    changed = true;
                }
            });
        } else {
            changed |= self.drag_text_selection_inner(key, index, clicks);
            if event == PointerEvent::Up {
                clicks.capture = None;
            }
        }
        Ok(changed)
    }
    fn drag_text_selection_inner(
        &mut self,
        key: &str,
        index: usize,
        clicks: &mut TextClickState,
    ) -> bool {
        let Some(capture) = clicks.capture.as_ref().filter(|c| c.key == key) else {
            return false;
        };
        let mut changed = false;
        let mut invalid = false;
        self.view.visit_mut(&mut |v| {
            if v.key != key {
                return;
            }
            if let Some(i) = &mut v.input {
                if !v.interaction.focused
                    || v.interaction.disabled
                    || i.editor.text() != capture.text
                {
                    invalid = true;
                    return;
                }
                let target = range(&i.editor, index, capture.unit);
                if let Some(anchor) = capture.fixed_anchor {
                    let endpoint = if index < anchor {
                        target.start
                    } else {
                        target.end
                    };
                    place_range(&mut i.editor, anchor, endpoint);
                } else if capture.unit == TextSelectionUnit::Character {
                    place_range(&mut i.editor, capture.anchor.start, target.start);
                } else if target.end <= capture.anchor.start {
                    place_range(&mut i.editor, capture.anchor.end, target.start);
                } else if target.start >= capture.anchor.end {
                    place_range(&mut i.editor, capture.anchor.start, target.end);
                } else {
                    place_range(&mut i.editor, capture.anchor.start, capture.anchor.end);
                }
                i.affinity = dunit_render::CaretAffinity::Downstream;
                i.visual_goal = None;
                i.caret_revision = i.caret_revision.wrapping_add(1);
                i.caret_visible = true;
                changed = true;
            }
        });
        if invalid {
            clicks.cancel();
        }
        changed
    }
    /// Edge-autoscroll uses the same word/line anchor as real pointer motion.
    pub fn drag_text_selection(
        &mut self,
        key: &str,
        index: usize,
        clicks: &mut TextClickState,
    ) -> bool {
        if self.dragging_input_key() != Some(key) || !self.in_scope(key) {
            clicks.cancel();
            return false;
        }
        if clicks.capture.is_some() {
            self.drag_text_selection_inner(key, index, clicks)
        } else {
            self.drag_selection(key, index)
        }
    }
}
