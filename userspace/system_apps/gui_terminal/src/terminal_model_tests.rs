//! Host entry point: rustc --edition=2021 --test terminal_model_tests.rs.
extern crate alloc;
#[path = "terminal_model.rs"]
mod terminal_model;
use terminal_model::{Palette, Term};
fn term(cols: usize, rows: usize) -> Term {
    Term::new(
        Palette {
            fg: 7,
            ansi: [1; 8],
            ansi_bright: [2; 8],
        },
        cols,
        rows,
        8,
    )
}
fn line(term: &Term, row: usize) -> String {
    term.grid[row].iter().map(|cell| cell.ch).collect()
}
#[test]
fn wraps_at_grid_boundary_and_preserves_logical_line() {
    let mut t = term(4, 3);
    t.feed(b"abcdef");
    assert_eq!(line(&t, 0), "abcd");
    assert_eq!(line(&t, 1), "ef  ");
    assert_eq!((t.crow, t.ccol), (1, 2));
    t.resize(8, 3);
    assert_eq!(line(&t, 0), "abcdef  ");
    assert_eq!((t.crow, t.ccol), (0, 6));
}
#[test]
fn explicit_newlines_never_merge_on_reflow() {
    let mut t = term(4, 3);
    t.feed(b"ab\ncd");
    t.resize(8, 3);
    assert_eq!(line(&t, 0), "ab      ");
    assert_eq!(line(&t, 1), "cd      ");
}
#[test]
fn narrow_then_wide_retains_text_colors_and_pending_wrap() {
    let mut t = term(8, 4);
    t.feed(b"ab\x1b[31mcdefgh");
    t.resize(3, 4);
    assert_eq!(line(&t, 0), "abc");
    assert_eq!(line(&t, 2), "gh ");
    assert_eq!((t.crow, t.ccol), (2, 2));
    t.resize(8, 4);
    assert_eq!(line(&t, 0), "abcdefgh");
    assert_eq!(t.grid[0][1].fg, 7);
    assert_eq!(t.grid[0][2].fg, 1);
    assert_eq!((t.crow, t.ccol), (0, 8));
    t.feed(b"i");
    assert_eq!(line(&t, 1), "i       ");
}
#[test]
fn reflows_history_and_keeps_cursor_visible() {
    let mut t = term(4, 2);
    t.feed(b"abcdefghijkl");
    assert_eq!(t.scrollback.len(), 1);
    t.resize(12, 2);
    assert!(t.scrollback.is_empty());
    assert_eq!(line(&t, 0), "abcdefghijkl");
    assert_eq!((t.crow, t.ccol), (0, 12));
}
#[test]
fn resize_candidate_does_not_change_live_grid_on_failure() {
    let mut live = term(8, 3);
    live.feed(b"abcdef");
    let mut candidate = live.clone();
    candidate.resize(3, 2);
    drop(candidate); // failed surface preparation: candidate is discarded
    assert_eq!(line(&live, 0), "abcdef  ");
    assert_eq!((live.cols, live.rows, live.crow, live.ccol), (8, 3, 0, 6));
}
#[test]
fn utf8_survives_every_input_chunk_boundary() {
    let bytes = "a▓бz".as_bytes();
    for split in 0..=bytes.len() {
        let mut t = term(8, 2);
        t.feed(&bytes[..split]);
        t.feed(&bytes[split..]);
        assert_eq!(line(&t, 0), "a▓бz    ");
    }
}
#[test]
fn osc_title_never_leaks_into_visible_text() {
    for suffix in [b"\x07".as_slice(), b"\x1b\\".as_slice()] {
        let mut t = term(8, 2);
        for byte in b"\x1b]0;title"
            .iter()
            .chain(suffix.iter())
            .chain(b"ok".iter())
        {
            t.feed(&[*byte]);
        }
        assert_eq!(line(&t, 0), "ok      ");
    }
}
#[test]
fn carriage_return_and_erase_leave_no_previous_suffix() {
    let mut t = term(12, 2);
    t.feed(b"abcdefghij\rshort\x1b[K");
    assert_eq!(line(&t, 0), "short       ");
    t.resize(6, 2);
    t.resize(12, 2);
    assert_eq!(line(&t, 0), "short       ");
}

#[test]
fn fractional_pitch_rounds_up_without_color_run_drift() {
    for (advance, expected) in [(7.1, 8), (7.0, 7), (7.9, 8), (0.0, 1)] {
        assert_eq!(terminal_model::cell_pitch(advance), expected);
    }
    let mut t = term(8, 2);
    assert_eq!(t.cell_w, 8);
    t.feed(b"abcdefghijklmnopqrstuv");
    t.scroll_by(1, true);
    assert_eq!(t.scroll, 1);
    t.scroll_by(1, false);
    assert_eq!(t.scroll, 0);
}

#[test]
fn native_wheel_up_enters_backlog_and_down_returns_to_live_bottom() {
    let mut t = term(4, 2);
    t.feed(b"one\ntwo\nthree\nfour\nfive\nsix\n");
    assert!(t.scrollback.len() > 3);
    t.scroll_wheel(-1, 3);
    assert_eq!(t.scroll, 3);
    t.scroll_wheel(1, 2);
    assert_eq!(t.scroll, 1);
    t.scroll_wheel(0, 3);
    assert_eq!(t.scroll, 1);
    t.scroll_wheel(i32::MIN, usize::MAX);
    assert_eq!(t.scroll, t.scrollback.len());
    t.scroll_wheel(i32::MAX, usize::MAX);
    assert_eq!(t.scroll, 0);
}
