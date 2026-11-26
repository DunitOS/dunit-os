//! Bounded UTF-8 document buffer with cursor/selection, multi-level undo/redo,
//! search/replace, line-column indexing, and shortcut help shared by TUI and GUI editors.
use alloc::{
    format,
    string::String,
    vec::Vec,
};
use core::ops::Range;

use crate::hash::{sha256, sha256_hex};

pub const MAX_DOC_BYTES: usize = 512 * 1024;
pub const MAX_FIND_HITS: usize = 4096;
const HISTORY_LIMIT: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorError {
    TooLarge,
    InvalidUtf8,
    InvalidText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPos {
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutHint {
    pub key: &'static str,
    pub label: &'static str,
}

pub const SHORTCUT_HINTS: &[ShortcutHint] = &[
    ShortcutHint { key: "^O / Ctrl+S", label: "Save" },
    ShortcutHint { key: "^X / Ctrl+Q", label: "Exit" },
    ShortcutHint { key: "^W / Ctrl+F", label: "Search" },
    ShortcutHint { key: "^R / Ctrl+H", label: "Replace" },
    ShortcutHint { key: "^Z", label: "Undo" },
    ShortcutHint { key: "^Y", label: "Redo" },
    ShortcutHint { key: "^K", label: "Cut Line" },
    ShortcutHint { key: "^U", label: "Paste" },
    ShortcutHint { key: "^_", label: "Goto Line" },
    ShortcutHint { key: "^G / F1", label: "Help" },
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentBuffer {
    path: Option<String>,
    text: String,
    cursor: usize,
    anchor: usize,
    preferred_col: Option<usize>,
    saved_hash: [u8; 32],
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    clipboard: String,
    clipboard_linewise: bool,
}

impl Default for DocumentBuffer {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_text(raw: &str) -> Result<String, EditorError> {
    if raw.len() > MAX_DOC_BYTES {
        return Err(EditorError::TooLarge);
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(ch),
            c if c.is_control() => return Err(EditorError::InvalidText),
            c => out.push(c),
        }
    }
    if out.len() > MAX_DOC_BYTES {
        return Err(EditorError::TooLarge);
    }
    Ok(out)
}

fn prev_char_boundary(s: &str, idx: usize) -> usize {
    let mut pos = idx.min(s.len());
    if pos == 0 {
        return 0;
    }
    pos -= 1;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn next_char_boundary(s: &str, idx: usize) -> usize {
    let mut pos = idx.min(s.len());
    if pos >= s.len() {
        return s.len();
    }
    pos += 1;
    while pos < s.len() && !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

impl DocumentBuffer {
    pub fn new() -> Self {
        let empty = String::new();
        let saved_hash = sha256(empty.as_bytes());
        Self {
            path: None,
            text: empty,
            cursor: 0,
            anchor: 0,
            preferred_col: None,
            saved_hash,
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: String::new(),
            clipboard_linewise: false,
        }
    }

    pub fn from_bytes(path: Option<String>, bytes: &[u8]) -> Result<Self, EditorError> {
        if bytes.len() > MAX_DOC_BYTES {
            return Err(EditorError::TooLarge);
        }
        let raw = core::str::from_utf8(bytes).map_err(|_| EditorError::InvalidUtf8)?;
        let text = normalize_text(raw)?;
        let saved_hash = sha256(text.as_bytes());
        Ok(Self {
            path,
            text,
            cursor: 0,
            anchor: 0,
            preferred_col: None,
            saved_hash,
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: String::new(),
            clipboard_linewise: false,
        })
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: impl Into<String>) {
        self.path = Some(path.into());
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn bytes_len(&self) -> usize {
        self.text.len()
    }

    pub fn is_dirty(&self) -> bool {
        sha256(self.text.as_bytes()) != self.saved_hash
    }

    pub fn mark_saved(&mut self) {
        self.saved_hash = sha256(self.text.as_bytes());
    }

    pub fn content_sha256_hex(&self) -> String {
        sha256_hex(self.text.as_bytes())
    }

    pub fn cursor_offset(&self) -> usize {
        self.cursor
    }

    pub fn anchor_offset(&self) -> usize {
        self.anchor
    }

    pub fn selection_range(&self) -> Range<usize> {
        if self.cursor <= self.anchor {
            self.cursor..self.anchor
        } else {
            self.anchor..self.cursor
        }
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection_range()]
    }

    pub fn clipboard(&self) -> &str {
        &self.clipboard
    }

    pub fn clipboard_is_linewise(&self) -> bool {
        self.clipboard_linewise
    }

    pub fn set_clipboard(&mut self, text: impl Into<String>, linewise: bool) {
        self.clipboard = text.into();
        self.clipboard_linewise = linewise;
    }

    pub fn selected_line_bounds(&self) -> (usize, usize) {
        let sel = self.selection_range();
        let a = self.pos_at(sel.start).line;
        let b = if sel.is_empty() {
            a
        } else {
            let end_pos = self.pos_at(prev_char_boundary(&self.text, sel.end));
            end_pos.line.max(a)
        };
        (a, b)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.text.bytes().filter(|&b| b == b'\n').count() + 1
    }

    pub fn line_slice(&self, line_idx: usize) -> Option<&str> {
        let (start, end) = self.line_byte_bounds(line_idx)?;
        Some(&self.text[start..end])
    }

    pub fn line_byte_bounds(&self, line_idx: usize) -> Option<(usize, usize)> {
        let mut cur_line = 0usize;
        let mut start = 0usize;
        for (idx, b) in self.text.bytes().enumerate() {
            if cur_line == line_idx && b == b'\n' {
                return Some((start, idx));
            }
            if b == b'\n' {
                cur_line += 1;
                start = idx + 1;
            }
        }
        if cur_line == line_idx {
            Some((start, self.text.len()))
        } else {
            None
        }
    }

    pub fn cursor_pos(&self) -> CursorPos {
        self.pos_at(self.cursor)
    }

    pub fn pos_at(&self, offset: usize) -> CursorPos {
        let clamped = offset.min(self.text.len());
        let mut line = 0usize;
        let mut line_start = 0usize;
        for (i, b) in self.text.bytes().take(clamped).enumerate() {
            if b == b'\n' {
                line += 1;
                line_start = i + 1;
            }
        }
        let col = self.text[line_start..clamped].chars().count();
        CursorPos { line, col }
    }

    pub fn offset_at(&self, line: usize, col: usize) -> usize {
        let target = line.min(self.line_count().saturating_sub(1));
        let (start, end) = self.line_byte_bounds(target).unwrap_or((0, self.text.len()));
        let slice = &self.text[start..end];
        let mut chars_seen = 0usize;
        for (byte_i, _) in slice.char_indices() {
            if chars_seen == col {
                return start + byte_i;
            }
            chars_seen += 1;
        }
        end
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn push_undo(&mut self) {
        if self.undo.len() >= HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(self.snapshot());
        self.redo.clear();
    }

    fn set_cursor(&mut self, pos: usize, extend: bool, reset_preferred_col: bool) {
        let clamped = pos.min(self.text.len());
        self.cursor = clamped;
        if !extend {
            self.anchor = clamped;
        }
        if reset_preferred_col {
            self.preferred_col = None;
        }
    }

    pub fn set_cursor_range(&mut self, cursor: usize, anchor: usize) {
        let mut c = cursor.min(self.text.len());
        let mut a = anchor.min(self.text.len());
        while c > 0 && !self.text.is_char_boundary(c) {
            c -= 1;
        }
        while a > 0 && !self.text.is_char_boundary(a) {
            a -= 1;
        }
        self.cursor = c;
        self.anchor = a;
        self.preferred_col = None;
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
        self.preferred_col = None;
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend && !self.selection_range().is_empty() {
            let start = self.selection_range().start;
            self.set_cursor(start, false, true);
            return;
        }
        let next = prev_char_boundary(&self.text, self.cursor);
        self.set_cursor(next, extend, true);
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend && !self.selection_range().is_empty() {
            let end = self.selection_range().end;
            self.set_cursor(end, false, true);
            return;
        }
        let next = next_char_boundary(&self.text, self.cursor);
        self.set_cursor(next, extend, true);
    }

    pub fn move_word_left(&mut self, extend: bool) {
        let bytes = self.text.as_bytes();
        let mut pos = self.cursor;
        while pos > 0 && !is_word_byte(bytes[pos - 1]) {
            pos = prev_char_boundary(&self.text, pos);
        }
        while pos > 0 && is_word_byte(bytes[pos - 1]) {
            pos = prev_char_boundary(&self.text, pos);
        }
        self.set_cursor(pos, extend, true);
    }

    pub fn move_word_right(&mut self, extend: bool) {
        let bytes = self.text.as_bytes();
        let mut pos = self.cursor;
        while pos < bytes.len() && is_word_byte(bytes[pos]) {
            pos = next_char_boundary(&self.text, pos);
        }
        while pos < bytes.len() && !is_word_byte(bytes[pos]) {
            pos = next_char_boundary(&self.text, pos);
        }
        self.set_cursor(pos, extend, true);
    }

    pub fn move_home(&mut self, extend: bool) {
        let pos = self.cursor_pos();
        let off = self.offset_at(pos.line, 0);
        self.set_cursor(off, extend, true);
    }

    pub fn move_end(&mut self, extend: bool) {
        let pos = self.cursor_pos();
        let end = self
            .line_byte_bounds(pos.line)
            .map_or(self.text.len(), |(_, e)| e);
        self.set_cursor(end, extend, true);
    }

    pub fn move_up(&mut self, lines: usize, extend: bool) {
        let pos = self.cursor_pos();
        let goal = *self.preferred_col.get_or_insert(pos.col);
        let next_line = pos.line.saturating_sub(lines);
        let off = self.offset_at(next_line, goal);
        self.set_cursor(off, extend, false);
    }

    pub fn move_down(&mut self, lines: usize, extend: bool) {
        let pos = self.cursor_pos();
        let goal = *self.preferred_col.get_or_insert(pos.col);
        let last = self.line_count().saturating_sub(1);
        let next_line = pos.line.saturating_add(lines).min(last);
        let off = self.offset_at(next_line, goal);
        self.set_cursor(off, extend, false);
    }

    pub fn move_doc_start(&mut self, extend: bool) {
        self.set_cursor(0, extend, true);
    }

    pub fn move_doc_end(&mut self, extend: bool) {
        self.set_cursor(self.text.len(), extend, true);
    }

    pub fn goto_line(&mut self, one_based_line: usize) -> CursorPos {
        let idx = one_based_line.saturating_sub(1).min(self.line_count().saturating_sub(1));
        let off = self.offset_at(idx, 0);
        self.set_cursor(off, false, true);
        self.cursor_pos()
    }

    pub fn insert_char(&mut self, ch: char) -> Result<(), EditorError> {
        if ch == '\n' || ch == '\t' || !ch.is_control() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            self.insert_str(s)
        } else {
            Err(EditorError::InvalidText)
        }
    }

    pub fn insert_str(&mut self, raw: &str) -> Result<(), EditorError> {
        let normalized = normalize_text(raw)?;
        let sel = self.selection_range();
        let new_len = self
            .text
            .len()
            .saturating_sub(sel.len())
            .checked_add(normalized.len())
            .ok_or(EditorError::TooLarge)?;
        if new_len > MAX_DOC_BYTES {
            return Err(EditorError::TooLarge);
        }
        self.push_undo();
        self.text.replace_range(sel.clone(), &normalized);
        let next_cursor = sel.start + normalized.len();
        self.cursor = next_cursor;
        self.anchor = next_cursor;
        self.preferred_col = None;
        Ok(())
    }

    /// Replace full document text (for synchronized GUI Textarea edits while keeping undo history).
    pub fn apply_full_text(&mut self, raw: &str) -> Result<bool, EditorError> {
        let normalized = normalize_text(raw)?;
        if normalized == self.text {
            return Ok(false);
        }
        self.push_undo();
        self.text = normalized;
        let clamped = self.cursor.min(self.text.len());
        self.set_cursor_range(clamped, clamped);
        Ok(true)
    }

    pub fn backspace(&mut self) -> bool {
        let sel = self.selection_range();
        if !sel.is_empty() {
            self.push_undo();
            self.text.replace_range(sel.clone(), "");
            self.set_cursor(sel.start, false, true);
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        let prev = prev_char_boundary(&self.text, self.cursor);
        self.push_undo();
        self.text.replace_range(prev..self.cursor, "");
        self.set_cursor(prev, false, true);
        true
    }

    pub fn delete_forward(&mut self) -> bool {
        let sel = self.selection_range();
        if !sel.is_empty() {
            self.push_undo();
            self.text.replace_range(sel.clone(), "");
            self.set_cursor(sel.start, false, true);
            return true;
        }
        if self.cursor >= self.text.len() {
            return false;
        }
        let next = next_char_boundary(&self.text, self.cursor);
        self.push_undo();
        self.text.replace_range(self.cursor..next, "");
        self.set_cursor(self.cursor, false, true);
        true
    }

    pub fn copy_selection(&mut self) -> bool {
        let sel = self.selection_range();
        if sel.is_empty() {
            return false;
        }
        self.clipboard = String::from(&self.text[sel]);
        true
    }

    pub fn cut_selection_or_line(&mut self) -> bool {
        let sel = self.selection_range();
        if !sel.is_empty() {
            self.clipboard = String::from(&self.text[sel.clone()]);
            self.clipboard_linewise = false;
            self.push_undo();
            self.text.replace_range(sel.clone(), "");
            self.set_cursor(sel.start, false, true);
            return true;
        }
        let pos = self.cursor_pos();
        let Some((start, end)) = self.line_byte_bounds(pos.line) else {
            return false;
        };
        let kill_end = if end < self.text.len() { end + 1 } else { end };
        if start == kill_end {
            return false;
        }
        self.clipboard = String::from(&self.text[start..kill_end]);
        self.clipboard_linewise = true;
        self.push_undo();
        self.text.replace_range(start..kill_end, "");
        self.set_cursor(start.min(self.text.len()), false, true);
        true
    }

    pub fn paste_clipboard(&mut self) -> Result<bool, EditorError> {
        if self.clipboard.is_empty() {
            return Ok(false);
        }
        let clip = self.clipboard.clone();
        self.insert_str(&clip)?;
        Ok(true)
    }

    pub fn move_first_non_blank(&mut self, extend: bool) {
        let pos = self.cursor_pos();
        let Some((start, end)) = self.line_byte_bounds(pos.line) else {
            return;
        };
        let mut off = start;
        for (rel, ch) in self.text[start..end].char_indices() {
            if !ch.is_whitespace() {
                off = start + rel;
                break;
            }
            off = start + rel + ch.len_utf8();
        }
        if off == end && end > start {
            off = prev_char_boundary(&self.text, end);
        }
        self.set_cursor(off, extend, true);
    }

    pub fn move_word_end(&mut self, extend: bool) {
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        if len == 0 || self.cursor >= len {
            return;
        }
        let mut pos = next_char_boundary(&self.text, self.cursor);
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos = next_char_boundary(&self.text, pos);
        }
        if pos >= len {
            self.set_cursor(prev_char_boundary(&self.text, len), extend, true);
            return;
        }
        let target_word = is_word_byte(bytes[pos]);
        while pos + 1 < len {
            let nxt = next_char_boundary(&self.text, pos);
            if nxt >= len || bytes[nxt].is_ascii_whitespace() || is_word_byte(bytes[nxt]) != target_word {
                break;
            }
            pos = nxt;
        }
        self.set_cursor(pos, extend, true);
    }

    pub fn move_paragraph_prev(&mut self, extend: bool) {
        let mut line = self.cursor_pos().line;
        while line > 0 && self.line_slice(line).is_some_and(|s| s.trim().is_empty()) {
            line -= 1;
        }
        while line > 0 && !self.line_slice(line - 1).is_some_and(|s| s.trim().is_empty()) {
            line -= 1;
        }
        let target = line.saturating_sub(usize::from(line > 0 && self.line_slice(line - 1).is_some_and(|s| s.trim().is_empty())));
        let off = self.offset_at(target, 0);
        self.set_cursor(off, extend, true);
    }

    pub fn move_paragraph_next(&mut self, extend: bool) {
        let total = self.line_count();
        let mut line = self.cursor_pos().line;
        while line + 1 < total && self.line_slice(line).is_some_and(|s| s.trim().is_empty()) {
            line += 1;
        }
        while line + 1 < total && !self.line_slice(line).is_some_and(|s| s.trim().is_empty()) {
            line += 1;
        }
        let off = self.offset_at(line, 0);
        self.set_cursor(off, extend, true);
    }

    pub fn word_bounds_at(&self, pos: usize) -> Option<Range<usize>> {
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        if len == 0 {
            return None;
        }
        let mut idx = pos.min(len);
        while idx > 0 && !self.text.is_char_boundary(idx) {
            idx -= 1;
        }
        if idx == len {
            if idx > 0 && is_word_byte(bytes[idx - 1]) {
                idx = prev_char_boundary(&self.text, idx);
            } else {
                return None;
            }
        } else if !is_word_byte(bytes[idx]) {
            return None;
        }
        let mut start = idx;
        while start > 0 && is_word_byte(bytes[start - 1]) {
            start = prev_char_boundary(&self.text, start);
        }
        let mut end = next_char_boundary(&self.text, idx);
        while end < len && is_word_byte(bytes[end]) {
            end = next_char_boundary(&self.text, end);
        }
        Some(start..end)
    }

    pub fn word_bounds_around(&self, pos: usize, around: bool) -> Option<Range<usize>> {
        let r = self.word_bounds_at(pos)?;
        if !around {
            return Some(r);
        }
        let bytes = self.text.as_bytes();
        let mut start = r.start;
        let mut end = r.end;
        while end < bytes.len() && matches!(bytes[end], b' ' | b'\t') {
            end += 1;
        }
        if end == r.end {
            while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
                start -= 1;
            }
        }
        Some(start..end)
    }

    pub fn word_under_cursor(&self) -> Option<String> {
        let r = self.word_bounds_at(self.cursor)?;
        Some(String::from(&self.text[r]))
    }

    pub fn find_char_in_line(&mut self, target: char, forward: bool, till: bool, extend: bool) -> bool {
        let pos = self.cursor_pos();
        let Some((start, end)) = self.line_byte_bounds(pos.line) else {
            return false;
        };
        if forward {
            let search_from = next_char_boundary(&self.text, self.cursor).min(end);
            for (rel, ch) in self.text[search_from..end].char_indices() {
                if ch == target {
                    let hit = search_from + rel;
                    let dest = if till {
                        prev_char_boundary(&self.text, hit).max(self.cursor)
                    } else {
                        hit
                    };
                    self.set_cursor(dest, extend, true);
                    return true;
                }
            }
        } else if self.cursor > start {
            for (rel, ch) in self.text[start..self.cursor].char_indices().rev() {
                if ch == target {
                    let hit = start + rel;
                    let dest = if till {
                        next_char_boundary(&self.text, hit).min(self.cursor)
                    } else {
                        hit
                    };
                    self.set_cursor(dest, extend, true);
                    return true;
                }
            }
        }
        false
    }

    pub fn matching_bracket_pair(&self) -> Option<(usize, usize)> {
        let bytes = self.text.as_bytes();
        if bytes.is_empty() {
            return None;
        }
        let mut probe = self.cursor.min(bytes.len());
        let is_bracket = |b: u8| matches!(b, b'(' | b')' | b'[' | b']' | b'{' | b'}');
        if probe >= bytes.len() || !is_bracket(bytes[probe]) {
            if probe > 0 && is_bracket(bytes[probe - 1]) {
                probe -= 1;
            } else {
                return None;
            }
        }
        let (open, close, forward) = match bytes[probe] {
            b'(' => (b'(', b')', true),
            b'[' => (b'[', b']', true),
            b'{' => (b'{', b'}', true),
            b')' => (b'(', b')', false),
            b']' => (b'[', b']', false),
            b'}' => (b'{', b'}', false),
            _ => return None,
        };
        let mut depth = 0usize;
        let max_scan = 32 * 1024;
        if forward {
            let limit = bytes.len().min(probe + max_scan);
            for idx in probe..limit {
                if bytes[idx] == open {
                    depth += 1;
                } else if bytes[idx] == close {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some((probe, idx));
                    }
                }
            }
        } else {
            let low = probe.saturating_sub(max_scan);
            for idx in (low..=probe).rev() {
                if bytes[idx] == close {
                    depth += 1;
                } else if bytes[idx] == open {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some((idx, probe));
                    }
                }
            }
        }
        None
    }

    pub fn move_matching_bracket(&mut self, extend: bool) -> bool {
        let Some((a, b)) = self.matching_bracket_pair() else {
            return false;
        };
        let dest = if self.cursor == a || self.cursor.saturating_sub(1) == a {
            b
        } else {
            a
        };
        self.set_cursor(dest, extend, true);
        true
    }

    pub fn delete_range(&mut self, range: Range<usize>, yank: bool, linewise: bool) -> bool {
        let start = range.start.min(self.text.len());
        let end = range.end.min(self.text.len());
        if start >= end {
            return false;
        }
        if yank {
            self.clipboard = String::from(&self.text[start..end]);
            self.clipboard_linewise = linewise;
        }
        self.push_undo();
        self.text.replace_range(start..end, "");
        self.set_cursor(start.min(self.text.len()), false, true);
        true
    }

    pub fn yank_range(&mut self, range: Range<usize>, linewise: bool) -> bool {
        let start = range.start.min(self.text.len());
        let end = range.end.min(self.text.len());
        if start >= end {
            return false;
        }
        self.clipboard = String::from(&self.text[start..end]);
        self.clipboard_linewise = linewise;
        true
    }

    fn line_span_range(&self, start_line: usize, count: usize) -> Option<Range<usize>> {
        let last = self.line_count().saturating_sub(1);
        if start_line > last || count == 0 {
            return None;
        }
        let end_line = (start_line + count - 1).min(last);
        let (s, _) = self.line_byte_bounds(start_line)?;
        let (_, e) = self.line_byte_bounds(end_line)?;
        let full_end = if e < self.text.len() { e + 1 } else { e };
        Some(s..full_end)
    }

    pub fn delete_lines(&mut self, count: usize) -> bool {
        let cur = self.cursor_pos().line;
        let Some(r) = self.line_span_range(cur, count) else {
            return false;
        };
        let mut clip = String::from(&self.text[r.clone()]);
        if !clip.ends_with('\n') {
            clip.push('\n');
        }
        self.clipboard = clip;
        self.clipboard_linewise = true;
        self.push_undo();
        self.text.replace_range(r.clone(), "");
        let target_line = cur.min(self.line_count().saturating_sub(1));
        let next_off = self.offset_at(target_line, 0);
        self.set_cursor(next_off, false, true);
        true
    }

    pub fn yank_lines(&mut self, count: usize) -> bool {
        let cur = self.cursor_pos().line;
        let Some(r) = self.line_span_range(cur, count) else {
            return false;
        };
        let mut clip = String::from(&self.text[r]);
        if !clip.ends_with('\n') {
            clip.push('\n');
        }
        self.clipboard = clip;
        self.clipboard_linewise = true;
        true
    }

    pub fn paste_vim(&mut self, before: bool) -> Result<bool, EditorError> {
        if self.clipboard.is_empty() {
            return Ok(false);
        }
        if self.clipboard_linewise {
            let pos = self.cursor_pos();
            let Some((start, end)) = self.line_byte_bounds(pos.line) else {
                return Ok(false);
            };
            let mut body = self.clipboard.clone();
            let insert_at = if before {
                if !body.ends_with('\n') {
                    body.push('\n');
                }
                start
            } else {
                if !body.ends_with('\n') {
                    body.push('\n');
                }
                if end < self.text.len() {
                    end + 1
                } else {
                    if !self.text.ends_with('\n') {
                        body.insert(0, '\n');
                        body.pop();
                    }
                    end
                }
            };
            if self.text.len().saturating_add(body.len()) > MAX_DOC_BYTES {
                return Err(EditorError::TooLarge);
            }
            self.push_undo();
            self.text.insert_str(insert_at, &body);
            let placed_line = if before { pos.line } else { pos.line + 1 };
            let off = self.offset_at(placed_line, 0);
            self.set_cursor(off, false, true);
            Ok(true)
        } else {
            let at = if before || self.cursor >= self.text.len() {
                self.cursor
            } else {
                let (_, line_end) = self.line_byte_bounds(self.cursor_pos().line).unwrap_or((0, self.text.len()));
                if self.cursor < line_end {
                    next_char_boundary(&self.text, self.cursor)
                } else {
                    self.cursor
                }
            };
            self.set_cursor(at, false, true);
            let clip = self.clipboard.clone();
            self.insert_str(&clip)?;
            Ok(true)
        }
    }

    pub fn auto_indent_for_line(&self, line_idx: usize) -> String {
        let Some(line) = self.line_slice(line_idx) else {
            return String::new();
        };
        let mut indent = String::new();
        for ch in line.chars() {
            if ch == ' ' || ch == '\t' {
                indent.push(ch);
            } else {
                break;
            }
        }
        let trimmed = line.trim_end();
        if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
            indent.push_str("    ");
        }
        indent
    }

    pub fn insert_newline_indented(&mut self, auto_indent: bool) -> Result<(), EditorError> {
        if !auto_indent {
            return self.insert_char('\n');
        }
        let line_idx = self.cursor_pos().line;
        let (start, _) = self.line_byte_bounds(line_idx).unwrap_or((0, 0));
        let prefix = &self.text[start..self.cursor.max(start).min(self.text.len())];
        let mut indent = String::new();
        for ch in prefix.chars() {
            if ch == ' ' || ch == '\t' {
                indent.push(ch);
            } else {
                break;
            }
        }
        if prefix.trim_end().ends_with('{') {
            indent.push_str("    ");
        }
        let mut ins = String::from("\n");
        ins.push_str(&indent);
        self.insert_str(&ins)
    }

    pub fn open_line(&mut self, above: bool, auto_indent: bool) -> Result<(), EditorError> {
        let pos = self.cursor_pos();
        let indent = if auto_indent {
            self.auto_indent_for_line(pos.line)
        } else {
            String::new()
        };
        let Some((start, end)) = self.line_byte_bounds(pos.line) else {
            return Ok(());
        };
        if above {
            self.set_cursor(start, false, true);
            let mut s = indent.clone();
            s.push('\n');
            self.insert_str(&s)?;
            self.set_cursor(start + indent.len(), false, true);
        } else {
            self.set_cursor(end, false, true);
            let mut s = String::from("\n");
            s.push_str(&indent);
            self.insert_str(&s)?;
        }
        Ok(())
    }

    pub fn join_next_line(&mut self) -> bool {
        let line = self.cursor_pos().line;
        let Some((_, end)) = self.line_byte_bounds(line) else {
            return false;
        };
        let Some((next_start, next_end)) = self.line_byte_bounds(line + 1) else {
            return false;
        };
        let trimmed_next = self.text[next_start..next_end].trim_start();
        let insert_space = !trimmed_next.is_empty() && !self.text[..end].ends_with(' ');
        let rep = if insert_space { " " } else { "" };
        let lead_ws_len = (next_end - next_start) - trimmed_next.len();
        self.push_undo();
        self.text.replace_range(end..(next_start + lead_ws_len), rep);
        self.set_cursor(end, false, true);
        true
    }

    pub fn replace_char_at_cursor(&mut self, ch: char) -> bool {
        if self.cursor >= self.text.len() || ch.is_control() {
            return false;
        }
        let next = next_char_boundary(&self.text, self.cursor);
        if &self.text[self.cursor..next] == "\n" {
            return false;
        }
        let mut buf = [0u8; 4];
        let rep = ch.encode_utf8(&mut buf);
        self.push_undo();
        self.text.replace_range(self.cursor..next, rep);
        self.set_cursor(self.cursor, false, true);
        true
    }

    pub fn indent_lines(&mut self, start_line: usize, end_line: usize, shift_right: bool) -> usize {
        let last = self.line_count().saturating_sub(1);
        let a = start_line.min(last);
        let b = end_line.min(last).max(a);
        let mut changed = 0usize;
        self.push_undo();
        for ln in (a..=b).rev() {
            let Some((s, e)) = self.line_byte_bounds(ln) else {
                continue;
            };
            if shift_right {
                if s < e && self.text.len() + 4 <= MAX_DOC_BYTES {
                    self.text.insert_str(s, "    ");
                    changed += 1;
                }
            } else {
                let slice = &self.text[s..e];
                let mut remove_bytes = 0usize;
                if slice.starts_with('\t') {
                    remove_bytes = 1;
                } else {
                    for b_ch in slice.bytes().take(4) {
                        if b_ch == b' ' {
                            remove_bytes += 1;
                        } else {
                            break;
                        }
                    }
                }
                if remove_bytes > 0 {
                    self.text.replace_range(s..s + remove_bytes, "");
                    changed += 1;
                }
            }
        }
        if changed == 0 {
            let _ = self.undo.pop();
        } else {
            let off = self.offset_at(a, 0);
            self.set_cursor(off, false, true);
        }
        changed
    }

    pub fn toggle_comment_lines(&mut self, start_line: usize, end_line: usize, prefix: &str) -> usize {
        if prefix.is_empty() {
            return 0;
        }
        let last = self.line_count().saturating_sub(1);
        let a = start_line.min(last);
        let b = end_line.min(last).max(a);
        let mut all_commented = true;
        let mut any_nonempty = false;
        for ln in a..=b {
            if let Some(line) = self.line_slice(ln) {
                let trimmed = line.trim_start();
                if !trimmed.is_empty() {
                    any_nonempty = true;
                    if !trimmed.starts_with(prefix) {
                        all_commented = false;
                        break;
                    }
                }
            }
        }
        if !any_nonempty {
            return 0;
        }
        let mut changed = 0usize;
        self.push_undo();
        for ln in (a..=b).rev() {
            let Some((s, e)) = self.line_byte_bounds(ln) else {
                continue;
            };
            let slice = &self.text[s..e];
            let lead = slice.len() - slice.trim_start().len();
            if slice[lead..].is_empty() {
                continue;
            }
            if all_commented {
                let after = s + lead + prefix.len();
                let drop_space = usize::from(self.text[after..].starts_with(' '));
                self.text.replace_range(s + lead..after + drop_space, "");
                changed += 1;
            } else {
                let ins = format!("{} ", prefix);
                if self.text.len() + ins.len() <= MAX_DOC_BYTES {
                    self.text.insert_str(s + lead, &ins);
                    changed += 1;
                }
            }
        }
        if changed == 0 {
            let _ = self.undo.pop();
        } else {
            let off = self.offset_at(a, 0);
            self.set_cursor(off, false, true);
        }
        changed
    }

    pub fn find_all_line_matches(line: &str, query: &str, case_sensitive: bool) -> Vec<Range<usize>> {
        let mut hits = Vec::new();
        if query.is_empty() {
            return hits;
        }
        let mut from = 0usize;
        while hits.len() < 128 {
            let Some(m) = find_match(line, query, from, case_sensitive) else {
                break;
            };
            from = m.end.max(from + 1);
            hits.push(m);
        }
        hits
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        if self.redo.len() >= HISTORY_LIMIT {
            self.redo.remove(0);
        }
        self.redo.push(self.snapshot());
        self.text = prev.text;
        self.cursor = prev.cursor.min(self.text.len());
        self.anchor = prev.anchor.min(self.text.len());
        self.preferred_col = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        if self.undo.len() >= HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(self.snapshot());
        self.text = next.text;
        self.cursor = next.cursor.min(self.text.len());
        self.anchor = next.anchor.min(self.text.len());
        self.preferred_col = None;
        true
    }

    pub fn find_next(&mut self, query: &str, case_sensitive: bool) -> Option<Range<usize>> {
        if query.is_empty() {
            return None;
        }
        let start = self.selection_range().end.min(self.text.len());
        let found = find_match(&self.text, query, start, case_sensitive)
            .or_else(|| find_match(&self.text, query, 0, case_sensitive))?;
        self.anchor = found.start;
        self.cursor = found.end;
        self.preferred_col = None;
        Some(found)
    }

    pub fn find_prev(&mut self, query: &str, case_sensitive: bool) -> Option<Range<usize>> {
        if query.is_empty() {
            return None;
        }
        let end = self.selection_range().start;
        let found = find_match_before(&self.text, query, end, case_sensitive)
            .or_else(|| find_match_before(&self.text, query, self.text.len(), case_sensitive))?;
        self.anchor = found.start;
        self.cursor = found.end;
        self.preferred_col = None;
        Some(found)
    }

    pub fn replace_next(
        &mut self,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
    ) -> Result<bool, EditorError> {
        if query.is_empty() {
            return Ok(false);
        }
        let norm_rep = normalize_text(replacement)?;
        let selected = self.selected_text();
        let matched = if case_sensitive {
            selected == query
        } else {
            eq_ignore_ascii_case(selected, query)
        };
        if !matched && self.find_next(query, case_sensitive).is_none() {
            return Ok(false);
        }
        self.insert_str(&norm_rep)?;
        let _ = self.find_next(query, case_sensitive);
        Ok(true)
    }

    pub fn replace_all(
        &mut self,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
    ) -> Result<usize, EditorError> {
        if query.is_empty() {
            return Ok(0);
        }
        let norm_rep = normalize_text(replacement)?;
        let mut hits = Vec::new();
        let mut pos = 0usize;
        while hits.len() < MAX_FIND_HITS {
            let Some(m) = find_match(&self.text, query, pos, case_sensitive) else {
                break;
            };
            pos = m.end.max(pos + 1);
            hits.push(m);
        }
        if hits.is_empty() {
            return Ok(0);
        }
        let delta = hits.len() * norm_rep.len();
        let removed: usize = hits.iter().map(|r| r.len()).sum();
        if self.text.len() - removed + delta > MAX_DOC_BYTES {
            return Err(EditorError::TooLarge);
        }
        self.push_undo();
        let mut rebuilt = String::with_capacity(self.text.len() - removed + delta);
        let mut last = 0usize;
        for m in &hits {
            rebuilt.push_str(&self.text[last..m.start]);
            rebuilt.push_str(&norm_rep);
            last = m.end;
        }
        rebuilt.push_str(&self.text[last..]);
        self.text = rebuilt;
        self.set_cursor(0, false, true);
        Ok(hits.len())
    }

    pub fn status_line(&self) -> String {
        let pos = self.cursor_pos();
        let name = self.path.as_deref().unwrap_or("[untitled]");
        let mark = if self.is_dirty() { " [Modified]" } else { "" };
        format!(
            "{}{} | Ln {}, Col {} | {} lines | {} B",
            name,
            mark,
            pos.line + 1,
            pos.col + 1,
            self.line_count(),
            self.text.len()
        )
    }
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn find_match(haystack: &str, needle: &str, from: usize, case_sensitive: bool) -> Option<Range<usize>> {
    if needle.is_empty() || from > haystack.len() {
        return None;
    }
    if case_sensitive {
        let rel = haystack[from..].find(needle)?;
        let start = from + rel;
        return Some(start..start + needle.len());
    }
    let nb = needle.as_bytes();
    let hb = haystack.as_bytes();
    let mut i = from;
    while i + nb.len() <= hb.len() {
        if haystack.is_char_boundary(i)
            && haystack.is_char_boundary(i + nb.len())
            && hb[i..i + nb.len()].eq_ignore_ascii_case(nb)
        {
            return Some(i..i + nb.len());
        }
        i += 1;
    }
    None
}

fn find_match_before(
    haystack: &str,
    needle: &str,
    before: usize,
    case_sensitive: bool,
) -> Option<Range<usize>> {
    let limit = before.min(haystack.len());
    if needle.is_empty() || limit < needle.len() {
        return None;
    }
    if case_sensitive {
        let start = haystack[..limit].rfind(needle)?;
        return Some(start..start + needle.len());
    }
    let nb = needle.as_bytes();
    let hb = haystack.as_bytes();
    let mut i = limit - nb.len();
    loop {
        if haystack.is_char_boundary(i)
            && haystack.is_char_boundary(i + nb.len())
            && hb[i..i + nb.len()].eq_ignore_ascii_case(nb)
        {
            return Some(i..i + nb.len());
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}
