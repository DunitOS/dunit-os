//! Hybrid nano + Vim TUI engine for `rock`:
//! - Starts in nano-compatible Insert mode by default (or Vim Normal via `--vim`)
//! - Supports `Esc` -> Vim Normal / Visual (`v`/`V`) / Ex Command (`:`) modes,
//!   counts, operators (`d`/`c`/`y`/`>`/`<`/`gc`), text objects (`iw`/`aw`),
//!   bracket match `%`, char find `f/F/t/T`, word search `*`/`#`, `/`/`n`/`N`
//! - Truecolor ANSI syntax highlighting via `dunit_editor::syntax`, bracket pair
//!   highlights, search highlights, cursorline shading, and optional relative numbers.
use alloc::{format, string::String, vec::Vec};
use core::ops::Range;
use dunit_editor::{
    compute_line_states, highlight_line, DocumentBuffer, HighlightSpan, Language, SHORTCUT_HINTS,
    TokenKind,
};
use dunit_shell::Utf8Decoder;

const DEFAULT_ROWS: usize = 24;
const DEFAULT_COLS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorTheme {
    Mocha,
    OneDark,
    Gruvbox,
}

impl EditorTheme {
    pub fn parse_name(name: &str) -> Option<Self> {
        let low = name.trim().to_ascii_lowercase();
        match low.as_str() {
            "mocha" | "catppuccin" | "catppuccin_mocha" => Some(EditorTheme::Mocha),
            "onedark" | "one-dark" | "dark" => Some(EditorTheme::OneDark),
            "gruvbox" => Some(EditorTheme::Gruvbox),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            EditorTheme::Mocha => "mocha",
            EditorTheme::OneDark => "onedark",
            EditorTheme::Gruvbox => "gruvbox",
        }
    }

    fn rgb_for_token(self, kind: TokenKind) -> (u8, u8, u8) {
        match self {
            EditorTheme::Mocha => match kind {
                TokenKind::Plain => (205, 214, 244),
                TokenKind::Keyword => (203, 166, 247),
                TokenKind::Type => (249, 226, 175),
                TokenKind::Constant => (250, 179, 135),
                TokenKind::Function => (137, 180, 250),
                TokenKind::String => (166, 227, 161),
                TokenKind::Number => (250, 179, 135),
                TokenKind::Comment => (127, 132, 156),
                TokenKind::Preprocessor => (148, 226, 213),
                TokenKind::Operator => (137, 220, 235),
                TokenKind::Heading => (243, 139, 168),
            },
            EditorTheme::OneDark => match kind {
                TokenKind::Plain => (171, 178, 191),
                TokenKind::Keyword => (198, 120, 221),
                TokenKind::Type => (229, 192, 123),
                TokenKind::Constant => (209, 154, 102),
                TokenKind::Function => (97, 175, 239),
                TokenKind::String => (152, 195, 121),
                TokenKind::Number => (209, 154, 102),
                TokenKind::Comment => (92, 99, 112),
                TokenKind::Preprocessor => (86, 182, 194),
                TokenKind::Operator => (86, 182, 194),
                TokenKind::Heading => (224, 108, 117),
            },
            EditorTheme::Gruvbox => match kind {
                TokenKind::Plain => (235, 219, 178),
                TokenKind::Keyword => (251, 73, 52),
                TokenKind::Type => (250, 189, 47),
                TokenKind::Constant => (211, 134, 155),
                TokenKind::Function => (184, 187, 38),
                TokenKind::String => (184, 187, 38),
                TokenKind::Number => (211, 134, 155),
                TokenKind::Comment => (146, 131, 116),
                TokenKind::Preprocessor => (142, 192, 124),
                TokenKind::Operator => (254, 128, 25),
                TokenKind::Heading => (250, 189, 47),
            },
        }
    }

    fn ui_colors(self) -> ThemeUi {
        match self {
            EditorTheme::Mocha => ThemeUi {
                bar_bg: (24, 24, 37),
                bar_fg: (205, 214, 244),
                badge_fg: (17, 17, 27),
                mode_insert: (166, 227, 161),
                mode_normal: (137, 180, 250),
                mode_visual: (203, 166, 247),
                mode_cmd: (250, 179, 135),
                cur_line_bg: (42, 44, 62),
                sel_bg: (69, 71, 90),
                search_bg: (249, 226, 175),
                search_fg: (17, 17, 27),
                bracket_bg: (56, 82, 102),
                bracket_fg: (148, 226, 213),
                gutter_fg: (108, 112, 134),
                gutter_cur_fg: (249, 226, 175),
            },
            EditorTheme::OneDark => ThemeUi {
                bar_bg: (33, 37, 43),
                bar_fg: (171, 178, 191),
                badge_fg: (30, 34, 42),
                mode_insert: (152, 195, 121),
                mode_normal: (97, 175, 239),
                mode_visual: (198, 120, 221),
                mode_cmd: (229, 192, 123),
                cur_line_bg: (44, 49, 60),
                sel_bg: (62, 68, 81),
                search_bg: (229, 192, 123),
                search_fg: (30, 34, 42),
                bracket_bg: (59, 68, 84),
                bracket_fg: (86, 182, 194),
                gutter_fg: (75, 82, 99),
                gutter_cur_fg: (229, 192, 123),
            },
            EditorTheme::Gruvbox => ThemeUi {
                bar_bg: (60, 56, 54),
                bar_fg: (235, 219, 178),
                badge_fg: (40, 40, 40),
                mode_insert: (184, 187, 38),
                mode_normal: (131, 165, 152),
                mode_visual: (211, 134, 155),
                mode_cmd: (250, 189, 47),
                cur_line_bg: (50, 48, 47),
                sel_bg: (80, 73, 69),
                search_bg: (250, 189, 47),
                search_fg: (40, 40, 40),
                bracket_bg: (69, 133, 136),
                bracket_fg: (251, 241, 199),
                gutter_fg: (124, 111, 100),
                gutter_cur_fg: (250, 189, 47),
            },
        }
    }
}

