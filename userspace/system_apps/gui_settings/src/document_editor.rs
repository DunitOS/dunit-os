//! Source-preserving draft editor. Validation and persistence belong to the
//! shared dwm_settings document API, never to a second graphical schema.
extern crate alloc;
use alloc::{string::String, vec::Vec};

pub const MAX_BYTES: usize = 64 * 1024;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Panels,
    Typography,
    Widgets,
    Bindings,
    Rules,
    Scripts,
}
impl Section {
    fn includes(self, table: &str, line: &str) -> bool {
        match self {
            Self::Panels => table.starts_with("panels."),
            Self::Widgets => table.starts_with("components."),
            Self::Bindings => table.starts_with("bindings."),
            Self::Rules => table.starts_with("rules."),
            Self::Scripts => table == "scripts",
            Self::Typography => {
                (table == "desktop" && (line.starts_with("font ") || line.starts_with("font=")))
                    || (table == "layout" && line.contains("font_px"))
            }
        }
    }
    pub fn table_prefix(self) -> &'static str {
        match self {
            Self::Panels => "panels",
            Self::Widgets => "components",
            Self::Bindings => "bindings",
            Self::Rules => "rules",
            Self::Scripts => "scripts",
            Self::Typography => "layout",
        }
    }
}

pub struct Editor {
    pub original: String,
    pub lines: Vec<String>,
    pub selected: Option<usize>,
    pub cursor: usize,
    pub dirty: bool,
}
impl Editor {
    pub fn new(text: String) -> Self {
        Self {
            lines: text.lines().map(String::from).collect(),
            original: text,
            selected: None,
            cursor: 0,
            dirty: false,
        }
    }
    pub fn text(&self) -> String {
        self.lines.join("\n") + "\n"
    }
    pub fn rows(&self, section: Section) -> Vec<usize> {
        let mut table = "";
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(index, raw)| {
                let line = raw.trim();
                let heading = line.strip_prefix('[').and_then(|s| s.strip_suffix(']'));
                if let Some(name) = heading {
                    table = name;
                }
                if (section.includes(table, line) && !line.is_empty())
                    || self.selected == Some(index)
                {
                    Some(index)
                } else {
                    None
                }
            })
            .collect()
    }
    pub fn select(&mut self, index: usize) {
        if let Some(line) = self.lines.get(index) {
            self.selected = Some(index);
            self.cursor = line.len();
        }
    }
    pub fn insert_line(&mut self, section: Section) {
        if self.text().len() + 128 > MAX_BYTES {
            return;
        }
        let last = self.selected.or_else(|| self.rows(section).last().copied());
        let index = last.map(|n| n + 1).unwrap_or(self.lines.len());
        let line = if last.is_none() {
            alloc::format!("[{}.new_item]", section.table_prefix())
        } else {
            String::from("key = \"value\"")
        };
        self.lines.insert(index, line);
        self.select(index);
        self.dirty = true;
    }
    pub fn new_table(&mut self, section: Section) {
        if self.text().len() + 128 > MAX_BYTES {
            return;
        }
        let index = self.lines.len();
        self.lines
            .push(alloc::format!("[{}.new_item]", section.table_prefix()));
        self.select(index);
        self.dirty = true;
    }
    pub fn remove_line(&mut self) {
        if let Some(index) = self.selected.take() {
            self.lines.remove(index);
            self.cursor = 0;
            self.dirty = true;
        }
    }
    /// Keyboard input uses the compositor's existing cooked ASCII protocol.
    /// Cursor positions remain UTF-8 boundaries for pre-existing Unicode text.
    pub fn key(&mut self, scancode: u8, ascii: u8) {
        let Some(index) = self.selected else {
            return;
        };
        let bytes = self.lines.iter().map(|s| s.len() + 1).sum::<usize>();
        let line = &mut self.lines[index];
        let previous = |s: &str, cursor: usize| {
            s[..cursor]
                .char_indices()
                .next_back()
                .map(|p| p.0)
                .unwrap_or(0)
        };
        let next = |s: &str, cursor: usize| {
            s[cursor..]
                .chars()
                .next()
                .map(|c| cursor + c.len_utf8())
                .unwrap_or(cursor)
        };
        match scancode {
            0x4b => self.cursor = previous(line, self.cursor),
            0x4d => self.cursor = next(line, self.cursor),
            0x47 => self.cursor = 0,
            0x4f => self.cursor = line.len(),
            0x0e => {
                if self.cursor > 0 {
                    let start = previous(line, self.cursor);
                    line.replace_range(start..self.cursor, "");
                    self.cursor = start;
                    self.dirty = true;
                }
            }
            0x53 => {
                if self.cursor < line.len() {
                    line.replace_range(self.cursor..next(line, self.cursor), "");
                    self.dirty = true;
                }
            }
            _ => {
                if (32..=126).contains(&ascii) && bytes < MAX_BYTES {
                    line.insert(self.cursor, ascii as char);
                    self.cursor += 1;
                    self.dirty = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_preserves_other_tables_and_unicode_boundaries() {
        let mut e = Editor::new(
            "# header\n[panels.main]\nsize = 32\n[components.clock]\nlabel = \"часы\"\n".into(),
        );
        assert_eq!(e.rows(Section::Panels), alloc::vec![1, 2]);
        assert_eq!(e.rows(Section::Widgets), alloc::vec![3, 4]);
        e.select(2);
        e.key(0x0e, 0);
        e.key(0, b'8');
        assert!(e.text().contains("size = 38\n[components.clock]"));
        e.select(4);
        e.key(0x4b, 0);
        e.key(0x0e, 0);
        assert!(e.text().contains("label = \"час\""));
        assert!(e.dirty);
    }
    #[test]
    fn insertion_and_removal_preserve_source_order() {
        let mut e = Editor::new("[scripts]\nenabled = true\n".into());
        e.select(1);
        e.insert_line(Section::Scripts);
        assert_eq!(e.selected, Some(2));
        e.remove_line();
        assert_eq!(e.text(), e.original);
        assert_eq!(e.rows(Section::Scripts), alloc::vec![0, 1]);
    }
    #[test]
    fn incomplete_headers_stay_editable() {
        let mut e = Editor::new("[panels.main]\nsize = 32\n".into());
        e.select(0);
        e.key(0x0e, 0);
        assert_eq!(e.lines[0], "[panels.main");
        assert!(e.rows(Section::Panels).contains(&0));
        e.key(0, b']');
        assert_eq!(e.rows(Section::Panels), alloc::vec![0, 1]);
    }
}
