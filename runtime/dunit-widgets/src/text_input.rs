//! Pure single-line UTF-8 editing. Indices are byte offsets at scalar boundaries;
//! word operations use whitespace runs, not linguistic or grapheme segmentation.

use alloc::{string::String, vec::Vec};
use core::ops::Range;

pub const MAX_INPUT_BYTES: usize = 64 * 1024;
const HISTORY_LIMIT: usize = 64;

fn valid_character(ch: char) -> bool {
    !ch.is_control() && ch != '\u{2028}' && ch != '\u{2029}'
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCommand {
    Insert(char),
    Backspace { word: bool },
    Delete { word: bool },
    Left { extend: bool, word: bool },
    Right { extend: bool, word: bool },
    Home { extend: bool },
    End { extend: bool },
    SelectAll,
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEditor {
    value: Snapshot,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl TextEditor {
    /// Preserve the supplied value; callers can validate it with `valid_text`.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            value: Snapshot {
                text,
                cursor,
                anchor: cursor,
            },
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn valid_text(text: &str) -> bool {
        text.len() <= MAX_INPUT_BYTES && text.chars().all(valid_character)
    }

    pub fn text(&self) -> &str {
        &self.value.text
    }
    pub fn cursor(&self) -> usize {
        self.value.cursor
    }
    pub fn anchor(&self) -> usize {
        self.value.anchor
    }
    pub fn selection(&self) -> Range<usize> {
        self.cursor().min(self.anchor())..self.cursor().max(self.anchor())
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// External valid changes reset history. Invalid values are rejected atomically.
    pub fn sync(&mut self, value: &str) {
        if value == self.text() || !Self::valid_text(value) {
            return;
        }
        self.value.text = String::from(value);
        self.value.cursor = self.boundary(self.cursor());
        self.value.anchor = self.boundary(self.anchor());
        self.undo.clear();
        self.redo.clear();
    }

    fn boundary(&self, index: usize) -> usize {
        let mut index = index.min(self.text().len());
        while !self.text().is_char_boundary(index) {
            index -= 1;
        }
        index
    }

    pub fn place(&mut self, index: usize, extend: bool) -> bool {
        let index = self.boundary(index);
        let anchor = if extend { self.anchor() } else { index };
        let changed = index != self.cursor() || anchor != self.anchor();
        self.value.cursor = index;
        self.value.anchor = anchor;
        changed
    }

    pub fn select_all(&mut self) -> bool {
        let changed = self.anchor() != 0 || self.cursor() != self.text().len();
        self.value.anchor = 0;
        self.value.cursor = self.text().len();
        changed
    }

    /// Select the whitespace or non-whitespace run under the supplied offset.
    /// An offset at the end selects the preceding run.
    pub fn select_word(&mut self, index: usize) -> bool {
        if self.text().is_empty() {
            return false;
        }
        let index = self.boundary(index);
        let index = if index == self.text().len() {
            self.previous(index)
        } else {
            index
        };
        let whitespace = self.text()[index..].chars().next().unwrap().is_whitespace();
        let mut start = index;
        while start > 0 {
            let previous = self.previous(start);
            if self.text()[previous..]
                .chars()
                .next()
                .unwrap()
                .is_whitespace()
                != whitespace
            {
                break;
            }
            start = previous;
        }
        let mut end = index;
        for ch in self.text()[index..].chars() {
            if ch.is_whitespace() != whitespace {
                break;
            }
            end += ch.len_utf8();
        }
        let changed = self.anchor() != start || self.cursor() != end;
        self.value.anchor = start;
        self.value.cursor = end;
        changed
    }

    fn previous(&self, index: usize) -> usize {
        self.text()[..index]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self, index: usize) -> usize {
        self.text()[index..]
            .chars()
            .next()
            .map_or(index, |ch| index + ch.len_utf8())
    }
    fn word_left(&self, index: usize) -> usize {
        let mut result = index;
        let mut in_word = false;
        for (i, ch) in self.text()[..index].char_indices().rev() {
            if ch.is_whitespace() {
                if in_word {
                    break;
                }
            } else {
                in_word = true;
            }
            result = i;
        }
        result
    }
    fn word_right(&self, index: usize) -> usize {
        let mut result = index;
        let mut in_space = false;
        for ch in self.text()[index..].chars() {
            if ch.is_whitespace() {
                in_space = true;
            } else if in_space {
                break;
            }
            result += ch.len_utf8();
        }
        result
    }
    fn push(history: &mut Vec<Snapshot>, snapshot: Snapshot) {
        if history.len() == HISTORY_LIMIT {
            history.remove(0);
        }
        history.push(snapshot);
    }
    fn replace(&mut self, range: Range<usize>, insert: &str, max_chars: usize) -> bool {
        if self.text().len() - range.len() + insert.len() > MAX_INPUT_BYTES {
            return false;
        }
        let chars = self.text().chars().count() - self.text()[range.clone()].chars().count()
            + insert.chars().count();
        if chars > max_chars {
            return false;
        }
        if &self.text()[range.clone()] == insert {
            // Replacing selected text with itself still completes the typing
            // gesture, but must not create an undo entry or a model change.
            return self.place(range.start + insert.len(), false);
        }
        Self::push(&mut self.undo, self.value.clone());
        self.redo.clear();
        let cursor = range.start + insert.len();
        self.value.text.replace_range(range, insert);
        self.value.cursor = cursor;
        self.value.anchor = cursor;
        true
    }

    /// Returns whether text or selection changed. Rejected edits preserve all state.
    /// Read-only editors accept navigation and selection, including word selection,
    /// but reject edits and history traversal.
    pub fn apply(&mut self, command: EditCommand, max_chars: usize, read_only: bool) -> bool {
        use EditCommand::*;
        if !Self::valid_text(self.text()) {
            return false;
        }
        match command {
            Left { extend, word } => {
                let index = if !extend && !self.selection().is_empty() {
                    self.selection().start
                } else if word {
                    self.word_left(self.cursor())
                } else {
                    self.previous(self.cursor())
                };
                self.place(index, extend)
            }
            Right { extend, word } => {
                let index = if !extend && !self.selection().is_empty() {
                    self.selection().end
                } else if word {
                    self.word_right(self.cursor())
                } else {
                    self.next(self.cursor())
                };
                self.place(index, extend)
            }
            Home { extend } => self.place(0, extend),
            End { extend } => self.place(self.text().len(), extend),
            SelectAll => self.select_all(),
            Insert(ch) => {
                if read_only || !valid_character(ch) {
                    return false;
                }
                let mut encoded = [0; 4];
                self.replace(self.selection(), ch.encode_utf8(&mut encoded), max_chars)
            }
            Backspace { word } | Delete { word } => {
                if read_only {
                    return false;
                }
                let mut range = self.selection();
                if range.is_empty() {
                    if matches!(command, Backspace { .. }) {
                        range.start = if word {
                            self.word_left(self.cursor())
                        } else {
                            self.previous(self.cursor())
                        };
                    } else {
                        range.end = if word {
                            self.word_right(self.cursor())
                        } else {
                            self.next(self.cursor())
                        };
                    }
                }
                self.replace(range, "", max_chars)
            }
            Undo | Redo => {
                if read_only {
                    return false;
                }
                let (source, destination) = if command == Undo {
                    (&mut self.undo, &mut self.redo)
                } else {
                    (&mut self.redo, &mut self.undo)
                };
                let Some(previous) = source.last() else {
                    return false;
                };
                if previous.text.chars().count() > max_chars {
                    return false;
                }
                let previous = source.pop().unwrap();
                let current = core::mem::replace(&mut self.value, previous);
                Self::push(destination, current);
                true
            }
        }
    }
}
