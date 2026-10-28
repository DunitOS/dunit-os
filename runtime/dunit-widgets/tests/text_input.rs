use dunit_widgets::{EditCommand::*, TextEditor, MAX_INPUT_BYTES};

const LIMIT: usize = usize::MAX;

#[test]
fn utf8_offsets_and_selection_replacement() {
    let mut editor = TextEditor::new("aé界🙂");
    assert_eq!(editor.cursor(), 10);
    assert!(editor.place(8, false));
    assert_eq!(editor.cursor(), 6);
    assert!(editor.place(2, true));
    assert_eq!(editor.selection(), 1..6);
    assert!(editor.apply(Insert('Ж'), LIMIT, false));
    assert_eq!(editor.text(), "aЖ🙂");
    assert_eq!(editor.cursor(), 3);
    assert_eq!(editor.anchor(), 3);
    assert!(editor.apply(Delete { word: false }, LIMIT, false));
    assert_eq!(editor.text(), "aЖ");
    assert!(editor.apply(Backspace { word: false }, LIMIT, false));
    assert_eq!(editor.text(), "a");
}

#[test]
fn navigation_extends_and_collapses_selection() {
    let mut e = TextEditor::new("é界🙂");
    e.apply(
        Left {
            extend: true,
            word: false,
        },
        LIMIT,
        false,
    );
    assert_eq!(e.selection(), 5..9);
    e.apply(
        Left {
            extend: false,
            word: false,
        },
        LIMIT,
        false,
    );
    assert_eq!((e.cursor(), e.anchor()), (5, 5));
    e.apply(Home { extend: true }, LIMIT, false);
    assert_eq!(e.selection(), 0..5);
    e.apply(
        Right {
            extend: false,
            word: false,
        },
        LIMIT,
        false,
    );
    assert_eq!((e.cursor(), e.anchor()), (5, 5));
    e.apply(End { extend: true }, LIMIT, false);
    assert_eq!(e.selection(), 5..9);
    assert!(!e.place(9, true));
    assert!(e.place(usize::MAX, false));
    assert_eq!(e.cursor(), 9);
    assert!(!e.apply(
        Right {
            extend: false,
            word: false
        },
        LIMIT,
        false
    ));
}

#[test]
fn whitespace_word_movement_and_deletion() {
    let mut e = TextEditor::new("one  два\u{2003}界");
    e.apply(Home { extend: false }, LIMIT, false);
    e.apply(
        Right {
            extend: false,
            word: true,
        },
        LIMIT,
        false,
    );
    assert_eq!(e.cursor(), 5);
    e.apply(Delete { word: true }, LIMIT, false);
    assert_eq!(e.text(), "one  界");
    e.apply(End { extend: false }, LIMIT, false);
    e.apply(Backspace { word: true }, LIMIT, false);
    assert_eq!(e.text(), "one  ");
    e.apply(
        Left {
            extend: false,
            word: true,
        },
        LIMIT,
        false,
    );
    assert_eq!(e.cursor(), 0);
    e.apply(Delete { word: true }, LIMIT, false);
    assert_eq!(e.text(), "");
}

#[test]
fn select_word_handles_unicode_whitespace_and_end() {
    let mut e = TextEditor::new("éé\u{2003}界界");
    assert!(e.select_word(2));
    assert_eq!(e.selection(), 0..4);
    assert!(e.select_word(5));
    assert_eq!(e.selection(), 4..7);
    assert!(e.select_word(usize::MAX));
    assert_eq!(e.selection(), 7..13);
    assert!(!e.select_word(13));
    assert!(!TextEditor::new("").select_word(99));
}

#[test]
fn limits_and_control_rejection_preserve_all_state() {
    let mut e = TextEditor::new("界🙂");
    e.place(3, true);
    let saved = e.clone();
    assert!(!e.apply(Insert('a'), 1, false));
    assert_eq!(e, saved);
    assert!(!e.apply(Insert('\n'), LIMIT, false));
    assert_eq!(e, saved);
    assert!(e.apply(Insert('é'), 2, false));
    assert_eq!(e.text(), "界é");
    let mut large = TextEditor::new("a".repeat(MAX_INPUT_BYTES));
    let saved = large.clone();
    assert!(!large.apply(Insert('界'), LIMIT, false));
    assert_eq!(large, saved);
    large.place(MAX_INPUT_BYTES - 1, true);
    assert!(!large.apply(Insert('界'), LIMIT, false));
    assert_eq!(large.text().len(), MAX_INPUT_BYTES);
}

