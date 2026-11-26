//! Terminal cell grid and streaming parser; independent of graphics and IPC.
extern crate alloc;
use alloc::{string::String, vec::Vec};
const SCROLL_CAP: usize = 200;
/// Reserve a whole pixel cell for the font's fractional advance.
pub(super) fn cell_pitch(advance: f32) -> i32 {
    let whole = advance as i32;
    (whole + i32::from(advance > whole as f32)).max(1)
}

/// The live text palette, resolved from `[terminal]` config (`fg`, `ansi[8]`,
/// `ansi_bright[8]`). Carried in `Term` so SGR (`ESC[…m`) selects configured
/// colors instead of hardcoded constants — the single source of truth is the
/// TOML (falling back to `TerminalCfg::baseline`).
#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub(super) fg: u32,
    pub(super) ansi: [u32; 8],
    pub(super) ansi_bright: [u32; 8],
}

pub(super) const ATTR_BOLD: u8 = 1 << 0;
pub(super) const ATTR_DIM: u8 = 1 << 1;
pub(super) const ATTR_UNDERLINE: u8 = 1 << 2;
pub(super) const ATTR_INVERSE: u8 = 1 << 3;

/// One screen cell: glyph, SGR foreground, optional opaque background (`0` = inherit
/// terminal surface background), and text attribute flags.
#[derive(Clone, Copy)]
pub(super) struct Cell {
    pub(super) ch: char,
    pub(super) fg: u32,
    pub(super) bg: u32,
    pub(super) attrs: u8,
}

impl Cell {
    pub(super) fn blank(fg: u32) -> Self {
        Cell {
            ch: ' ',
            fg,
            bg: 0,
            attrs: 0,
        }
    }

    pub(super) fn blank_with_bg(fg: u32, bg: u32) -> Self {
        Cell {
            ch: ' ',
            fg,
            bg,
            attrs: 0,
        }
    }
}

/// Convert an xterm-256 color index into `0xFFRRGGBB` using the active palette
/// for indices 0..16.
fn xterm256(palette: &Palette, n: u32) -> u32 {
    if n < 8 {
        return palette.ansi[n as usize];
    }
    if n < 16 {
        return palette.ansi_bright[(n - 8) as usize];
    }
    if n >= 232 {
        let l = 8 + (n - 232).min(23) * 10;
        return 0xFF00_0000 | (l << 16) | (l << 8) | l;
    }
    let idx = (n - 16).min(215);
    let (r, g, b) = ((idx / 36) % 6, (idx / 6) % 6, idx % 6);
    let comp = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
    0xFF00_0000 | (comp(r) << 16) | (comp(g) << 8) | comp(b)
}

/// A fresh row of `cols` blank cells in the default foreground.
fn blank_row(cols: usize, fg: u32) -> Vec<Cell> {
    let mut r: Vec<Cell> = Vec::new();
    r.resize(cols, Cell::blank(fg));
    r
}

/// ESC-sequence parser state for interpreting the shell's stdout byte stream.
#[derive(Clone)]
enum Esc {
    Normal,
    Esc,
    Csi,
    Osc,
    OscEsc,
}

/// Selection endpoints are document cell boundaries, including scrollback.
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub(super) struct Point {pub row:usize,pub col:usize}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) struct Selection {pub anchor:Point,pub end:Point}
impl Selection {
    fn range(self)->(Point,Point){if self.anchor<=self.end{(self.anchor,self.end)}else{(self.end,self.anchor)}}
}

