use crate::{
    compute_line_states, highlight_line, parse_markdown, sha256_hex, CursorPos, DocBlock,
    DocumentBuffer, EditorError, Language, TokenKind,
};

#[test]
fn sha256_matches_standard_vector() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn edit_selection_undo_redo_and_save_hash_roundtrip() {
    let mut doc = DocumentBuffer::from_bytes(
        Some("/tmp/hello.c".into()),
        b"int main(void) {\n    return 0;\n}\n",
    )
    .expect("load valid UTF-8 C file");
    assert!(!doc.is_dirty());
    assert_eq!(doc.line_count(), 4);

    doc.goto_line(2);
    assert_eq!(doc.cursor_pos(), CursorPos { line: 1, col: 0 });
    doc.move_end(false);
    doc.insert_str(" /* ok */").expect("insert comment");
    assert!(doc.is_dirty());
    assert_eq!(doc.line_slice(1), Some("    return 0; /* ok */"));

    // Undo and redo restore exact bytes and cursor positions.
    assert!(doc.undo());
    assert_eq!(doc.line_slice(1), Some("    return 0;"));
    assert!(!doc.is_dirty());
    assert!(doc.redo());
    assert_eq!(doc.line_slice(1), Some("    return 0; /* ok */"));
    assert!(doc.is_dirty());

    // Selection + cut + paste roundtrip.
    doc.goto_line(2);
    doc.move_end(true);
    assert_eq!(doc.selected_text(), "    return 0; /* ok */");
    assert!(doc.cut_selection_or_line());
    assert_eq!(doc.line_slice(1), Some(""));
    doc.paste_clipboard().expect("paste clipboard");
    assert_eq!(doc.line_slice(1), Some("    return 0; /* ok */"));

    let disk_bytes = doc.text().as_bytes().to_vec();
    doc.mark_saved();
    assert!(!doc.is_dirty());
    let reloaded = DocumentBuffer::from_bytes(Some("/tmp/hello.c".into()), &disk_bytes).unwrap();
    assert_eq!(reloaded.content_sha256_hex(), doc.content_sha256_hex());
}

#[test]
fn search_replace_and_case_insensitive_matching() {
    let mut doc = DocumentBuffer::from_bytes(None, b"Alpha beta ALPHA gamma").unwrap();
    let hit = doc.find_next("alpha", false).unwrap();
    assert_eq!(hit, 0..5);
    let hit2 = doc.find_next("alpha", false).unwrap();
    assert_eq!(hit2, 11..16);
    assert!(doc.replace_next("alpha", "Omega", false).unwrap());
    assert_eq!(doc.text(), "Alpha beta Omega gamma");

    let count = doc.replace_all("omega", "Delta", false).unwrap();
    assert_eq!(count, 1);
    assert_eq!(doc.text(), "Alpha beta Delta gamma");

    assert_eq!(
        DocumentBuffer::from_bytes(None, &[0xff, 0xfe]),
        Err(EditorError::InvalidUtf8)
    );
}

#[test]
fn markdown_parser_extracts_blocks_and_links() {
    let md = "# Dunit OS\n\nParagraph with **bold**, `code`, and [link](https://du.net.test).\n\n- item 1\n  - nested\n\n1. first\n\n```c\nint x = 1;\n```\n\n---\n";
    let blocks = parse_markdown(md);
    assert_eq!(
        blocks,
        vec![
            DocBlock::Heading {
                level: 1,
                text: "Dunit OS".into()
            },
            DocBlock::Paragraph(
                "Paragraph with bold, code, and link (https://du.net.test).".into()
            ),
            DocBlock::Bullet {
                indent: 0,
                text: "item 1".into()
            },
            DocBlock::Bullet {
                indent: 1,
                text: "nested".into()
            },
            DocBlock::Numbered {
                index: 1,
                text: "first".into()
            },
            DocBlock::CodeBlock {
                lang: "c".into(),
                text: "int x = 1;".into()
            },
            DocBlock::Rule,
        ]
    );
}

#[test]
fn fuzz_buffer_and_markdown_never_panic() {
    let mut seed = 0x517cc1b7_27220a95u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut doc = DocumentBuffer::new();
    for _ in 0..1000 {
        match next() % 10 {
            0..=2 => {
                let ch = char::from_u32((0x20 + (next() % 95)) as u32).unwrap_or('x');
                let _ = doc.insert_char(ch);
            }
            3 => {
                let _ = doc.insert_char('\n');
            }
            4 => {
                doc.backspace();
            }
            5 => {
                doc.move_left((next() & 1) != 0);
            }
            6 => {
                doc.move_down(1, (next() & 1) != 0);
            }
            7 => {
                doc.undo();
            }
            8 => {
                doc.redo();
            }
            _ => {
                let _ = doc.find_next("a", false);
            }
        }
        let _ = parse_markdown(doc.text());
        assert!(doc.text().len() <= crate::buffer::MAX_DOC_BYTES);
    }
}

