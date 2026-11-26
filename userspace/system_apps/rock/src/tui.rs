//! Pure TUI state machine, ANSI key parser, and renderer for `rock` so both
//! the native `/app/rock` binary and host unit tests exercise the exact same code.
use alloc::{
    format,
    string::String,
};
use dunit_editor::{DocumentBuffer, SHORTCUT_HINTS};
use dunit_shell::Utf8Decoder;

const DEFAULT_ROWS: usize = 24;
const DEFAULT_COLS: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptMode {
    None,
    SavePath(String),
    ConfirmExit,
    Search(String),
    ReplaceQuery(String),
    ReplaceWith { query: String, with: String },
    ConfirmReplace { query: String, with: String },
    GotoLine(String),
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EscState {
    Normal,
    Esc,
    Csi,
    Ss3,
}

pub struct RockTui {
    pub doc: DocumentBuffer,
    pub rows: usize,
    pub cols: usize,
    pub top_line: usize,
    pub left_col: usize,
    pub mark_mode: bool,
    pub prompt: PromptMode,
    pub status_msg: String,
    pub last_query: String,
    pub last_saved_sha256: Option<String>,
    pub quit: bool,
    esc: EscState,
    csi_buf: String,
    utf8: Utf8Decoder,
}

impl RockTui {
    pub fn new(doc: DocumentBuffer, rows: usize, cols: usize) -> Self {
        let path_note = doc
            .path()
            .map(|p| format!("Opened {}", p))
            .unwrap_or_else(|| String::from("New buffer"));
        Self {
            doc,
            rows: rows.max(8),
            cols: cols.max(32),
            top_line: 0,
            left_col: 0,
            mark_mode: false,
            prompt: PromptMode::None,
            status_msg: path_note,
            last_query: String::new(),
            last_saved_sha256: None,
            quit: false,
            esc: EscState::Normal,
            csi_buf: String::new(),
            utf8: Utf8Decoder::default(),
        }
    }

    pub fn viewport_lines(&self) -> usize {
        self.rows.saturating_sub(4).max(1)
    }

    pub fn sync_viewport(&mut self) {
        let pos = self.doc.cursor_pos();
        let vh = self.viewport_lines();
        if pos.line < self.top_line {
            self.top_line = pos.line;
        } else if pos.line >= self.top_line + vh {
            self.top_line = pos.line + 1 - vh;
        }
        let gutter = 6usize;
        let vw = self.cols.saturating_sub(gutter + 1).max(8);
        if pos.col < self.left_col {
            self.left_col = pos.col;
        } else if pos.col >= self.left_col + vw {
            self.left_col = pos.col + 1 - vw;
        }
    }

    pub fn feed_bytes<F>(&mut self, bytes: &[u8], mut save_fn: F) -> bool
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        let mut dirty_screen = false;
        for &b in bytes {
            if self.feed_byte(b, &mut save_fn) {
                dirty_screen = true;
            }
            if self.quit {
                break;
            }
        }
        dirty_screen
    }

    pub fn feed_byte<F>(&mut self, b: u8, save_fn: &mut F) -> bool
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        match self.esc {
            EscState::Normal => {
                if b == 0x1B {
                    self.esc = EscState::Esc;
                    return false;
                }
                let Some(ch) = self.utf8.push(b) else {
                    return false;
                };
                self.handle_char(ch, save_fn);
                true
            }
            EscState::Esc => {
                if b == b'[' {
                    self.esc = EscState::Csi;
                    self.csi_buf.clear();
                    false
                } else if b == b'O' {
                    self.esc = EscState::Ss3;
                    false
                } else {
                    self.esc = EscState::Normal;
                    if b == 0x1B {
                        self.prompt = PromptMode::None;
                        self.status_msg = String::from("Cancelled");
                        true
                    } else {
                        false
                    }
                }
            }
            EscState::Ss3 => {
                self.esc = EscState::Normal;
                match b {
                    b'P' => {
                        self.toggle_help();
                        true
                    }
                    b'H' => {
                        self.doc.move_home(self.mark_mode);
                        self.sync_viewport();
                        true
                    }
                    b'F' => {
                        self.doc.move_end(self.mark_mode);
                        self.sync_viewport();
                        true
                    }
                    _ => false,
                }
            }
            EscState::Csi => {
                if (b.is_ascii_digit() || b == b';') && self.csi_buf.len() < 16 {
                    self.csi_buf.push(b as char);
                    return false;
                }
                self.esc = EscState::Normal;
                let seq = self.csi_buf.clone();
                self.csi_buf.clear();
                self.handle_csi(&seq, b);
                self.sync_viewport();
                true
            }
        }
    }

    fn handle_csi(&mut self, params: &str, final_byte: u8) {
        if !matches!(self.prompt, PromptMode::None) {
            return;
        }
        let mut parts = params.split(';');
        let p1 = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
        let mods = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
        let shift = mods == 2 || mods == 6 || self.mark_mode;
        let ctrl = mods == 5 || mods == 6;

        match final_byte {
            b'A' => self.doc.move_up(1, shift),
            b'B' => self.doc.move_down(1, shift),
            b'C' => {
                if ctrl {
                    self.doc.move_word_right(shift);
                } else {
                    self.doc.move_right(shift);
                }
            }
            b'D' => {
                if ctrl {
                    self.doc.move_word_left(shift);
                } else {
                    self.doc.move_left(shift);
                }
            }
            b'H' => self.doc.move_home(shift),
            b'F' => self.doc.move_end(shift),
            b'~' => match p1 {
                1 | 7 => self.doc.move_home(shift),
                4 | 8 => self.doc.move_end(shift),
                3 => {
                    self.doc.delete_forward();
                }
                5 => self.doc.move_up(self.viewport_lines(), shift),
                6 => self.doc.move_down(self.viewport_lines(), shift),
                11 => self.toggle_help(),
                _ => {}
            },
            _ => {}
        }
    }

    fn toggle_help(&mut self) {
        self.prompt = if matches!(self.prompt, PromptMode::Help) {
            PromptMode::None
        } else {
            PromptMode::Help
        };
    }

    fn save_to_path<F>(&mut self, path: &str, save_fn: &mut F)
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        if path.trim().is_empty() {
            self.status_msg = String::from("Save aborted: empty filename");
            return;
        }
        match save_fn(path, self.doc.text().as_bytes()) {
            Ok(()) => {
                self.doc.set_path(String::from(path));
                self.doc.mark_saved();
                let hash = self.doc.content_sha256_hex();
                self.status_msg = format!("Wrote {} bytes to {} (sha256:{})", self.doc.bytes_len(), path, &hash[..12]);
                self.last_saved_sha256 = Some(hash);
            }
            Err(err) => {
                self.status_msg = format!("Save failed: {}", err);
            }
        }
    }

    fn handle_char<F>(&mut self, ch: char, save_fn: &mut F)
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        if !matches!(self.prompt, PromptMode::None) {
            self.handle_prompt_char(ch, save_fn);
            self.sync_viewport();
            return;
        }
        let b = if ch.is_ascii() { ch as u8 } else { 0 };
        match b {
            0x07 => self.toggle_help(), // ^G Help
            0x0F => {
                // ^O Write Out / Save
                let initial = String::from(self.doc.path().unwrap_or(""));
                self.prompt = PromptMode::SavePath(initial);
            }
            0x18 => {
                // ^X Exit
                if self.doc.is_dirty() {
                    self.prompt = PromptMode::ConfirmExit;
                } else {
                    self.quit = true;
                }
            }
            0x17 => {
                // ^W Search
                self.prompt = PromptMode::Search(self.last_query.clone());
            }
            0x12 => {
                // ^R Search & Replace
                self.prompt = PromptMode::ReplaceQuery(self.last_query.clone());
            }
            0x1A => {
                // ^Z Undo
                if self.doc.undo() {
                    self.status_msg = String::from("Undo");
                } else {
                    self.status_msg = String::from("Nothing to undo");
                }
            }
            0x19 => {
                // ^Y Redo
                if self.doc.redo() {
                    self.status_msg = String::from("Redo");
                } else {
                    self.status_msg = String::from("Nothing to redo");
                }
            }
            0x0B => {
                // ^K Cut line or selection
                if self.doc.cut_selection_or_line() {
                    self.mark_mode = false;
                    self.status_msg = String::from("Cut to buffer");
                }
            }
            0x15 => {
                // ^U Uncut / Paste
                let _ = self.doc.paste_clipboard();
                self.mark_mode = false;
            }
            0x1E => {
                // ^^ Mark set/unset
                self.mark_mode = !self.mark_mode;
                self.status_msg = if self.mark_mode {
                    String::from("Mark Set (selection active)")
                } else {
                    String::from("Mark Unset")
                };
            }
            0x01 => self.doc.move_home(self.mark_mode), // ^A
            0x05 => self.doc.move_end(self.mark_mode),  // ^E
            0x06 => {
                // ^F Find next saved query
                if !self.last_query.is_empty() {
                    let q = self.last_query.clone();
                    if self.doc.find_next(&q, false).is_some() {
                        self.status_msg = format!("Found '{}'", q);
                    } else {
                        self.status_msg = format!("'{}' not found", q);
                    }
                }
            }
            0x1F | 0x0C => {
                // ^_ Goto line
                self.prompt = PromptMode::GotoLine(String::new());
            }
            b'\r' | b'\n' => {
                let _ = self.doc.insert_char('\n');
                self.mark_mode = false;
            }
            b'\t' => {
                let _ = self.doc.insert_str("    ");
                self.mark_mode = false;
            }
            0x08 | 0x7F => {
                self.doc.backspace();
                self.mark_mode = false;
            }
            _ if !ch.is_control() => {
                let _ = self.doc.insert_char(ch);
                self.mark_mode = false;
            }
            _ => {}
        }
        self.sync_viewport();
    }

    fn handle_prompt_char<F>(&mut self, ch: char, save_fn: &mut F)
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        if ch == '\x03' {
            self.prompt = PromptMode::None;
            self.status_msg = String::from("Cancelled");
            return;
        }
        match core::mem::replace(&mut self.prompt, PromptMode::None) {
            PromptMode::None => {}
            PromptMode::Help => {
                self.prompt = PromptMode::None;
            }
            PromptMode::ConfirmExit => match ch.to_ascii_lowercase() {
                'y' => {
                    if let Some(p) = self.doc.path().map(String::from) {
                        self.save_to_path(&p, save_fn);
                        if !self.doc.is_dirty() {
                            self.quit = true;
                        }
                    } else {
                        self.prompt = PromptMode::SavePath(String::new());
                    }
                }
                'n' => {
                    self.quit = true;
                }
                'c' => {
                    self.status_msg = String::from("Exit cancelled");
                }
                _ => {
                    self.prompt = PromptMode::ConfirmExit;
                }
            },
            PromptMode::SavePath(mut buf) => match ch {
                '\r' | '\n' => self.save_to_path(&buf, save_fn),
                '\x08' | '\x7f' => {
                    buf.pop();
                    self.prompt = PromptMode::SavePath(buf);
                }
                c if !c.is_control() => {
                    buf.push(c);
                    self.prompt = PromptMode::SavePath(buf);
                }
                _ => self.prompt = PromptMode::SavePath(buf),
            },
            PromptMode::Search(mut q) => match ch {
                '\r' | '\n' => {
                    self.last_query = q.clone();
                    if self.doc.find_next(&q, false).is_some() {
                        self.status_msg = format!("Match: {}", q);
                    } else {
                        self.status_msg = format!("Not found: {}", q);
                    }
                }
                '\x08' | '\x7f' => {
                    q.pop();
                    self.prompt = PromptMode::Search(q);
                }
                c if !c.is_control() => {
                    q.push(c);
                    self.prompt = PromptMode::Search(q);
                }
                _ => self.prompt = PromptMode::Search(q),
            },
            PromptMode::ReplaceQuery(mut q) => match ch {
                '\r' | '\n' if !q.is_empty() => {
                    self.last_query = q.clone();
                    self.prompt = PromptMode::ReplaceWith {
                        query: q,
                        with: String::new(),
                    };
                }
                '\r' | '\n' => self.status_msg = String::from("Empty search query"),
                '\x08' | '\x7f' => {
                    q.pop();
                    self.prompt = PromptMode::ReplaceQuery(q);
                }
                c if !c.is_control() => {
                    q.push(c);
                    self.prompt = PromptMode::ReplaceQuery(q);
                }
                _ => self.prompt = PromptMode::ReplaceQuery(q),
            },
            PromptMode::ReplaceWith { query, mut with } => match ch {
                '\r' | '\n' => {
                    if self.doc.find_next(&query, false).is_some() {
                        self.prompt = PromptMode::ConfirmReplace { query, with };
                    } else {
                        self.status_msg = format!("Not found: {}", query);
                    }
                }
                '\x08' | '\x7f' => {
                    with.pop();
                    self.prompt = PromptMode::ReplaceWith { query, with };
                }
                c if !c.is_control() => {
                    with.push(c);
                    self.prompt = PromptMode::ReplaceWith { query, with };
                }
                _ => self.prompt = PromptMode::ReplaceWith { query, with },
            },
            PromptMode::ConfirmReplace { query, with } => match ch.to_ascii_lowercase() {
                'y' => {
                    let _ = self.doc.replace_next(&query, &with, false);
                    if self.doc.find_next(&query, false).is_some() {
                        self.prompt = PromptMode::ConfirmReplace { query, with };
                    } else {
                        self.status_msg = String::from("Replaced occurrence");
                    }
                }
                'a' => {
                    let count = self.doc.replace_all(&query, &with, false).unwrap_or(0);
                    self.status_msg = format!("Replaced {} occurrence(s)", count);
                }
                'n' => {
                    if self.doc.find_next(&query, false).is_some() {
                        self.prompt = PromptMode::ConfirmReplace { query, with };
                    } else {
                        self.status_msg = String::from("No further matches");
                    }
                }
                'c' => {
                    self.status_msg = String::from("Replace cancelled");
                }
                _ => {
                    self.prompt = PromptMode::ConfirmReplace { query, with };
                }
            },
            PromptMode::GotoLine(mut digits) => match ch {
                '\r' | '\n' => {
                    if let Ok(n) = digits.trim().parse::<usize>() {
                        let p = self.doc.goto_line(n.max(1));
                        self.status_msg = format!("Line {}", p.line + 1);
                    }
                }
                '\x08' | '\x7f' => {
                    digits.pop();
                    self.prompt = PromptMode::GotoLine(digits);
                }
                c if c.is_ascii_digit() => {
                    digits.push(c);
                    self.prompt = PromptMode::GotoLine(digits);
                }
                _ => self.prompt = PromptMode::GotoLine(digits),
            },
        }
    }

    pub fn render_ansi(&self) -> String {
        let mut out = String::with_capacity(self.rows * self.cols);
        out.push_str("\x1b[?25l\x1b[H");

        // Header bar
        let title = format!(" Dunit rock 1.0   {}", self.doc.status_line());
        out.push_str("\x1b[7m");
        push_padded(&mut out, &title, self.cols);
        out.push_str("\x1b[27m\r\n");

        let vh = self.viewport_lines();
        let sel = self.doc.selection_range();
        let gutter = 6usize;
        let text_cols = self.cols.saturating_sub(gutter).max(8);

        if matches!(self.prompt, PromptMode::Help) {
            for r in 0..vh {
                if r == 0 {
                    out.push_str("  Rock Editor Shortcuts (press any key to return)\x1b[K\r\n");
                } else if let Some(hint) = SHORTCUT_HINTS.get(r - 1) {
                    out.push_str(&format!("  {:14} {}\x1b[K\r\n", hint.key, hint.label));
                } else {
                    out.push_str("\x1b[K\r\n");
                }
            }
        } else {
            for r in 0..vh {
                let line_idx = self.top_line + r;
                if let Some((start, end)) = self.doc.line_byte_bounds(line_idx) {
                    out.push_str(&format!("{:4}| ", line_idx + 1));
                    let slice = &self.doc.text()[start..end];
                    let mut col = 0usize;
                    let mut printed = 0usize;
                    let mut rev = false;
                    for (rel, ch) in slice.char_indices() {
                        if col >= self.left_col && printed < text_cols {
                            let abs = start + rel;
                            let in_sel = sel.start <= abs && abs < sel.end;
                            if in_sel != rev {
                                out.push_str(if in_sel { "\x1b[7m" } else { "\x1b[27m" });
                                rev = in_sel;
                            }
                            out.push(ch);
                            printed += 1;
                        }
                        col += 1;
                    }
                    if rev {
                        out.push_str("\x1b[27m");
                    }
                    out.push_str("\x1b[K\r\n");
                } else {
                    out.push_str("    ~\x1b[K\r\n");
                }
            }
        }

        // Status / prompt row
        let prompt_line = match &self.prompt {
            PromptMode::None => format!("[ {} ]", self.status_msg),
            PromptMode::SavePath(p) => format!("File Name to Write: {}", p),
            PromptMode::ConfirmExit => String::from("Save modified buffer? (Y Yes / N No / C Cancel)"),
            PromptMode::Search(q) => format!("Search: {}", q),
            PromptMode::ReplaceQuery(q) => format!("Search (to replace): {}", q),
            PromptMode::ReplaceWith { query, with } => format!("Replace '{}' with: {}", query, with),
            PromptMode::ConfirmReplace { query, with } => {
                format!("Replace '{}' -> '{}'? [Y/N/A(ll)/C(ancel)]", query, with)
            }
            PromptMode::GotoLine(n) => format!("Enter line number: {}", n),
            PromptMode::Help => String::from("Press any key to exit help"),
        };
        out.push_str("\x1b[7m ");
        push_padded(&mut out, &prompt_line, self.cols.saturating_sub(1));
        out.push_str("\x1b[27m\r\n");

        // 2 shortcut rows
        let row1 = "^G Help   ^O Save    ^X Exit   ^W Search   ^R Replace";
        let row2 = "^Z Undo   ^Y Redo    ^K Cut    ^U Paste    ^^ Mark    ^_ Line\x1b[K";
        out.push_str(row1);
        out.push_str("\x1b[K\r\n");
        out.push_str(row2);

        // Place terminal cursor
        let pos = self.doc.cursor_pos();
        let cur_row = 2 + pos.line.saturating_sub(self.top_line);
        let cur_col = 1 + gutter + pos.col.saturating_sub(self.left_col);
        out.push_str(&format!("\x1b[{};{}H\x1b[?25h", cur_row, cur_col));
        out
    }
}