/// On-screen terminal model: a real character CELL GRID (`rows`×`cols`) with a
/// cursor at (`crow`, `ccol`), plus a `scrollback` of rows that scrolled off the
/// top. `feed` interprets the shell's bytes like a VT — printable ASCII writes at
/// the cursor and advances it (wrapping at the right edge), `\r` returns to column
/// 0, `\n` is CR+LF, `\t` tabs, `\f`/`ESC[2J` clear, `\b` steps left, and the CSI
/// cursor/erase finals (`H`/`f`/`A`/`B`/`C`/`D`/`G`/`d`/`J`/`K`/`m`) move the cursor
/// and blank cells — so `dsh`'s `\r`+text+`ESC[K`+`ESC[nD` line editing renders with
/// a correct caret, like a raw terminal.
#[derive(Clone)]
pub(super) struct Term {
    pub(super) scrollback: Vec<Vec<Cell>>,
    pub(super) grid: Vec<Vec<Cell>>,
    grid_wrap: Vec<bool>,
    scroll_wrap: Vec<bool>,
    pub(super) cols: usize,
    pub(super) rows: usize,
    pub(super) crow: usize,
    pub(super) ccol: usize,
    pub(super) fg: u32,
    pub(super) bg: u32,
    pub(super) attrs: u8,
    pub(super) dirty: bool,
    /// Rows the view is lifted above the live bottom (0 = following new output).
    pub(super) scroll: usize,
    pub(super) selection:Option<Selection>,
    pub(super) selecting:bool,
    esc: Esc,
    pub(super) params: [u32; 16],
    pub(super) nparams: usize,
    pub(super) param: u32,
    pub(super) has_param: bool,
    /// Config-resolved colors (default fg + the 16 ANSI slots).
    pub(super) palette: Palette,
    /// Monospace cell width in px (grid column pitch).
    pub(super) cell_w: i32,
    /// Logical current-line accumulator for the `[term]` serial echo — tracks the
    /// shell's line independent of on-screen column wrapping (see `flush_echo`).
    pub(super) echo: String,
    utf8_buf: [u8; 4],
    utf8_len: usize,
}

impl Term {
    pub(super) fn new(palette: Palette, cols: usize, rows: usize, cell_w: i32) -> Self {
        let mut grid: Vec<Vec<Cell>> = Vec::new();
        for _ in 0..rows {
            grid.push(blank_row(cols, palette.fg));
        }
        Term {
            scrollback: Vec::new(),
            grid,
            grid_wrap: alloc::vec![false; rows],
            scroll_wrap: Vec::new(),
            cols,
            rows,
            crow: 0,
            ccol: 0,
            fg: palette.fg,
            bg: 0,
            attrs: 0,
            dirty: true,
            scroll: 0,
            selection:None,
            selecting:false,
            esc: Esc::Normal,
            params: [0; 16],
            nparams: 0,
            param: 0,
            has_param: false,
            palette,
            cell_w,
            echo: String::new(),
            utf8_buf: [0; 4],
            utf8_len: 0,
        }
    }

    pub(super) fn visible_start(&self)->usize{self.scrollback.len().saturating_sub(self.scroll)}
    pub(super) fn point(&self,visible_row:usize,col:usize)->Point{
        Point{row:(self.visible_start()+visible_row.min(self.rows-1)).min(self.scrollback.len()+self.rows-1),col:col.min(self.cols)}
    }
    pub(super) fn select_start(&mut self,row:usize,col:usize){
        let at=self.point(row,col);self.selection=Some(Selection{anchor:at,end:at});self.selecting=true;self.dirty=true;
    }
    pub(super) fn select_extend(&mut self,row:usize,col:usize){
        let at=self.point(row,col);
        if self.selecting{if let Some(s)=&mut self.selection{s.end=at;self.dirty=true;}}
    }
    pub(super) fn select_end(&mut self){self.selecting=false;}
    pub(super) fn cancel_selection(&mut self){self.selection=None;self.selecting=false;self.dirty=true;}
    pub(super) fn selected(&self,row:usize,col:usize)->bool{
        self.selection.is_some_and(|s|{let (a,b)=s.range();a<=Point{row,col}&&Point{row,col}<b})
    }
    pub(super) fn selected_text(&self)->Option<String>{
        let (a,b)=self.selection?.range();if a==b{return None;}
        let sb=self.scrollback.len();let mut out=String::new();
        for row in a.row..=b.row {
            let cells=if row<sb{self.scrollback.get(row)}else{self.grid.get(row-sb)}?;
            let start=if row==a.row{a.col.min(cells.len())}else{0};
            let end=if row==b.row{b.col.min(cells.len())}else{cells.len()};
            let wrapped=if row<sb{self.scroll_wrap[row]}else{self.grid_wrap[row-sb]};
            let text:String=cells[start..end].iter().map(|c|c.ch).collect();
            if row<b.row&&wrapped{out.push_str(&text);}else{out.push_str(text.trim_end_matches(' '));}
            if row<b.row&&!wrapped{out.push('\n');}
        }
        Some(out)
    }
    fn trim_selection(&mut self,rows:usize){
        if let Some(selection)=&mut self.selection {
            if selection.anchor.row<rows||selection.end.row<rows{self.cancel_selection();}
            else{selection.anchor.row-=rows;selection.end.row-=rows;}
        }
    }

