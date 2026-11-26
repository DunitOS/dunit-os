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

    fn offset_at(&self, line: usize, col: usize) -> usize {
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