fn push_padded(dst: &mut String, s: &str, width: usize) {
    let mut n = 0usize;
    for ch in s.chars() {
        if n >= width {
            break;
        }
        dst.push(ch);
        n += 1;
    }
    while n < width {
        dst.push(' ');
        n += 1;
    }
}

pub fn default_dims() -> (usize, usize) {
    (DEFAULT_ROWS, DEFAULT_COLS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    #[test]
    fn nano_edit_search_replace_undo_save_roundtrip() {
        let mut fs_mock: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let (rows, cols) = default_dims();
        let mut doc = DocumentBuffer::from_bytes(Some("/tmp/test.c".into()), b"int x = 1;\n").unwrap();
        let mut tui = RockTui::new(doc.clone(), rows, cols);

        let mut save = |path: &str, bytes: &[u8]| -> Result<(), String> {
            fs_mock.insert(String::from(path), bytes.to_vec());
            Ok(())
        };

        // End of line 1, type comment, undo, redo, search & replace all, save (^O + Enter), exit (^X).
        tui.feed_bytes(b"\x05 /* a */\x1a\x19", &mut save);
        assert_eq!(tui.doc.text(), "int x = 1; /* a */\n");

        // ^R x Enter 42 Enter a
        tui.feed_bytes(b"\x12x\r42\ra", &mut save);
        assert_eq!(tui.doc.text(), "int 42 = 1; /* a */\n");

        // ^O Enter ^X
        tui.feed_bytes(b"\x0f\r\x18", &mut save);
        assert!(tui.quit);
        let saved = fs_mock.get("/tmp/test.c").expect("saved file");
        doc = DocumentBuffer::from_bytes(Some("/tmp/test.c".into()), saved).unwrap();
        assert_eq!(
            Some(doc.content_sha256_hex()),
            tui.last_saved_sha256
        );
        let ansi = tui.render_ansi();
        assert!(ansi.contains("Dunit rock 1.0"));
    }
}