    /// Move the view up (`up = true`) or down through the scrollback by `rows`,
    /// clamped so it never scrolls past the top or below the live bottom.
    pub(super) fn scroll_by(&mut self, rows: usize, up: bool) {
        let max = self.scrollback.len();
        self.scroll = if up {
            self.scroll.saturating_add(rows).min(max)
        } else {
            self.scroll.saturating_sub(rows)
        };
        self.dirty = true;
    }

    /// Native wheel deltas are negative upward and positive downward.
    pub(super) fn scroll_wheel(&mut self, delta: i32, rows_per_notch: usize) {
        if delta != 0 {
            self.scroll_by((delta.unsigned_abs() as usize).saturating_mul(rows_per_notch), delta < 0);
        }
    }

    /// Push the top grid row into scrollback and open a fresh blank bottom row —
    /// the screen scrolled up by one line.
    pub(super) fn scroll_up(&mut self) {
        let row = self.grid.remove(0);
        self.scrollback.push(row);
        self.scroll_wrap.push(self.grid_wrap.remove(0));
        if self.scrollback.len() > SCROLL_CAP {
            let excess = self.scrollback.len() - SCROLL_CAP;
            self.scrollback.drain(0..excess);
            self.scroll_wrap.drain(0..excess);
            self.trim_selection(excess);
        }
        self.grid.push(blank_row(self.cols, self.palette.fg));
        self.grid_wrap.push(false);
        self.scroll = 0; // new output snaps the view back to the live bottom
    }

    /// Advance to the next row (scrolling at the bottom) and return to column 0.
    /// `flush` emits the logical echo line for a real `\n`; a column wrap passes
    /// false so the wrapped text stays one logical `[term]` echo.
    pub(super) fn newline(&mut self, flush: bool) {
        if flush {
            self.flush_echo();
        }
        self.grid_wrap[self.crow] = !flush;
        self.crow += 1;
        if self.crow >= self.rows {
            self.scroll_up();
            self.crow = self.rows - 1;
        }
        self.ccol = 0;
    }

    /// Emit the accumulated logical line to fd 1 as `[term] <line>` — the headless
    /// verification channel (our own stdout, not the pty) — then reset it.
    pub(super) fn flush_echo(&mut self) {
        #[cfg(not(test))]
        {
            let _ = dunit::io::Write::write_all(&mut dunit::io::stdout(), b"[term] ");
            let _ = dunit::io::Write::write_all(&mut dunit::io::stdout(), self.echo.as_bytes());
            let _ = dunit::io::Write::write_all(&mut dunit::io::stdout(), b"\n");
        }
        self.echo.clear();
    }

    /// Write one printable char at the cursor, wrapping at the right edge, and
    /// advance. The char also appends to the logical echo line.
    pub(super) fn put_char(&mut self, c: char) {
        if self.ccol >= self.cols {
            self.newline(false);
        }
        self.grid[self.crow][self.ccol] = Cell {
            ch: c,
            fg: self.fg,
            bg: self.bg,
            attrs: self.attrs,
        };
        self.ccol += 1;
        self.echo.push(c);
    }

    /// Move the cursor one column left (non-destructive, like a raw terminal BS).
    pub(super) fn backspace(&mut self) {
        if self.ccol > 0 {
            self.ccol -= 1;
        }
        self.echo.pop();
    }

    /// Advance to the next 8-column tab stop (clamped inside the row).
    pub(super) fn tab(&mut self) {
        let next = ((self.ccol / 8) + 1) * 8;
        self.ccol = next.min(self.cols.saturating_sub(1));
        self.echo.push(' ');
    }

    /// Blank every cell and home the cursor (`\f` / `ESC[2J`+`ESC[H`).
    pub(super) fn clear_screen(&mut self) {
        self.cancel_selection();
        let fg = self.palette.fg;
        for row in self.grid.iter_mut() {
            for cell in row.iter_mut() {
                *cell = Cell::blank(fg);
            }
        }
        self.grid_wrap.fill(false);
        self.crow = 0;
        self.ccol = 0;
        self.echo.clear();
        self.scroll = 0;
    }

