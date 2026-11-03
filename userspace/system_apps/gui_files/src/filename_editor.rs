//! Filename editing uses the same grapheme/history engine and asynchronous
//! clipboard transaction as SDK fields. A revision prevents focus/ABA replies.
use alloc::string::String;
use dunit::ui::{EditCommand, TextEditor};
use dunit::window::Input;
#[derive(Clone)]
pub(super) struct Snapshot {
    editor: TextEditor,
    revision: u64,
}
impl dunit::text_clipboard::Context for Snapshot {
    fn selected_text(&self) -> Option<&str> {
        let range = self.editor.selection();
        if range.is_empty() {
            None
        } else {
            Some(&self.editor.text()[range])
        }
    }
    fn writable(&self) -> bool {
        true
    }
}
impl Snapshot {
    pub fn current(&self, editor: &Editor, revision: u64) -> bool {
        self.revision == revision && self.editor == editor.0
    }
}
pub(super) struct Editor(TextEditor);
impl Editor {
    pub fn new(value: String) -> Self {
        let mut editor = TextEditor::new(value);
        editor.apply(EditCommand::SelectAll, 255, false);
        Self(editor)
    }
    pub fn text(&self) -> &str {
        self.0.text()
    }
    pub fn cursor(&self) -> usize {
        self.0.cursor()
    }
    pub fn selection(&self) -> core::ops::Range<usize> {
        self.0.selection()
    }
    pub fn place(&mut self, index: usize, extend: bool) -> bool {
        self.0.place(index, extend)
    }
    pub fn position(&self, font: &dunit::ui::Font, x: f32) -> usize {
        let mut previous = 0;
        let mut left = 0.0;
        for (index, ch) in self.text().char_indices() {
            let end = index + ch.len_utf8();
            let right = font.layout_line(&self.text()[..end], 13.0).1;
            if x < (left + right) * 0.5 {
                return previous;
            }
            previous = end;
            left = right;
        }
        self.text().len()
    }
    pub fn snapshot(&self, revision: u64) -> Snapshot {
        Snapshot {
            editor: self.0.clone(),
            revision,
        }
    }
    fn valid(&self) -> bool {
        self.text().len() <= 255 && !self.text().contains('/')
    }
    pub fn insert(&mut self, text: &str) -> bool {
        let old = self.0.clone();
        let changed = self.0.insert_text(text, 255, false);
        if !self.valid() {
            self.0 = old;
            return false;
        }
        changed
    }
    pub fn key(&mut self, input: &Input) -> bool {
        let command = dunit::text_clipboard::edit_command(input, false);
        let Some(command) = command else {
            return false;
        };
        let old = self.0.clone();
        let changed = self.0.apply(command, 255, false);
        if !self.valid() {
            self.0 = old;
            return false;
        }
        changed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filename_paste_is_atomic_and_one_undo() {
        let mut editor = Editor::new("old".into());
        assert!(editor.insert("новый 👩‍💻.txt"));
        assert_eq!(editor.text(), "новый 👩‍💻.txt");
        assert!(editor.0.apply(EditCommand::Undo, 255, false));
        assert_eq!(editor.text(), "old");
        let old = editor.0.clone();
        assert!(!editor.insert("bad/path"));
        assert_eq!(editor.0, old);
        assert!(!editor.insert(&"я".repeat(128)));
        assert_eq!(editor.0, old);
    }
    #[test]
    fn pointer_caret_and_extension_use_the_shared_grapheme_editor() {
        let font = dunit::ui::Font::parse(
            include_bytes!("../../../../assets/fonts/DejaVuSans.ttf").to_vec(),
        )
        .unwrap();
        let mut editor = Editor::new("a👩‍💻b".into());
        assert_eq!(editor.position(&font, -10.0), 0);
        assert_eq!(editor.position(&font, 10000.0), editor.text().len());
        assert!(editor.place(0, false));
        let end = editor.position(&font, 10000.0);
        assert!(editor.place(end, true));
        assert_eq!(editor.selection(), 0..editor.text().len());
        assert!(editor.insert("x"));
        assert_eq!(editor.text(), "x");
    }
    #[test]
    fn identical_text_cannot_revalidate_a_stale_focus_revision() {
        let mut editor = Editor::new("name".into());
        let context = editor.snapshot(5);
        assert_eq!(editor.cursor(), 4);
        assert_eq!(editor.selection(), 0..4);
        assert!(context.current(&editor, 5));
        assert!(!context.current(&editor, 6));
        assert!(editor.key(&Input {
            kind: gui_protocol_v1::control::IN_KEY,
            x: 0x4b,
            y: 0,
            value: 0
        }));
        assert_eq!(editor.cursor(), 0);
    }
}
