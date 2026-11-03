//! Source-preserving draft editor. Validation and persistence belong to the
//! shared dwm_settings document API, never to a second graphical schema.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use dunit::ui::TextEditor;

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
    pub input: TextEditor,
    pub dirty: bool,
}
impl Editor {
    pub fn new(text: String) -> Self {
        Self {
            lines: text.lines().map(String::from).collect(),
            original: text,
            selected: None,
            cursor: 0,
            input:TextEditor::new(String::new()),
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
            self.input=TextEditor::new(line.clone());
            self.cursor = self.input.cursor();
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
    fn commit_input(&mut self,old:TextEditor)->bool{
        let Some(index)=self.selected else{self.input=old;return false;};
        let bytes=self.lines.iter().map(|line|line.len()+1).sum::<usize>();
        if bytes.saturating_sub(self.lines[index].len()).saturating_add(self.input.text().len())>MAX_BYTES{
            self.input=old;return false;
        }
        let changed=self.input!=old;
        if self.lines[index]!=self.input.text(){self.lines[index]=String::from(self.input.text());self.dirty=true;}
        self.cursor=self.input.cursor();changed
    }
    pub fn input(&mut self,input:&dunit::window::Input)->bool{
        if self.selected.is_none(){return false;}
        let Some(command)=dunit::text_clipboard::edit_command(input,false)else{return false;};
        let old=self.input.clone();self.input.apply(command,MAX_BYTES,false);self.commit_input(old)
    }
    pub fn insert(&mut self,text:&str)->bool{
        if self.selected.is_none(){return false;}
        let old=self.input.clone();self.input.insert_text(text,MAX_BYTES,false);self.commit_input(old)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(editor:&mut Editor,scan:u8,ascii:u8){editor.input(&dunit::window::Input{kind:gui_protocol_v1::control::IN_KEY,x:scan as i32,y:0,value:ascii as u32});}

    #[test]
    fn editing_preserves_other_tables_and_unicode_boundaries() {
        let mut e = Editor::new(
            "# header\n[panels.main]\nsize = 32\n[components.clock]\nlabel = \"часы\"\n".into(),
        );
        assert_eq!(e.rows(Section::Panels), alloc::vec![1, 2]);
        assert_eq!(e.rows(Section::Widgets), alloc::vec![3, 4]);
        e.select(2);
        key(&mut e,0x0e, 0);
        key(&mut e,0, b'8');
        assert!(e.text().contains("size = 38\n[components.clock]"));
        e.select(4);
        key(&mut e,0x4b, 0);
        key(&mut e,0x0e, 0);
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
        key(&mut e,0x0e, 0);
        assert_eq!(e.lines[0], "[panels.main");
        assert!(e.rows(Section::Panels).contains(&0));
        key(&mut e,0, b']');
        assert_eq!(e.rows(Section::Panels), alloc::vec![0, 1]);
    }
    #[test]
    fn source_lines_accept_one_atomic_utf8_paste_and_undo(){
        let mut editor=Editor::new("[scripts]\nlabel = \"old\"\n".into());editor.select(1);
        editor.input.apply(dunit::ui::EditCommand::SelectAll,MAX_BYTES,false);
        assert!(editor.insert("label = \"часы 👩‍💻\""));assert!(editor.text().contains("часы 👩‍💻"));
        let before=editor.text();assert!(!editor.insert(&"x".repeat(MAX_BYTES)));assert_eq!(editor.text(),before);
        assert!(!editor.insert("bad\nsecond line"));assert_eq!(editor.text(),before);
        editor.input(&dunit::window::Input{kind:gui_protocol_v1::control::IN_KEY,x:0x2c,y:libdunit::KEYMOD_CTRL as i32,value:0});
        assert_eq!(editor.lines[1],"label = \"old\"");
    }
    #[test]
    fn every_section_can_add_a_selected_draft_table_without_losing_source(){
        for section in [Section::Panels,Section::Typography,Section::Widgets,Section::Bindings,Section::Rules,Section::Scripts]{
            let mut editor=Editor::new("# preserved\n".into());editor.new_table(section);
            assert_eq!(editor.rows(section).last(),editor.selected.as_ref());assert!(editor.text().starts_with("# preserved\n"));
        }
    }

}
