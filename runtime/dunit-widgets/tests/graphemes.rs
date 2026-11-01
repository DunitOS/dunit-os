use dunit_widgets::text_input::{EditCommand as E, TextEditor};
fn apply(e: &mut TextEditor, c: E) -> bool {
    e.apply(c, 1000, false)
}
#[test]
fn navigation_deletion_and_undo_keep_combining_emoji_and_flags_whole() {
    for cluster in ["e\u{301}", "👨‍👩‍👧‍👦", "🇷🇺", "👍🏽", "क्‍ष"]
    {
        let mut e = TextEditor::new(format!("a{cluster}b"));
        e.place(1, false);
        assert!(apply(
            &mut e,
            E::Right {
                extend: true,
                word: false
            }
        ));
        assert_eq!(&e.text()[e.selection()], cluster);
        apply(&mut e, E::Backspace { word: false });
        assert_eq!(e.text(), "ab");
        apply(&mut e, E::Undo);
        assert_eq!(e.text(), format!("a{cluster}b"));
        assert_eq!(&e.text()[e.selection()], cluster);
        apply(
            &mut e,
            E::Left {
                extend: false,
                word: false,
            },
        );
        assert_eq!(e.cursor(), 1);
        apply(&mut e, E::Delete { word: false });
        assert_eq!(e.text(), "ab");
    }
}
#[test]
fn arbitrary_hit_offsets_snap_to_cluster_start() {
    let mut e = TextEditor::new("ae\u{301}👩‍💻b");
    for i in 2..4 {
        e.place(i, false);
        assert_eq!(e.cursor(), 1);
    }
    for i in 5..15 {
        e.place(i, false);
        assert_eq!(e.cursor(), 4);
    }
}
#[test]
fn vertical_movement_preserves_grapheme_column() {
    let mut e = TextEditor::new_multiline("e\u{301}👍🏽x\nab\nc🇷🇺z");
    e.place("e\u{301}👍🏽".len(), false);
    apply(&mut e, E::Down { extend: false });
    assert_eq!(e.cursor(), "e\u{301}👍🏽x\nab".len());
    apply(&mut e, E::Down { extend: false });
    assert_eq!(e.cursor(), "e\u{301}👍🏽x\nab\nc🇷🇺".len());
}
#[test]
fn joining_clusters_never_leaves_caret_inside_following_cluster() {
    let mut e = TextEditor::new("👩💻");
    e.place("👩".len(), false);
    apply(&mut e, E::Insert('\u{200d}'));
    assert_eq!(e.text(), "👩‍💻");
    assert_eq!(e.cursor(), e.text().len());
    apply(&mut e, E::Backspace { word: false });
    assert_eq!(e.text(), "");
    apply(&mut e, E::Undo);
    assert_eq!(e.text(), "👩‍💻");
}
#[test]
fn word_operations_do_not_split_space_combining_cluster() {
    let mut e = TextEditor::new("x \u{301}y");
    e.select_word(1);
    assert_eq!(&e.text()[e.selection()], " \u{301}");
    apply(&mut e, E::Delete { word: true });
    assert_eq!(e.text(), "xy");
}