#[derive(Clone, Copy)]
struct ThemeUi {
    bar_bg: (u8, u8, u8),
    bar_fg: (u8, u8, u8),
    badge_fg: (u8, u8, u8),
    mode_insert: (u8, u8, u8),
    mode_normal: (u8, u8, u8),
    mode_visual: (u8, u8, u8),
    mode_cmd: (u8, u8, u8),
    cur_line_bg: (u8, u8, u8),
    sel_bg: (u8, u8, u8),
    search_bg: (u8, u8, u8),
    search_fg: (u8, u8, u8),
    bracket_bg: (u8, u8, u8),
    bracket_fg: (u8, u8, u8),
    gutter_fg: (u8, u8, u8),
    gutter_cur_fg: (u8, u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    Insert,
    Normal,
    Visual,
    VisualLine,
}

impl EditMode {
    pub fn label(self) -> &'static str {
        match self {
            EditMode::Insert => "INSERT",
            EditMode::Normal => "NORMAL",
            EditMode::Visual => "VISUAL",
            EditMode::VisualLine => "V-LINE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingOp {
    None,
    Delete,
    Change,
    Yank,
    IndentRight,
    IndentLeft,
    GPrefix,
    CommentOperator,
    TextObj { op: PendingTextOp, around: bool },
    FindChar { forward: bool, till: bool, op: Option<PendingTextOp> },
    ReplaceChar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingTextOp {
    Delete,
    Change,
    Yank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorOptions {
    pub show_numbers: bool,
    pub relative_numbers: bool,
    pub cursor_line: bool,
    pub hl_search: bool,
    pub auto_indent: bool,
    pub syntax_override: Option<Language>,
    pub theme: EditorTheme,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            show_numbers: true,
            relative_numbers: false,
            cursor_line: true,
            hl_search: true,
            auto_indent: true,
            syntax_override: None,
            theme: EditorTheme::Mocha,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptMode {
    None,
    SavePath(String),
    ConfirmExit,
    Search(String),
    VimSearch { query: String, forward: bool },
    ExCommand(String),
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
    pub mode: EditMode,
    pub options: EditorOptions,
    pub rows: usize,
    pub cols: usize,
    pub top_line: usize,
    pub left_col: usize,
    pub mark_mode: bool,
    pub prompt: PromptMode,
    pub status_msg: String,
    pub last_query: String,
    pub last_find_char: Option<(char, bool, bool)>,
    pub last_saved_sha256: Option<String>,
    pub quit: bool,
    count_prefix: Option<usize>,
    pending_op: PendingOp,
    esc: EscState,
    csi_buf: String,
    utf8: Utf8Decoder,
}

impl RockTui {
    pub fn with_mode(doc: DocumentBuffer, rows: usize, cols: usize, mode: EditMode) -> Self {
        let lang = Language::from_path_and_text(doc.path(), doc.text());
        let path_note = match doc.path() {
            Some(p) => format!("Opened {} [{}]", p, lang.display_name()),
            None => String::from("New buffer"),
        };
        Self {
            doc,
            mode,
            options: EditorOptions::default(),
            rows: rows.max(8),
            cols: cols.max(32),
            top_line: 0,
            left_col: 0,
            mark_mode: false,
            prompt: PromptMode::None,
            status_msg: path_note,
            last_query: String::new(),
            last_find_char: None,
            last_saved_sha256: None,
            quit: false,
            count_prefix: None,
            pending_op: PendingOp::None,
            esc: EscState::Normal,
            csi_buf: String::new(),
            utf8: Utf8Decoder::default(),
        }
    }

    pub fn active_language(&self) -> Language {
        self.options
            .syntax_override
            .unwrap_or_else(|| Language::from_path_and_text(self.doc.path(), self.doc.text()))
    }

    fn gutter_width(&self) -> usize {
        if self.options.show_numbers {
            6
        } else {
            1
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
        let gutter = self.gutter_width();
        let vw = self.cols.saturating_sub(gutter + 1).max(8);
        if pos.col < self.left_col {
            self.left_col = pos.col;
        } else if pos.col >= self.left_col + vw {
            self.left_col = pos.col + 1 - vw;
        }
    }

    pub fn flush_pending_esc(&mut self) -> bool {
        if self.esc != EscState::Esc {
            return false;
        }
        self.esc = EscState::Normal;
        self.handle_escape_key();
        self.sync_viewport();
        true
    }

    fn handle_escape_key(&mut self) {
        if !matches!(self.prompt, PromptMode::None) {
            self.prompt = PromptMode::None;
            self.status_msg = String::from("Cancelled");
            return;
        }
        if self.pending_op != PendingOp::None || self.count_prefix.is_some() {
            self.pending_op = PendingOp::None;
            self.count_prefix = None;
            return;
        }
        match self.mode {
            EditMode::Insert => {
                self.mark_mode = false;
                self.mode = EditMode::Normal;
                self.status_msg = String::from("-- NORMAL --");
            }
            EditMode::Visual | EditMode::VisualLine => {
                let c = self.doc.cursor_offset();
                self.doc.set_cursor_range(c, c);
                self.mark_mode = false;
                self.mode = EditMode::Normal;
                self.status_msg = String::from("-- NORMAL --");
            }
            EditMode::Normal => {
                self.mark_mode = false;
                let c = self.doc.cursor_offset();
                self.doc.set_cursor_range(c, c);
            }
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
                } else if b == b'O' && matches!(self.prompt, PromptMode::None) && self.mode == EditMode::Insert {
                    self.esc = EscState::Ss3;
                    false
                } else if b == 0x1B {
                    self.esc = EscState::Normal;
                    self.handle_escape_key();
                    self.sync_viewport();
                    true
                } else {
                    // Bare Esc followed immediately by another key (e.g. scripted Vim `\edd` or `:wq`)
                    self.esc = EscState::Normal;
                    self.handle_escape_key();
                    let Some(ch) = self.utf8.push(b) else {
                        return true;
                    };
                    self.handle_char(ch, save_fn);
                    true
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
                        self.doc.move_home(self.selection_extend());
                        self.sync_viewport();
                        true
                    }
                    b'F' => {
                        self.doc.move_end(self.selection_extend());
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

    fn selection_extend(&self) -> bool {
        self.mark_mode || matches!(self.mode, EditMode::Visual | EditMode::VisualLine)
    }

    fn handle_csi(&mut self, params: &str, final_byte: u8) {
        if !matches!(self.prompt, PromptMode::None) {
            return;
        }
        let mut parts = params.split(';');
        let p1 = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
        let mods = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
        let shift = mods == 2 || mods == 6 || self.selection_extend();
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

    fn save_to_path<F>(&mut self, path: &str, save_fn: &mut F) -> bool
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        if path.trim().is_empty() {
            self.status_msg = String::from("Save aborted: empty filename");
            return false;
        }
        match save_fn(path, self.doc.text().as_bytes()) {
            Ok(()) => {
                self.doc.set_path(String::from(path));
                self.doc.mark_saved();
                let hash = self.doc.content_sha256_hex();
                self.status_msg = format!(
                    "Wrote {} B to {} (sha256:{})",
                    self.doc.bytes_len(),
                    path,
                    &hash[..12]
                );
                self.last_saved_sha256 = Some(hash);
                true
            }
            Err(err) => {
                self.status_msg = format!("Save failed: {}", err);
                false
            }
        }
    }

    fn handle_shared_nano_ctrl<F>(&mut self, b: u8, save_fn: &mut F) -> bool
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        match b {
            0x07 => {
                // ^G Help
                self.toggle_help();
                true
            }
            0x0F => {
                // ^O Save
                let initial = String::from(self.doc.path().unwrap_or(""));
                self.prompt = PromptMode::SavePath(initial);
                true
            }
            0x18 => {
                // ^X Exit
                if self.doc.is_dirty() {
                    self.prompt = PromptMode::ConfirmExit;
                } else {
                    self.quit = true;
                }
                true
            }
            0x17 => {
                // ^W Search
                self.prompt = PromptMode::Search(self.last_query.clone());
                true
            }
            0x1A => {
                // ^Z Undo
                self.status_msg = if self.doc.undo() {
                    String::from("Undo")
                } else {
                    String::from("Nothing to undo")
                };
                true
            }
            0x1F | 0x0C => {
                // ^_ Goto line
                self.prompt = PromptMode::GotoLine(String::new());
                true
            }
            _ => {
                let _ = save_fn;
                false
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
        match self.mode {
            EditMode::Insert => self.handle_insert_char(ch, b, save_fn),
            EditMode::Normal => self.handle_normal_char(ch, b, save_fn),
            EditMode::Visual | EditMode::VisualLine => self.handle_visual_char(ch, b, save_fn),
        }
        self.sync_viewport();
    }

    fn handle_insert_char<F>(&mut self, ch: char, b: u8, save_fn: &mut F)
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        if self.handle_shared_nano_ctrl(b, save_fn) {
            return;
        }
        match b {
            0x12 => self.prompt = PromptMode::ReplaceQuery(self.last_query.clone()), // ^R Replace
            0x19 => {
                self.status_msg = if self.doc.redo() {
                    String::from("Redo")
                } else {
                    String::from("Nothing to redo")
                };
            }
            0x0B => {
                if self.doc.cut_selection_or_line() {
                    self.mark_mode = false;
                    self.status_msg = String::from("Cut to buffer");
                }
            }
            0x15 => {
                let _ = self.doc.paste_clipboard();
                self.mark_mode = false;
            }
            0x1E => {
                self.mark_mode = !self.mark_mode;
                self.status_msg = if self.mark_mode {
                    String::from("Mark Set")
                } else {
                    String::from("Mark Unset")
                };
            }
            0x01 => self.doc.move_home(self.mark_mode),
            0x05 => self.doc.move_end(self.mark_mode),
            0x06 => self.find_saved_next(true),
            b'\r' | b'\n' => {
                let _ = self.doc.insert_newline_indented(self.options.auto_indent);
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
    }

    fn take_count(&mut self) -> usize {
        self.count_prefix.take().unwrap_or(1).max(1)
    }

    fn handle_normal_char<F>(&mut self, ch: char, b: u8, save_fn: &mut F)
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        // One-key immediate follow-ups first (`r<c>`, `f<c>`, `t<c>`, `i`/`a` text obj)
        match self.pending_op {
            PendingOp::ReplaceChar => {
                self.pending_op = PendingOp::None;
                let count = self.take_count();
                for _ in 0..count {
                    if !self.doc.replace_char_at_cursor(ch) {
                        break;
                    }
                    if count > 1 {
                        self.doc.move_right(false);
                    }
                }
                return;
            }
            PendingOp::FindChar { forward, till, op } => {
                self.pending_op = PendingOp::None;
                let count = self.take_count();
                self.last_find_char = Some((ch, forward, till));
                self.apply_char_find(ch, forward, till, count, op);
                return;
            }
            PendingOp::TextObj { op, around } => {
                self.pending_op = PendingOp::None;
                self.count_prefix = None;
                if ch == 'w' {
                    if let Some(r) = self.doc.word_bounds_around(self.doc.cursor_offset(), around) {
                        self.apply_text_op_range(op, r);
                    }
                }
                return;
            }
            PendingOp::Delete | PendingOp::Change | PendingOp::Yank => {
                let text_op = match self.pending_op {
                    PendingOp::Delete => PendingTextOp::Delete,
                    PendingOp::Change => PendingTextOp::Change,
                    PendingOp::Yank => PendingTextOp::Yank,
                    _ => unreachable!(),
                };
                if ch == 'i' || ch == 'a' {
                    self.pending_op = PendingOp::TextObj {
                        op: text_op,
                        around: ch == 'a',
                    };
                    return;
                }
                if let Some((fwd, till)) = match ch {
                    'f' => Some((true, false)),
                    'F' => Some((false, false)),
                    't' => Some((true, true)),
                    'T' => Some((false, true)),
                    _ => None,
                } {
                    self.pending_op = PendingOp::FindChar {
                        forward: fwd,
                        till,
                        op: Some(text_op),
                    };
                    return;
                }
                let count = self.take_count();
                self.pending_op = PendingOp::None;
                match (text_op, ch) {
                    (PendingTextOp::Delete, 'd') => {
                        if self.doc.delete_lines(count) {
                            self.status_msg = format!("Deleted {} line(s)", count);
                        }
                    }
                    (PendingTextOp::Yank, 'y') => {
                        if self.doc.yank_lines(count) {
                            self.status_msg = format!("Yanked {} line(s)", count);
                        }
                    }
                    (PendingTextOp::Change, 'c') => {
                        if self.doc.delete_lines(count) {
                            let _ = self.doc.open_line(true, self.options.auto_indent);
                            self.mode = EditMode::Insert;
                        }
                    }
                    _ => {
                        let start = self.doc.cursor_offset();
                        self.doc.set_cursor_range(start, start);
                        if self.run_simple_motion(ch, count, true) {
                            let mut r = self.doc.selection_range();
                            if matches!(ch, 'e' | '$' | '%' | 'l') && r.end < self.doc.text().len() {
                                // Inclusive Vim motions include the character at destination
                                let tail = &self.doc.text()[r.end..];
                                if let Some(first) = tail.chars().next() {
                                    if first != '\n' && (matches!(ch, 'e' | '%')) {
                                        r.end += first.len_utf8();
                                    }
                                }
                            }
                            self.apply_text_op_range(text_op, r);
                        }
                    }
                }
                return;
            }
            PendingOp::IndentRight => {
                self.pending_op = PendingOp::None;
                let count = self.take_count();
                if ch == '>' {
                    let line = self.doc.cursor_pos().line;
                    let n = self.doc.indent_lines(line, line + count - 1, true);
                    self.status_msg = format!("Indented {} line(s)", n);
                }
                return;
            }
            PendingOp::IndentLeft => {
                self.pending_op = PendingOp::None;
                let count = self.take_count();
                if ch == '<' {
                    let line = self.doc.cursor_pos().line;
                    let n = self.doc.indent_lines(line, line + count - 1, false);
                    self.status_msg = format!("Dedented {} line(s)", n);
                }
                return;
            }
            PendingOp::GPrefix => {
                self.pending_op = PendingOp::None;
                match ch {
                    'g' => {
                        if let Some(n) = self.count_prefix.take() {
                            let p = self.doc.goto_line(n);
                            self.status_msg = format!("Line {}", p.line + 1);
                        } else {
                            self.doc.move_doc_start(false);
                        }
                    }
                    'c' => {
                        self.pending_op = PendingOp::CommentOperator;
                    }
                    _ => {
                        self.count_prefix = None;
                    }
                }
                return;
            }
            PendingOp::CommentOperator => {
                self.pending_op = PendingOp::None;
                let count = self.take_count();
                if ch == 'c' {
                    let line = self.doc.cursor_pos().line;
                    let prefix = self.active_language().comment_prefix();
                    let n = self.doc.toggle_comment_lines(line, line + count - 1, prefix);
                    self.status_msg = format!("Toggled comment on {} line(s)", n);
                }
                return;
            }
            PendingOp::None => {}
        }

        if self.handle_shared_nano_ctrl(b, save_fn) {
            self.count_prefix = None;
            return;
        }
        // In Vim Normal mode: Ctrl+R redoes, Ctrl+D/U half-page scroll!
        if b == 0x12 {
            self.count_prefix = None;
            self.status_msg = if self.doc.redo() {
                String::from("Redo")
            } else {
                String::from("Already at newest change")
            };
            return;
        }
        if b == 0x04 {
            let step = (self.viewport_lines() / 2).max(1);
            self.doc.move_down(step, false);
            return;
        }
        if b == 0x15 {
            let step = (self.viewport_lines() / 2).max(1);
            self.doc.move_up(step, false);
            return;
        }
        if b == 0x19 {
            self.status_msg = if self.doc.redo() {
                String::from("Redo")
            } else {
                String::from("Nothing to redo")
            };
            return;
        }

        // Count digits (`1..=9`, or `0` appended after a nonzero count)
        if ch.is_ascii_digit() && (ch != '0' || self.count_prefix.is_some()) {
            let d = (ch as u8 - b'0') as usize;
            let next = self.count_prefix.unwrap_or(0).saturating_mul(10).saturating_add(d);
            self.count_prefix = Some(next.min(9999));
            return;
        }

        match ch {
            'i' => {
                self.count_prefix = None;
                self.mode = EditMode::Insert;
                self.status_msg = String::from("-- INSERT --");
            }
            'I' => {
                self.count_prefix = None;
                self.doc.move_first_non_blank(false);
                self.mode = EditMode::Insert;
                self.status_msg = String::from("-- INSERT --");
            }
            'a' => {
                self.count_prefix = None;
                let pos = self.doc.cursor_pos();
                if let Some((_, end)) = self.doc.line_byte_bounds(pos.line) {
                    if self.doc.cursor_offset() < end {
                        self.doc.move_right(false);
                    }
                }
                self.mode = EditMode::Insert;
                self.status_msg = String::from("-- INSERT --");
            }
            'A' => {
                self.count_prefix = None;
                self.doc.move_end(false);
                self.mode = EditMode::Insert;
                self.status_msg = String::from("-- INSERT --");
            }
            'o' => {
                self.count_prefix = None;
                let _ = self.doc.open_line(false, self.options.auto_indent);
                self.mode = EditMode::Insert;
                self.status_msg = String::from("-- INSERT --");
            }
            'O' => {
                self.count_prefix = None;
                let _ = self.doc.open_line(true, self.options.auto_indent);
                self.mode = EditMode::Insert;
                self.status_msg = String::from("-- INSERT --");
            }
            'v' => {
                self.count_prefix = None;
                let cur = self.doc.cursor_offset();
                self.doc.set_cursor_range(cur, cur);
                self.mode = EditMode::Visual;
                self.status_msg = String::from("-- VISUAL --");
            }
            'V' => {
                self.count_prefix = None;
                let cur = self.doc.cursor_offset();
                self.doc.set_cursor_range(cur, cur);
                self.mode = EditMode::VisualLine;
                self.status_msg = String::from("-- VISUAL LINE --");
            }
            ':' => {
                self.count_prefix = None;
                self.prompt = PromptMode::ExCommand(String::new());
            }
            '/' => {
                self.count_prefix = None;
                self.prompt = PromptMode::VimSearch {
                    query: String::new(),
                    forward: true,
                };
            }
            '?' => {
                self.count_prefix = None;
                self.prompt = PromptMode::VimSearch {
                    query: String::new(),
                    forward: false,
                };
            }
            'd' => self.pending_op = PendingOp::Delete,
            'c' => self.pending_op = PendingOp::Change,
            'y' => self.pending_op = PendingOp::Yank,
            '>' => self.pending_op = PendingOp::IndentRight,
            '<' => self.pending_op = PendingOp::IndentLeft,
            'g' => self.pending_op = PendingOp::GPrefix,
            'r' => self.pending_op = PendingOp::ReplaceChar,
            'f' => self.pending_op = PendingOp::FindChar { forward: true, till: false, op: None },
            'F' => self.pending_op = PendingOp::FindChar { forward: false, till: false, op: None },
            't' => self.pending_op = PendingOp::FindChar { forward: true, till: true, op: None },
            'T' => self.pending_op = PendingOp::FindChar { forward: false, till: true, op: None },
            ';' => {
                let count = self.take_count();
                if let Some((c, fwd, till)) = self.last_find_char {
                    self.apply_char_find(c, fwd, till, count, None);
                }
            }
            ',' => {
                let count = self.take_count();
                if let Some((c, fwd, till)) = self.last_find_char {
                    self.apply_char_find(c, !fwd, till, count, None);
                }
            }
            'G' => {
                if let Some(n) = self.count_prefix.take() {
                    let p = self.doc.goto_line(n);
                    self.status_msg = format!("Line {}", p.line + 1);
                } else {
                    self.doc.move_doc_end(false);
                }
            }
            'x' => {
                let count = self.take_count();
                let mut deleted = 0usize;
                for _ in 0..count {
                    if self.doc.delete_forward() {
                        deleted += 1;
                    } else {
                        break;
                    }
                }
                if deleted > 0 {
                    self.status_msg = format!("Deleted {} char(s)", deleted);
                }
            }
            'X' => {
                let count = self.take_count();
                for _ in 0..count {
                    if !self.doc.backspace() {
                        break;
                    }
                }
            }
            'D' => {
                self.count_prefix = None;
                let cur = self.doc.cursor_offset();
                self.doc.move_end(true);
                let r = self.doc.selection_range();
                if self.doc.delete_range(r, true, false) {
                    self.status_msg = String::from("Deleted to end of line");
                } else {
                    self.doc.set_cursor_range(cur, cur);
                }
            }
            'C' => {
                self.count_prefix = None;
                self.doc.move_end(true);
                let r = self.doc.selection_range();
                let _ = self.doc.delete_range(r, true, false);
                self.mode = EditMode::Insert;
                self.status_msg = String::from("-- INSERT --");
            }
            'S' => {
                let count = self.take_count();
                if self.doc.delete_lines(count) {
                    let _ = self.doc.open_line(true, self.options.auto_indent);
                }
                self.mode = EditMode::Insert;
                self.status_msg = String::from("-- INSERT --");
            }
            'Y' => {
                let count = self.take_count();
                if self.doc.yank_lines(count) {
                    self.status_msg = format!("Yanked {} line(s)", count);
                }
            }
            'p' => {
                self.count_prefix = None;
                if let Ok(true) = self.doc.paste_vim(false) {
                    self.status_msg = String::from("Pasted");
                }
            }
            'P' => {
                self.count_prefix = None;
                if let Ok(true) = self.doc.paste_vim(true) {
                    self.status_msg = String::from("Pasted before");
                }
            }
            'J' => {
                self.count_prefix = None;
                if self.doc.join_next_line() {
                    self.status_msg = String::from("Joined line");
                }
            }
            'u' => {
                self.count_prefix = None;
                self.status_msg = if self.doc.undo() {
                    String::from("Undo")
                } else {
                    String::from("Already at oldest change")
                };
            }
            'n' => {
                self.count_prefix = None;
                self.find_saved_next(true);
            }
            'N' => {
                self.count_prefix = None;
                self.find_saved_next(false);
            }
            '*' | '#' => {
                self.count_prefix = None;
                if let Some(word) = self.doc.word_under_cursor() {
                    self.last_query = word;
                    self.find_saved_next(ch == '*');
                } else {
                    self.status_msg = String::from("No word under cursor");
                }
            }
            _ => {
                let count = self.take_count();
                let _ = self.run_simple_motion(ch, count, false);
            }
        }
    }

    fn handle_visual_char<F>(&mut self, ch: char, b: u8, save_fn: &mut F)
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        if self.handle_shared_nano_ctrl(b, save_fn) {
            return;
        }
        if self.pending_op == PendingOp::GPrefix {
            self.pending_op = PendingOp::None;
            match ch {
                'g' => self.doc.move_doc_start(true),
                'c' => {
                    let (a, b_ln) = self.doc.selected_line_bounds();
                    let prefix = self.active_language().comment_prefix();
                    let n = self.doc.toggle_comment_lines(a, b_ln, prefix);
                    self.mode = EditMode::Normal;
                    self.status_msg = format!("Toggled comment on {} line(s)", n);
                }
                _ => {}
            }
            return;
        }
        match ch {
            'v' if self.mode == EditMode::Visual => {
                let c = self.doc.cursor_offset();
                self.doc.set_cursor_range(c, c);
                self.mode = EditMode::Normal;
            }
            'V' if self.mode == EditMode::VisualLine => {
                let c = self.doc.cursor_offset();
                self.doc.set_cursor_range(c, c);
                self.mode = EditMode::Normal;
            }
            'd' | 'x' | 'y' | 'c' | 's' => {
                let op = match ch {
                    'y' => PendingTextOp::Yank,
                    'c' | 's' => PendingTextOp::Change,
                    _ => PendingTextOp::Delete,
                };
                if self.mode == EditMode::VisualLine {
                    let (a, b_ln) = self.doc.selected_line_bounds();
                    let count = (b_ln - a) + 1;
                    self.doc.goto_line(a + 1);
                    match op {
                        PendingTextOp::Yank => {
                            let _ = self.doc.yank_lines(count);
                            self.status_msg = format!("Yanked {} line(s)", count);
                            self.mode = EditMode::Normal;
                        }
                        PendingTextOp::Delete => {
                            let _ = self.doc.delete_lines(count);
                            self.status_msg = format!("Deleted {} line(s)", count);
                            self.mode = EditMode::Normal;
                        }
                        PendingTextOp::Change => {
                            let _ = self.doc.delete_lines(count);
                            let _ = self.doc.open_line(true, self.options.auto_indent);
                            self.mode = EditMode::Insert;
                        }
                    }
                } else {
                    let r = self.doc.selection_range();
                    self.apply_text_op_range(op, r);
                    if op != PendingTextOp::Change {
                        self.mode = EditMode::Normal;
                    }
                }
            }
            '>' | '<' => {
                let (a, b_ln) = self.doc.selected_line_bounds();
                let n = self.doc.indent_lines(a, b_ln, ch == '>');
                self.mode = EditMode::Normal;
                self.status_msg = format!("{} {} line(s)", if ch == '>' { "Indented" } else { "Dedented" }, n);
            }
            'g' => self.pending_op = PendingOp::GPrefix,
            'G' => self.doc.move_doc_end(true),
            _ => {
                let _ = self.run_simple_motion(ch, 1, true);
            }
        }
    }

    fn apply_char_find(
        &mut self,
        ch: char,
        forward: bool,
        till: bool,
        count: usize,
        op: Option<PendingTextOp>,
    ) {
        let extend = op.is_some() || self.selection_extend();
        if let Some(text_op) = op {
            let c = self.doc.cursor_offset();
            self.doc.set_cursor_range(c, c);
            let mut ok = false;
            for _ in 0..count {
                ok = self.doc.find_char_in_line(ch, forward, till, true);
                if !ok {
                    break;
                }
            }
            if ok {
                let mut r = self.doc.selection_range();
                if forward && r.end < self.doc.text().len() {
                    if let Some(first) = self.doc.text()[r.end..].chars().next() {
                        if first != '\n' {
                            r.end += first.len_utf8();
                        }
                    }
                }
                self.apply_text_op_range(text_op, r);
            } else {
                self.doc.set_cursor_range(c, c);
            }
        } else {
            for _ in 0..count {
                if !self.doc.find_char_in_line(ch, forward, till, extend) {
                    break;
                }
            }
        }
    }

    fn apply_text_op_range(&mut self, op: PendingTextOp, r: Range<usize>) {
        let len = r.len();
        match op {
            PendingTextOp::Yank => {
                if self.doc.yank_range(r.clone(), false) {
                    self.doc.set_cursor_range(r.start, r.start);
                    self.status_msg = format!("Yanked {} byte(s)", len);
                }
            }
            PendingTextOp::Delete => {
                if self.doc.delete_range(r, true, false) {
                    self.status_msg = format!("Deleted {} byte(s)", len);
                }
            }
            PendingTextOp::Change => {
                let _ = self.doc.delete_range(r, true, false);
                self.mode = EditMode::Insert;
                self.status_msg = String::from("-- INSERT --");
            }
        }
    }

    fn run_simple_motion(&mut self, ch: char, count: usize, extend: bool) -> bool {
        match ch {
            'h' => {
                for _ in 0..count {
                    self.doc.move_left(extend);
                }
                true
            }
            'l' => {
                for _ in 0..count {
                    self.doc.move_right(extend);
                }
                true
            }
            'j' => {
                self.doc.move_down(count, extend);
                true
            }
            'k' => {
                self.doc.move_up(count, extend);
                true
            }
            'w' => {
                for _ in 0..count {
                    self.doc.move_word_right(extend);
                }
                true
            }
            'b' => {
                for _ in 0..count {
                    self.doc.move_word_left(extend);
                }
                true
            }
            'e' => {
                for _ in 0..count {
                    self.doc.move_word_end(extend);
                }
                true
            }
            '0' => {
                self.doc.move_home(extend);
                true
            }
            '^' => {
                self.doc.move_first_non_blank(extend);
                true
            }
            '$' => {
                self.doc.move_end(extend);
                true
            }
            '%' => self.doc.move_matching_bracket(extend),
            '{' => {
                for _ in 0..count {
                    self.doc.move_paragraph_prev(extend);
                }
                true
            }
            '}' => {
                for _ in 0..count {
                    self.doc.move_paragraph_next(extend);
                }
                true
            }
            _ => false,
        }
    }

    fn find_saved_next(&mut self, forward: bool) {
        if self.last_query.is_empty() {
            self.status_msg = String::from("No previous search pattern");
            return;
        }
        let q = self.last_query.clone();
        let hit = if forward {
            self.doc.find_next(&q, false)
        } else {
            self.doc.find_prev(&q, false)
        };
        if let Some(r) = hit {
            let p = self.doc.pos_at(r.start);
            self.doc.set_cursor_range(r.start, r.start);
            self.status_msg = format!("Match '{}' at {}:{}", q, p.line + 1, p.col + 1);
        } else {
            self.status_msg = format!("Pattern not found: {}", q);
        }
    }

    fn execute_ex_command<F>(&mut self, raw: &str, save_fn: &mut F)
    where
        F: FnMut(&str, &[u8]) -> Result<(), String>,
    {
        let cmd = raw.trim();
        if cmd.is_empty() {
            return;
        }
        if let Ok(line) = cmd.parse::<usize>() {
            let p = self.doc.goto_line(line.max(1));
            self.status_msg = format!("Line {}", p.line + 1);
            return;
        }
        if cmd == "q" || cmd == "quit" {
            if self.doc.is_dirty() {
                self.status_msg = String::from("No write since last change (use :q! to override)");
            } else {
                self.quit = true;
            }
            return;
        }
        if cmd == "q!" || cmd == "quit!" {
            self.quit = true;
            return;
        }
        if cmd == "w" {
            if let Some(p) = self.doc.path().map(String::from) {
                let _ = self.save_to_path(&p, save_fn);
            } else {
                self.prompt = PromptMode::SavePath(String::new());
            }
            return;
        }
        if let Some(path) = cmd.strip_prefix("w ") {
            let _ = self.save_to_path(path.trim(), save_fn);
            return;
        }
        if cmd == "wq" || cmd == "x" {
            if let Some(p) = self.doc.path().map(String::from) {
                if self.save_to_path(&p, save_fn) {
                    self.quit = true;
                }
            } else {
                self.prompt = PromptMode::SavePath(String::new());
            }
            return;
        }
        if cmd == "noh" || cmd == "nohlsearch" {
            self.last_query.clear();
            self.status_msg = String::from("Cleared search highlight");
            return;
        }
        if cmd == "help" || cmd == "h" {
            self.prompt = PromptMode::Help;
            return;
        }
        if let Some(arg) = cmd.strip_prefix("set ") {
            self.execute_set(arg.trim());
            return;
        }
        if let Some(arg) = cmd.strip_prefix("theme ") {
            if let Some(t) = EditorTheme::parse_name(arg) {
                self.options.theme = t;
                self.status_msg = format!("Theme: {}", t.name());
            } else {
                self.status_msg = String::from("Themes: mocha, onedark, gruvbox");
            }
            return;
        }
        if let Some(sub) = cmd.strip_prefix("%s").or_else(|| cmd.strip_prefix("s")) {
            if sub.chars().next().is_some_and(|c| !c.is_ascii_alphanumeric()) {
                let global = cmd.starts_with("%s");
                self.execute_substitute(sub, global);
                return;
            }
        }
        self.status_msg = format!("Unknown command: :{}", cmd);
    }

    fn execute_substitute(&mut self, rest: &str, all_doc: bool) {
        let mut chars = rest.chars();
        let Some(delim) = chars.next() else {
            self.status_msg = String::from("Usage: :%s/old/new/g");
            return;
        };
        let parts: Vec<&str> = chars.as_str().split(delim).collect();
        if parts.len() < 2 || parts[0].is_empty() {
            self.status_msg = String::from("Usage: :%s/old/new/[gi]");
            return;
        }
        let query = parts[0];
        let repl = parts[1];
        let flags = parts.get(2).copied().unwrap_or("");
        let case_sensitive = !flags.contains('i');
        let global = flags.contains('g') || all_doc;
        if all_doc || global {
            match self.doc.replace_all(query, repl, case_sensitive) {
                Ok(n) => self.status_msg = format!("Replaced {} occurrence(s)", n),
                Err(_) => self.status_msg = String::from("Substitute failed"),
            }
        } else {
            match self.doc.replace_next(query, repl, case_sensitive) {
                Ok(true) => self.status_msg = String::from("Replaced 1 occurrence"),
                Ok(false) => self.status_msg = format!("Not found: {}", query),
                Err(_) => self.status_msg = String::from("Substitute failed"),
            }
        }
    }

    fn execute_set(&mut self, spec: &str) {
        if let Some(lang_str) = spec.strip_prefix("syntax=") {
            if let Some(lang) = Language::parse_name(lang_str) {
                self.options.syntax_override = Some(lang);
                self.status_msg = format!("syntax={}", lang.display_name());
            } else {
                self.status_msg = String::from("Unknown syntax (rust, c, toml, md, sh, lua, json, off)");
            }
            return;
        }
        match spec {
            "nu" | "number" => {
                self.options.show_numbers = true;
                self.status_msg = String::from("number enabled");
            }
            "nonu" | "nonumber" => {
                self.options.show_numbers = false;
                self.status_msg = String::from("number disabled");
            }
            "rnu" | "relativenumber" => {
                self.options.show_numbers = true;
                self.options.relative_numbers = true;
                self.status_msg = String::from("relativenumber enabled");
            }
            "nornu" | "norelativenumber" => {
                self.options.relative_numbers = false;
                self.status_msg = String::from("relativenumber disabled");
            }
            "cul" | "cursorline" => {
                self.options.cursor_line = true;
                self.status_msg = String::from("cursorline enabled");
            }
            "nocul" | "nocursorline" => {
                self.options.cursor_line = false;
                self.status_msg = String::from("cursorline disabled");
            }
            "hls" | "hlsearch" => {
                self.options.hl_search = true;
                self.status_msg = String::from("hlsearch enabled");
            }
            "nohls" | "nohlsearch" => {
                self.options.hl_search = false;
                self.status_msg = String::from("hlsearch disabled");
            }
            "ai" | "autoindent" => {
                self.options.auto_indent = true;
                self.status_msg = String::from("autoindent enabled");
            }
            "noai" | "noautoindent" => {
                self.options.auto_indent = false;
                self.status_msg = String::from("autoindent disabled");
            }
            _ => self.status_msg = format!("Unknown option: {}", spec),
        }
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
                        if self.save_to_path(&p, save_fn) {
                            self.quit = true;
                        }
                    } else {
                        self.prompt = PromptMode::SavePath(String::new());
                    }
                }
                'n' => self.quit = true,
                'c' => self.status_msg = String::from("Exit cancelled"),
                _ => self.prompt = PromptMode::ConfirmExit,
            },
            PromptMode::SavePath(mut buf) => match ch {
                '\r' | '\n' => {
                    let _ = self.save_to_path(&buf, save_fn);
                }
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
                    self.last_query = q;
                    self.find_saved_next(true);
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
            PromptMode::VimSearch { mut query, forward } => match ch {
                '\r' | '\n' => {
                    self.last_query = query;
                    self.find_saved_next(forward);
                }
                '\x08' | '\x7f' => {
                    query.pop();
                    self.prompt = PromptMode::VimSearch { query, forward };
                }
                c if !c.is_control() => {
                    query.push(c);
                    self.prompt = PromptMode::VimSearch { query, forward };
                }
                _ => self.prompt = PromptMode::VimSearch { query, forward },
            },
            PromptMode::ExCommand(mut cmd) => match ch {
                '\r' | '\n' => self.execute_ex_command(&cmd, save_fn),
                '\x08' | '\x7f' => {
                    cmd.pop();
                    self.prompt = PromptMode::ExCommand(cmd);
                }
                c if !c.is_control() => {
                    cmd.push(c);
                    self.prompt = PromptMode::ExCommand(cmd);
                }
                _ => self.prompt = PromptMode::ExCommand(cmd),
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
                'c' => self.status_msg = String::from("Replace cancelled"),
                _ => self.prompt = PromptMode::ConfirmReplace { query, with },
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

    fn effective_selection(&self) -> Option<Range<usize>> {
        if self.mode == EditMode::VisualLine {
            let (a, b) = self.doc.selected_line_bounds();
            let (sa, _) = self.doc.line_byte_bounds(a)?;
            let (_, eb) = self.doc.line_byte_bounds(b)?;
            Some(sa..eb)
        } else {
            let r = self.doc.selection_range();
            if r.is_empty() {
                None
            } else {
                Some(r)
            }
        }
    }

    pub fn render_ansi(&self) -> String {
        let ui = self.options.theme.ui_colors();
        let lang = self.active_language();
        let mut out = String::with_capacity(self.rows * self.cols * 2);
        out.push_str("\x1b[?25l\x1b[H");

        // Header bar with colored mode pill + file info + syntax + cursor
        let (mode_label, mode_rgb) = match self.prompt {
            PromptMode::ExCommand(_) => ("COMMAND", ui.mode_cmd),
            PromptMode::VimSearch { .. } => ("SEARCH", ui.mode_cmd),
            _ => match self.mode {
                EditMode::Insert => (self.mode.label(), ui.mode_insert),
                EditMode::Normal => (self.mode.label(), ui.mode_normal),
                EditMode::Visual => (self.mode.label(), ui.mode_visual),
                EditMode::VisualLine => (self.mode.label(), ui.mode_visual),
            },
        };
        let badge = format!(" {} ", mode_label);
        push_bg(&mut out, mode_rgb);
        push_fg(&mut out, ui.badge_fg);
        out.push_str(&badge);

        let pos = self.doc.cursor_pos();
        let name = self.doc.path().unwrap_or("[untitled]");
        let mod_mark = if self.doc.is_dirty() { " [+]" } else { "" };
        let header_body = format!(
            " Dunit rock 2.0  {}{}  [{}]  Ln {}/{}, Col {}",
            name,
            mod_mark,
            lang.display_name(),
            pos.line + 1,
            self.doc.line_count(),
            pos.col + 1
        );
        push_bg(&mut out, ui.bar_bg);
        push_fg(&mut out, ui.bar_fg);
        let rem_hdr = self.cols.saturating_sub(badge.chars().count());
        push_padded(&mut out, &header_body, rem_hdr);
        out.push_str("\x1b[0m\x1b[K\r\n");

        let vh = self.viewport_lines();
        let gutter = self.gutter_width();
        let text_cols = self.cols.saturating_sub(gutter).max(8);

        if matches!(self.prompt, PromptMode::Help) {
            for r in 0..vh {
                if r == 0 {
                    out.push_str("  Rock 2.0  (Nano Insert default  |  Esc -> Vim Normal)  [any key returns]\x1b[K\r\n");
                } else if r <= SHORTCUT_HINTS.len() {
                    let hint = &SHORTCUT_HINTS[r - 1];
                    out.push_str(&format!("  {:14} {}\x1b[K\r\n", hint.key, hint.label));
                } else {
                    let vim_row = r - SHORTCUT_HINTS.len() - 1;
                    let vim_tips = [
                        "  -- Vim Normal Mode (press Esc) --",
                        "  Motions:   h j k l   w b e   0 ^ $   gg G   % (bracket)   f<c> t<c>",
                        "  Editing:   i I a A o O   dd yy cc p P   diw ciw yiw   >> <<   gcc",
                        "  Visual:    v / V  then d y c > < gc   |  Undo/Redo: u / Ctrl+R",
                        "  Command:   :w  :wq  :q!  :%s/old/new/g  :set rnu  :theme mocha",
                    ];
                    if let Some(tip) = vim_tips.get(vim_row) {
                        push_padded(&mut out, tip, self.cols);
                    }
                    out.push_str("\x1b[K\r\n");
                }
            }
        } else {
            let line_states = compute_line_states(self.doc.text(), lang);
            let sel = self.effective_selection();
            let bracket_pair = self.doc.matching_bracket_pair();
            let cur_line = pos.line;
            let query = if self.options.hl_search && !self.last_query.is_empty() {
                self.last_query.as_str()
            } else {
                ""
            };

            for r in 0..vh {
                let line_idx = self.top_line + r;
                let is_cur_line = self.options.cursor_line && line_idx == cur_line;
                if let Some((start, end)) = self.doc.line_byte_bounds(line_idx) {
                    if self.options.show_numbers {
                        if is_cur_line {
                            push_bg(&mut out, ui.cur_line_bg);
                            push_fg(&mut out, ui.gutter_cur_fg);
                            out.push_str(&format!("{:4}", line_idx + 1));
                        } else {
                            push_fg(&mut out, ui.gutter_fg);
                            let disp = if self.options.relative_numbers {
                                line_idx.abs_diff(cur_line)
                            } else {
                                line_idx + 1
                            };
                            out.push_str(&format!("{:4}", disp));
                        }
                        push_fg(&mut out, ui.gutter_fg);
                        out.push_str("| ");
                    } else if is_cur_line {
                        push_bg(&mut out, ui.cur_line_bg);
                    }

                    let slice = &self.doc.text()[start..end];
                    let entry = line_states.get(line_idx).copied().unwrap_or_default();
                    let (spans, _) = highlight_line(slice, lang, entry);
                    let search_hits = DocumentBuffer::find_all_line_matches(slice, query, false);

                    self.render_highlighted_slice(
                        &mut out,
                        slice,
                        start,
                        text_cols,
                        is_cur_line,
                        &spans,
                        &search_hits,
                        sel.as_ref(),
                        bracket_pair,
                        &ui,
                    );
                    out.push_str("\x1b[0m\x1b[K\r\n");
                } else {
                    push_fg(&mut out, ui.gutter_fg);
                    out.push_str("    ~\x1b[0m\x1b[K\r\n");
                }
            }
        }

        // Status / prompt row
        let prompt_line = match &self.prompt {
            PromptMode::None => format!(" {}", self.status_msg),
            PromptMode::SavePath(p) => format!(" File Name to Write: {}", p),
            PromptMode::ConfirmExit => String::from(" Save modified buffer? (Y Yes / N No / C Cancel)"),
            PromptMode::Search(q) => format!(" Search: {}", q),
            PromptMode::VimSearch { query, forward } => {
                format!(" {}{}", if *forward { '/' } else { '?' }, query)
            }
            PromptMode::ExCommand(c) => format!(" :{}", c),
            PromptMode::ReplaceQuery(q) => format!(" Search (to replace): {}", q),
            PromptMode::ReplaceWith { query, with } => {
                format!(" Replace '{}' with: {}", query, with)
            }
            PromptMode::ConfirmReplace { query, with } => {
                format!(" Replace '{}' -> '{}'? [Y/N/A(ll)/C(ancel)]", query, with)
            }
            PromptMode::GotoLine(n) => format!(" Enter line number: {}", n),
            PromptMode::Help => String::from(" Press any key to exit help"),
        };
        push_bg(&mut out, ui.bar_bg);
        push_fg(&mut out, ui.bar_fg);
        push_padded(&mut out, &prompt_line, self.cols);
        out.push_str("\x1b[0m\r\n");

        // Adaptive bottom shortcut rows
        let (row1, row2) = match self.mode {
            EditMode::Insert => (
                "^G Help  ^O Save  ^X Exit  ^W Find  ^R Replace  Esc Vim-Normal",
                "^Z Undo  ^Y Redo  ^K Cut   ^U Paste ^^ Mark     ^_ Line",
            ),
            EditMode::Normal => (
                "i Insert  v/V Visual  : Cmd  / Search  n/N Next  % Match  * Word",
                "dd Del    yy Yank     p Paste  u Undo  ^R Redo  >> Ind   gcc Comment",
            ),
            EditMode::Visual | EditMode::VisualLine => (
                "d Delete  y Yank  c Change  > Indent  < Dedent  gc Comment",
                "Motions: h j k l  w b e  0 ^ $  gg G  % Match   Esc Normal",
            ),
        };
        push_fg(&mut out, ui.gutter_fg);
        push_padded(&mut out, row1, self.cols);
        out.push_str("\x1b[0m\r\n");
        push_fg(&mut out, ui.gutter_fg);
        push_padded(&mut out, row2, self.cols);
        out.push_str("\x1b[0m");

        // Place terminal cursor
        let cur_row = 2 + pos.line.saturating_sub(self.top_line);
        let cur_col = 1 + gutter + pos.col.saturating_sub(self.left_col);
        out.push_str(&format!("\x1b[{};{}H\x1b[?25h", cur_row, cur_col));
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn render_highlighted_slice(
        &self,
        out: &mut String,
        slice: &str,
        line_abs_start: usize,
        text_cols: usize,
        is_cur_line: bool,
        spans: &[HighlightSpan],
        search_hits: &[Range<usize>],
        sel: Option<&Range<usize>>,
        bracket_pair: Option<(usize, usize)>,
        ui: &ThemeUi,
    ) {
        let mut span_idx = 0usize;
        let mut col = 0usize;
        let mut printed = 0usize;
        let mut cur_fg: Option<(u8, u8, u8)> = None;
        let mut cur_bg: Option<(u8, u8, u8)> = None;

        for (rel, ch) in slice.char_indices() {
            while span_idx < spans.len() && spans[span_idx].end <= rel {
                span_idx += 1;
            }
            if col >= self.left_col && printed < text_cols {
                let abs = line_abs_start + rel;
                let token_kind = spans
                    .get(span_idx)
                    .filter(|s| s.start <= rel && rel < s.end)
                    .map_or(TokenKind::Plain, |s| s.kind);

                let in_sel = sel.is_some_and(|r| r.start <= abs && abs < r.end);
                let in_search = search_hits.iter().any(|r| r.start <= rel && rel < r.end);
                let is_bracket = bracket_pair.is_some_and(|(a, b)| abs == a || abs == b);

                let (fg, bg) = if in_sel {
                    (self.options.theme.rgb_for_token(token_kind), Some(ui.sel_bg))
                } else if in_search {
                    (ui.search_fg, Some(ui.search_bg))
                } else if is_bracket {
                    (ui.bracket_fg, Some(ui.bracket_bg))
                } else {
                    (
                        self.options.theme.rgb_for_token(token_kind),
                        if is_cur_line { Some(ui.cur_line_bg) } else { None },
                    )
                };

                if cur_bg != bg {
                    match bg {
                        Some(rgb) => push_bg(out, rgb),
                        None => out.push_str("\x1b[49m"),
                    }
                    cur_bg = bg;
                }
                if cur_fg != Some(fg) {
                    push_fg(out, fg);
                    cur_fg = Some(fg);
                }
                out.push(ch);
                printed += 1;
            }
            col += 1;
        }
        if is_cur_line && printed < text_cols {
            if cur_bg != Some(ui.cur_line_bg) {
                push_bg(out, ui.cur_line_bg);
            }
            for _ in printed..text_cols {
                out.push(' ');
            }
        }
    }
}

fn push_fg(out: &mut String, rgb: (u8, u8, u8)) {
    out.push_str(&format!("\x1b[38;2;{};{};{}m", rgb.0, rgb.1, rgb.2));
}

fn push_bg(out: &mut String, rgb: (u8, u8, u8)) {
    out.push_str(&format!("\x1b[48;2;{};{};{}m", rgb.0, rgb.1, rgb.2));
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
        let mut doc =
            DocumentBuffer::from_bytes(Some("/tmp/test.c".into()), b"int x = 1;\n").unwrap();
        let mut tui = RockTui::with_mode(doc.clone(), rows, cols, EditMode::Insert);

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
        assert_eq!(Some(doc.content_sha256_hex()), tui.last_saved_sha256);
        let ansi = tui.render_ansi();
        assert!(ansi.contains("Dunit rock 2.0"));
        assert!(ansi.contains("[C]"));
    }

    #[test]
    fn vim_normal_visual_operators_substitute_and_syntax_ansi() {
        let mut fs_mock: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut save = |path: &str, bytes: &[u8]| -> Result<(), String> {
            fs_mock.insert(String::from(path), bytes.to_vec());
            Ok(())
        };
        let doc = DocumentBuffer::from_bytes(
            Some("/tmp/demo.rs".into()),
            b"fn main() {\n    let val = 10;\n    let two = 20;\n}\n",
        )
        .unwrap();
        let mut tui = RockTui::with_mode(doc, 22, 68, EditMode::Normal);

        // Move down to line 2, yy then p, toggle comment via gcc, then undo comment
        tui.feed_bytes(b"jyy", &mut save);
        tui.feed_bytes(b"p", &mut save);
        assert_eq!(tui.doc.line_count(), 6);
        tui.feed_bytes(b"gcc", &mut save);
        assert_eq!(tui.doc.line_slice(2), Some("    // let val = 10;"));
        tui.feed_bytes(b"u", &mut save);
        assert_eq!(tui.doc.line_slice(2), Some("    let val = 10;"));

        // Visual line select 2 lines and indent with >
        tui.feed_bytes(b"Vj>", &mut save);
        assert_eq!(tui.doc.line_slice(2), Some("        let val = 10;"));

        // Change inner word `ciw` on `val` -> `count` + standalone Esc flush
        tui.feed_bytes(b"wwciwcount", &mut save);
        tui.feed_bytes(b"\x1b", &mut save);
        assert!(tui.flush_pending_esc());
        assert_eq!(tui.doc.line_slice(2), Some("        let count = 10;"));
        assert_eq!(tui.mode, EditMode::Normal);

        // Ex command :set rnu, :%s/count/total/g, and :wq
        tui.feed_bytes(b":set rnu\r", &mut save);
        assert!(tui.options.relative_numbers);
        tui.feed_bytes(b":%s/count/total/g\r", &mut save);
        assert!(tui.doc.text().contains("total = 10"));

        let rendered = tui.render_ansi();
        assert!(rendered.contains("NORMAL"));
        assert!(rendered.contains("[Rust]"));
        assert!(rendered.contains("\x1b[38;2;203;166;247m")); // Mocha Keyword highlight

        tui.feed_bytes(b":wq\r", &mut save);
        assert!(tui.quit);
        assert!(fs_mock.contains_key("/tmp/demo.rs"));
    }
}