    /// Erase within the cursor row (EL): 0 = to end, 1 = to start, 2 = whole line.
    pub(super) fn erase_line(&mut self, mode: u32) {
        let (a, b) = match mode {
            1 => (0, self.ccol + 1),
            2 => (0, self.cols),
            _ => (self.ccol, self.cols),
        };
        let fg = self.palette.fg;
        let bg = self.bg;
        let end = b.min(self.cols);
        for cell in self.grid[self.crow].iter_mut().take(end).skip(a) {
            *cell = Cell::blank_with_bg(fg, bg);
        }
        if end == self.cols {
            self.grid_wrap[self.crow] = false;
        }
        if mode == 2 {
            self.echo.clear();
        }
    }
    // ERASE_RESIZE_MARKER

    /// Erase within the screen (ED): 0 = cursor→end, 1 = start→cursor, 2 = all.
    pub(super) fn erase_display(&mut self, mode: u32) {
        let fg = self.palette.fg;
        match mode {
            2 => {
                self.grid_wrap.fill(false);
                for row in self.grid.iter_mut() {
                    for cell in row.iter_mut() {
                        *cell = Cell::blank(fg);
                    }
                }
                self.echo.clear();
            }
            1 => {
                for r in 0..self.crow {
                    self.grid_wrap[r] = false;
                    for cell in self.grid[r].iter_mut() {
                        *cell = Cell::blank(fg);
                    }
                }
                self.erase_line(1);
            }
            _ => {
                self.erase_line(0);
                for r in (self.crow + 1)..self.rows {
                    self.grid_wrap[r] = false;
                    for cell in self.grid[r].iter_mut() {
                        *cell = Cell::blank(fg);
                    }
                }
            }
        }
    }