#[test]
fn syntax_highlights_rust_c_toml_markdown_with_multiline_comments() {
    let rust_src = "/* outer /* nested */ still comment */\nfn main() -> u32 {\n    let msg = \"hi\"; // note\n    return 42;\n}\n";
    let states = compute_line_states(rust_src, Language::Rust);
    assert_eq!(states.len(), 6);
    assert_eq!(states[0].block_comment_depth, 0);
    assert_eq!(states[1].block_comment_depth, 0);

    let (line0, st1) = highlight_line("/* multi", Language::C, states[0]);
    assert_eq!(st1.block_comment_depth, 1);
    assert_eq!(line0[0].kind, TokenKind::Comment);

    let (line_cont, st2) = highlight_line("  end */ int x = 7;", Language::C, st1);
    assert_eq!(st2.block_comment_depth, 0);
    assert_eq!(line_cont[0].kind, TokenKind::Comment);
    assert!(line_cont.iter().any(|s| s.kind == TokenKind::Type));
    assert!(line_cont.iter().any(|s| s.kind == TokenKind::Number));

    let (rust_fn, _) = highlight_line("pub fn helper(v: usize) -> \"x\"", Language::Rust, Default::default());
    assert!(rust_fn.iter().any(|s| s.kind == TokenKind::Keyword));
    assert!(rust_fn.iter().any(|s| s.kind == TokenKind::Function));
    assert!(rust_fn.iter().any(|s| s.kind == TokenKind::Type));
    assert!(rust_fn.iter().any(|s| s.kind == TokenKind::String));

    let (toml_hdr, _) = highlight_line("[desktop] # cfg", Language::Toml, Default::default());
    assert!(toml_hdr.iter().any(|s| s.kind == TokenKind::Preprocessor));
    assert!(toml_hdr.iter().any(|s| s.kind == TokenKind::Comment));

    assert_eq!(
        Language::from_path_and_text(Some("/tmp/a.rs"), ""),
        Language::Rust
    );
    assert_eq!(
        Language::from_path_and_text(None, "#!/bin/dsh\necho ok\n"),
        Language::Shell
    );
}

#[test]
fn vim_motions_brackets_indent_comments_and_linewise_paste() {
    let mut doc = DocumentBuffer::from_bytes(
        Some("/tmp/demo.rs".into()),
        b"fn demo() {\n    let a = (1 + 2);\n}\n",
    )
    .unwrap();

    // Bracket match '(' -> ')' and '%'
    doc.goto_line(2);
    assert!(doc.find_char_in_line('(', true, false, false));
    assert_eq!(doc.cursor_pos().col, 12);
    let pair = doc.matching_bracket_pair().unwrap();
    assert_eq!(&doc.text()[pair.0..=pair.0], "(");
    assert_eq!(&doc.text()[pair.1..=pair.1], ")");
    assert!(doc.move_matching_bracket(false));
    assert_eq!(doc.cursor_pos().col, 18);

    // Inner word + yank/delete/linewise paste + indent + toggle comment + undo
    assert_eq!(doc.word_under_cursor(), None); // on ')'
    doc.goto_line(2);
    doc.move_word_right(false); // on 'a'
    let w = doc.word_under_cursor().unwrap();
    assert_eq!(w, "let");

    doc.goto_line(2);
    assert!(doc.yank_lines(1));
    assert!(doc.clipboard_is_linewise());
    assert!(doc.paste_vim(false).unwrap());
    assert_eq!(doc.line_count(), 5);

    assert_eq!(doc.toggle_comment_lines(1, 2, "//"), 2);
    assert_eq!(doc.line_slice(1), Some("    // let a = (1 + 2);"));
    assert_eq!(doc.toggle_comment_lines(1, 2, "//"), 2);
    assert_eq!(doc.line_slice(1), Some("    let a = (1 + 2);"));

    assert_eq!(doc.indent_lines(1, 1, true), 1);
    assert_eq!(doc.line_slice(1), Some("        let a = (1 + 2);"));
    assert_eq!(doc.indent_lines(1, 1, false), 1);
    assert_eq!(doc.line_slice(1), Some("    let a = (1 + 2);"));
}