#[test]
fn undo_redo_restore_selection_and_branch_history() {
    let mut e = TextEditor::new("abc");
    e.place(1, true);
    let before = e.clone();
    e.apply(Insert('界'), LIMIT, false);
    assert_eq!(e.text(), "a界");
    assert!(e.apply(Undo, LIMIT, false));
    assert_eq!(e.text(), before.text());
    assert_eq!(e.selection(), before.selection());
    let with_redo = e.clone();
    assert!(!e.apply(Insert('\r'), LIMIT, false));
    assert_eq!(e, with_redo);
    assert!(!e.apply(Redo, 1, false));
    assert_eq!(e, with_redo);
    assert!(e.apply(Redo, LIMIT, false));
    assert_eq!(e.text(), "a界");
    e.apply(Undo, LIMIT, false);
    e.apply(Insert('x'), LIMIT, false);
    assert!(!e.can_redo());
    assert!(!e.apply(Redo, LIMIT, false));
}

#[test]
fn history_is_bounded_at_sixty_four_edits() {
    let mut e = TextEditor::new("");
    for _ in 0..70 {
        assert!(e.apply(Insert('a'), LIMIT, false));
    }
    for _ in 0..64 {
        assert!(e.apply(Undo, LIMIT, false));
    }
    assert_eq!(e.text(), "aaaaaa");
    assert!(!e.can_undo());
    assert!(!e.apply(Undo, LIMIT, false));
    for _ in 0..64 {
        assert!(e.apply(Redo, LIMIT, false));
    }
    assert_eq!(e.text().len(), 70);
    assert!(!e.can_redo());
}

#[test]
fn sync_resets_history_clamps_boundaries_and_rejects_invalid() {
    let mut e = TextEditor::new("abc");
    e.apply(Insert('x'), LIMIT, false);
    let unchanged = e.clone();
    e.sync("abcx");
    assert_eq!(e, unchanged);
    e.sync("bad\nvalue");
    assert_eq!(e, unchanged);
    e.sync(&"a".repeat(MAX_INPUT_BYTES + 1));
    assert_eq!(e, unchanged);
    e.place(2, true);
    e.sync("界");
    assert_eq!((e.cursor(), e.anchor()), (0, 3));
    assert!(!e.can_undo());
    assert!(!e.can_redo());
    for separator in ['\n', '\r', '\t', '\u{85}', '\u{2028}', '\u{2029}'] {
        assert!(!TextEditor::valid_text(&separator.to_string()));
        let before = e.clone();
        assert!(!e.apply(Insert(separator), LIMIT, false));
        assert_eq!(e, before);
    }
    assert!(!TextEditor::valid_text("a\tb"));
    assert!(!TextEditor::valid_text("\u{7f}"));
    assert!(TextEditor::valid_text("é\u{2003}🙂"));
    let mut invalid = TextEditor::new("bad\nvalue");
    assert_eq!(invalid.text(), "bad\nvalue");
    assert!(!invalid.apply(Insert('x'), LIMIT, false));
}

#[test]
fn read_only_allows_selection_but_preserves_edits_and_history() {
    let mut e = TextEditor::new("abc");
    e.apply(Insert('d'), LIMIT, false);
    for command in [
        Insert('x'),
        Backspace { word: false },
        Delete { word: true },
        Undo,
        Redo,
    ] {
        let previous = e.clone();
        assert!(!e.apply(command, LIMIT, true));
        assert_eq!(e, previous);
    }
    assert!(e.apply(SelectAll, LIMIT, true));
    assert_eq!(e.selection(), 0..4);
    assert!(e.apply(Home { extend: false }, LIMIT, true));
    assert_eq!(e.cursor(), 0);
    assert!(e.can_undo());
}

#[test]
fn no_op_edits_do_not_create_history() {
    let mut e = TextEditor::new("a");
    e.select_all();
    assert!(e.apply(Insert('a'), LIMIT, false));
    assert_eq!(e.text(), "a");
    assert_eq!(e.selection(), 1..1);
    assert!(!e.can_undo());
    e.place(0, false);
    assert!(!e.apply(Backspace { word: false }, LIMIT, false));
    e.place(1, false);
    assert!(!e.apply(Delete { word: false }, LIMIT, false));
    assert!(!e.can_undo());
}