    /// Reflow only soft-wrapped physical rows into logical lines. Explicit
    /// newlines remain boundaries; foreground colors and cursor offset survive.
    pub(super) fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cancel_selection();
        let cursor_row = self.scrollback.len() + self.crow;
        let mut source = self.scrollback.clone();
        source.extend(self.grid.iter().cloned());
        let mut wraps = self.scroll_wrap.clone();
        wraps.extend(self.grid_wrap.iter().copied());
        // Empty screen padding below the cursor is not terminal output.
        while source.len() > cursor_row + 1
            && source.last().unwrap().iter().all(|cell| cell.ch == ' ')
        {
            source.pop();
            wraps.pop();
        }
        let mut output = Vec::new();
        let mut output_wrap = Vec::new();
        let mut cursor = (0usize, 0usize);
        let mut index = 0;
        while index < source.len() {
            let mut logical = Vec::new();
            let mut cursor_offset = None;
            loop {
                let row = &source[index];
                let mut length = if wraps[index] {
                    row.len()
                } else {
                    row.iter()
                        .rposition(|cell| cell.ch != ' ')
                        .map_or(0, |last| last + 1)
                };
                if index == cursor_row {
                    cursor_offset = Some(logical.len() + self.ccol);
                    length = length.max(self.ccol);
                }
                logical.extend_from_slice(&row[..length.min(row.len())]);
                let continued = wraps[index];
                index += 1;
                if !continued || index == source.len() {
                    break;
                }
            }
            let count = ((logical.len() + cols - 1) / cols).max(1);
            if let Some(offset) = cursor_offset {
                // Preserve the pending-wrap cursor at the end of a full row.
                let local_row = (offset / cols).min(count - 1);
                cursor = (output.len() + local_row, offset - local_row * cols);
            }
            for part in 0..count {
                let mut row = blank_row(cols, self.palette.fg);
                let start = part * cols;
                let end = (start + cols).min(logical.len());
                if start < end {
                    row[..end - start].copy_from_slice(&logical[start..end]);
                }
                output.push(row);
                output_wrap.push(part + 1 < count);
            }
        }
        while output.len() < rows {
            output.push(blank_row(cols, self.palette.fg));
            output_wrap.push(false);
        }
        let split = output.len().saturating_sub(rows);
        self.grid = output.split_off(split);
        self.grid_wrap = output_wrap.split_off(split);
        self.scrollback = output;
        self.scroll_wrap = output_wrap;
        self.crow = cursor.0.saturating_sub(split).min(rows - 1);
        self.ccol = cursor.1.min(cols);
        if self.scrollback.len() > SCROLL_CAP {
            let excess = self.scrollback.len() - SCROLL_CAP;
            self.scrollback.drain(0..excess);
            self.scroll_wrap.drain(0..excess);
            self.trim_selection(excess);
        }
        self.cols = cols;
        self.rows = rows;
        self.scroll = self.scroll.min(self.scrollback.len());
        self.dirty = true;
    }

    /// Apply collected SGR (`ESC [ … m`) params to foreground, background, and attributes.
    pub(super) fn apply_sgr(&mut self) {
        let n = if self.nparams == 0 { 1 } else { self.nparams };
        let mut i = 0usize;
        while i < n {
            let p = if self.nparams == 0 { 0 } else { self.params[i] };
            match p {
                0 => {
                    self.fg = self.palette.fg;
                    self.bg = 0;
                    self.attrs = 0;
                }
                1 => self.attrs |= ATTR_BOLD,
                2 => self.attrs |= ATTR_DIM,
                4 => self.attrs |= ATTR_UNDERLINE,
                7 => self.attrs |= ATTR_INVERSE,
                22 => self.attrs &= !(ATTR_BOLD | ATTR_DIM),
                24 => self.attrs &= !ATTR_UNDERLINE,
                27 => self.attrs &= !ATTR_INVERSE,
                30..=37 => {
                    let slot = (p - 30) as usize;
                    self.fg = if (self.attrs & ATTR_BOLD) != 0 {
                        self.palette.ansi_bright[slot]
                    } else {
                        self.palette.ansi[slot]
                    };
                }
                38 => {
                    if i + 1 < n {
                        let mode = self.params[i + 1];
                        if mode == 2 && i + 4 < n {
                            let r = self.params[i + 2] & 0xFF;
                            let g = self.params[i + 3] & 0xFF;
                            let b = self.params[i + 4] & 0xFF;
                            self.fg = 0xFF00_0000 | (r << 16) | (g << 8) | b;
                            i += 4;
                        } else if mode == 5 && i + 2 < n {
                            self.fg = xterm256(&self.palette, self.params[i + 2]);
                            i += 2;
                        }
                    }
                }
                39 => self.fg = self.palette.fg,
                40..=47 => self.bg = self.palette.ansi[(p - 40) as usize] | 0xFF00_0000,
                48 => {
                    if i + 1 < n {
                        let mode = self.params[i + 1];
                        if mode == 2 && i + 4 < n {
                            let r = self.params[i + 2] & 0xFF;
                            let g = self.params[i + 3] & 0xFF;
                            let b = self.params[i + 4] & 0xFF;
                            self.bg = 0xFF00_0000 | (r << 16) | (g << 8) | b;
                            i += 4;
                        } else if mode == 5 && i + 2 < n {
                            self.bg = xterm256(&self.palette, self.params[i + 2]);
                            i += 2;
                        }
                    }
                }
                49 => self.bg = 0,
                90..=97 => self.fg = self.palette.ansi_bright[(p - 90) as usize],
                100..=107 => self.bg = self.palette.ansi_bright[(p - 100) as usize] | 0xFF00_0000,
                _ => {}
            }
            i += 1;
        }
    }

    pub(super) fn push_param(&mut self) {
        if self.nparams < self.params.len() {
            self.params[self.nparams] = self.param;
            self.nparams += 1;
        }
        self.param = 0;
        self.has_param = false;
    }

    /// Parameter `i` with a fallback when absent OR zero (CSI convention: an
    /// omitted/zero cursor-motion parameter means 1).
    pub(super) fn param_or(&self, i: usize, default: u32) -> u32 {
        let v = if i < self.nparams { self.params[i] } else { 0 };
        if v == 0 {
            default
        } else {
            v
        }
    }

    /// Raw parameter `i` (0 when absent) — for ED/EL, where 0 is a real mode.
    pub(super) fn param_raw(&self, i: usize) -> u32 {
        if i < self.nparams {
            self.params[i]
        } else {
            0
        }
    }

    /// Handle a CSI final byte: SGR color, cursor motion, and erase.
    pub(super) fn dispatch_csi(&mut self, final_byte: u8) {
        match final_byte {
            b'm' => self.apply_sgr(),
            b'A' => self.crow = self.crow.saturating_sub(self.param_or(0, 1) as usize),
            b'B' => self.crow = (self.crow + self.param_or(0, 1) as usize).min(self.rows - 1),
            b'C' => self.ccol = (self.ccol + self.param_or(0, 1) as usize).min(self.cols - 1),
            b'D' => self.ccol = self.ccol.saturating_sub(self.param_or(0, 1) as usize),
            b'G' => self.ccol = (self.param_or(0, 1) as usize - 1).min(self.cols - 1),
            b'd' => self.crow = (self.param_or(0, 1) as usize - 1).min(self.rows - 1),
            b'H' | b'f' => {
                self.crow = (self.param_or(0, 1) as usize - 1).min(self.rows - 1);
                self.ccol = (self.param_or(1, 1) as usize - 1).min(self.cols - 1);
            }
            b'J' => self.erase_display(self.param_raw(0)),
            b'K' => self.erase_line(self.param_raw(0)),
            _ => {} // other finals / private modes: consumed
        }
    }

    /// Feed one chunk of raw shell stdout through the ESC state machine.
    pub(super) fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.esc {
                Esc::Esc => {
                    self.esc = match b {
                        b'[' => Esc::Csi,
                        b']' => Esc::Osc,
                        _ => Esc::Normal,
                    };
                    continue;
                }
                Esc::Csi => {
                    match b {
                        b'0'..=b'9' => {
                            self.param = self
                                .param
                                .saturating_mul(10)
                                .saturating_add((b - b'0') as u32);
                            self.has_param = true;
                        }
                        b';' => self.push_param(),
                        0x40..=0x7e => {
                            if self.has_param || self.nparams > 0 {
                                self.push_param();
                            }
                            self.dispatch_csi(b);
                            self.nparams = 0;
                            self.param = 0;
                            self.has_param = false;
                            self.esc = Esc::Normal;
                        }
                        _ => {} // '?' prefix and intermediates: keep scanning
                    }
                    continue;
                }
                Esc::Osc => {
                    self.esc = match b {
                        0x07 => Esc::Normal,
                        0x1b => Esc::OscEsc,
                        _ => Esc::Osc,
                    };
                    continue;
                }
                Esc::OscEsc => {
                    self.esc = if b == b'\\' { Esc::Normal } else { Esc::Osc };
                    continue;
                }
                Esc::Normal => {}
            }
            match b {
                0x1b => {
                    self.utf8_len = 0;
                    self.esc = Esc::Esc;
                }
                b'\n' => {
                    self.utf8_len = 0;
                    self.newline(true);
                }
                b'\r' => {
                    self.utf8_len = 0;
                    self.ccol = 0;
                    self.echo.clear();
                }
                b'\t' => {
                    self.utf8_len = 0;
                    self.tab();
                }
                0x0c => {
                    self.utf8_len = 0;
                    self.clear_screen();
                }
                0x08 => {
                    self.utf8_len = 0;
                    self.backspace();
                }
                _ => {
                    if self.utf8_len == 0 {
                        if b < 0x80 {
                            if (0x20..=0x7e).contains(&b) {
                                self.put_char(b as char);
                            }
                        } else if (b & 0xE0) == 0xC0 || (b & 0xF0) == 0xE0 || (b & 0xF8) == 0xF0 {
                            self.utf8_buf[0] = b;
                            self.utf8_len = 1;
                        }
                    } else if (b & 0xC0) == 0x80 && self.utf8_len < 4 {
                        self.utf8_buf[self.utf8_len] = b;
                        self.utf8_len += 1;
                        if let Ok(s) = core::str::from_utf8(&self.utf8_buf[..self.utf8_len]) {
                            if let Some(ch) = s.chars().next() {
                                self.put_char(ch);
                            }
                            self.utf8_len = 0;
                        }
                    } else {
                        self.utf8_len = 0;
                        if (0x20..=0x7e).contains(&b) {
                            self.put_char(b as char);
                        } else if (b & 0xE0) == 0xC0 || (b & 0xF0) == 0xE0 || (b & 0xF8) == 0xF0 {
                            self.utf8_buf[0] = b;
                            self.utf8_len = 1;
                        }
                    }
                }
            }
        }
        if !bytes.is_empty() {
            self.dirty = true;
        }
    }
}
