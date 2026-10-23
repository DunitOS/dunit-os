//! Host regression for the actual terminal line buffer used by read(0, ...).
#[path = "../src/process/terminal_stdin.rs"]
mod terminal_stdin;

use terminal_stdin::TerminalStdin;

#[test]
fn byte_reads_preserve_newline_and_readiness() {
    let mut stdin = TerminalStdin::new();
    let mut byte = [0];
    assert_eq!(stdin.take(&mut byte), None);
    stdin.provide(b"1\n");
    assert_eq!(stdin.take(&mut byte), Some(1));
    assert_eq!(byte, [b'1']);
    assert!(stdin.is_ready());
    assert_eq!(stdin.take(&mut byte), Some(1));
    assert_eq!(byte, [b'\n']);
    assert!(!stdin.is_ready());
    assert_eq!(stdin.take(&mut byte), None);
}

#[test]
fn arbitrary_short_reads_preserve_entire_line() {
    let mut stdin = TerminalStdin::new();
    let line = [b'x'; 256];
    stdin.provide(&line);
    assert_eq!(stdin.take(&mut []), Some(0));
    assert!(stdin.is_ready());
    let mut actual = Vec::new();
    let mut chunk = [0; 17];
    while let Some(read) = stdin.take(&mut chunk) {
        actual.extend_from_slice(&chunk[..read]);
    }
    assert_eq!(actual, line);
    assert!(!stdin.is_ready());
}

#[test]
fn foreground_reset_discards_remainder_and_accepts_next_line() {
    let mut stdin = TerminalStdin::new();
    stdin.provide(b"old\n");
    assert_eq!(stdin.take(&mut [0]), Some(1));
    stdin.clear();
    let mut out = [0; 16];
    assert_eq!(stdin.take(&mut out), None);
    stdin.provide(b"new\n");
    assert_eq!(stdin.take(&mut out), Some(4));
    assert_eq!(&out[..4], b"new\n");
    assert_eq!(stdin.take(&mut out), None);
}
