//! Host regression for the actual terminal line buffer used by read(0, ...).
#[path = "../src/process/terminal_stdin.rs"]
mod terminal_stdin;

use terminal_stdin::TerminalStdin;

#[test]
fn byte_reads_preserve_newline_and_readiness() {
    let mut stdin = TerminalStdin::new();
    let mut byte = [0];
    assert_eq!(stdin.take(1, &mut byte), None);
    stdin.request(1);
    assert!(stdin.provide(1, b"1\n"));
    assert_eq!(stdin.take(1, &mut byte), Some(1));
    assert_eq!(byte, [b'1']);
    assert!(stdin.is_ready(1));
    assert_eq!(stdin.take(1, &mut byte), Some(1));
    assert_eq!(byte, [b'\n']);
    assert!(!stdin.is_ready(1));
    assert_eq!(stdin.take(1, &mut byte), None);
}

#[test]
fn arbitrary_short_reads_preserve_entire_line() {
    let mut stdin = TerminalStdin::new();
    let line = [b'x'; 256];
    stdin.request(1);
    assert!(stdin.provide(1, &line));
    assert_eq!(stdin.take(1, &mut []), Some(0));
    assert!(stdin.is_ready(1));
    let mut actual = Vec::new();
    let mut chunk = [0; 17];
    while let Some(read) = stdin.take(1, &mut chunk) {
        actual.extend_from_slice(&chunk[..read]);
    }
    assert_eq!(actual, line);
    assert!(!stdin.is_ready(1));
}

#[test]
fn foreground_reset_discards_remainder_and_accepts_next_line() {
    let mut stdin = TerminalStdin::new();
    stdin.request(1);
    assert!(stdin.provide(1, b"old\n"));
    assert_eq!(stdin.take(1, &mut [0]), Some(1));
    stdin.clear();
    let mut out = [0; 16];
    assert_eq!(stdin.take(1, &mut out), None);
    stdin.request(1);
    assert!(stdin.provide(1, b"new\n"));
    assert_eq!(stdin.take(1, &mut out), Some(4));
    assert_eq!(&out[..4], b"new\n");
    assert_eq!(stdin.take(1, &mut out), None);
}

#[test]
fn pending_and_partial_lines_cannot_be_stolen_or_overwritten() {
    let mut stdin = TerminalStdin::new();
    stdin.request(1);
    stdin.request(2);
    assert_eq!(stdin.waiting_pid(), Some(1));
    assert!(!stdin.provide(2, b"wrong\n"));
    assert!(stdin.provide(1, b"old\n"));
    assert!(!stdin.provide(1, b"replacement\n"));
    assert_eq!(stdin.take(2, &mut [0; 8]), None);
    assert!(!stdin.is_ready(2));
    assert_eq!(stdin.take(1, &mut [0]), Some(1));
    stdin.request(2);
    assert_eq!(
        stdin.waiting_pid(),
        None,
        "a buffered line needs no further keyboard input"
    );
    let mut remainder = [0; 8];
    assert_eq!(stdin.take(1, &mut remainder), Some(3));
    assert_eq!(&remainder[..3], b"ld\n");
    assert_eq!(stdin.waiting_pid(), None);
    stdin.request(2);
    assert!(stdin.provide(2, b"next\n"));
    stdin.clear();
    stdin.request(3);
    assert!(!stdin.provide(2, b"late\n"));
    assert!(!stdin.is_ready(3));
    assert!(stdin.provide(3, b"current\n"));
}

#[allow(dead_code)]
#[path = "../src/sync.rs"]
mod actual_sync;

#[test]
fn concurrent_supply_and_short_reads_preserve_owner_and_bytes() {
    use actual_sync::SpinLock;
    use std::sync::Arc;
    let stdin = Arc::new(SpinLock::new(TerminalStdin::new()));
    const ROUNDS: usize = 10_000;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for n in 0..ROUNDS {
                let owner = (n % 2 + 1) as u64;
                loop {
                    let mut line = stdin.lock();
                    line.request(owner);
                    if line.provide(owner, &[n as u8, b'\n']) {
                        break;
                    }
                    drop(line);
                    std::thread::yield_now();
                }
            }
        });
        for owner in 1..=2 {
            let stdin = &stdin;
            scope.spawn(move || {
                for n in ((owner - 1) as usize..ROUNDS).step_by(2) {
                    for expected in [n as u8, b'\n'] {
                        loop {
                            let mut line = stdin.lock();
                            let mut byte = [0];
                            if line.take(owner, &mut byte).is_some() {
                                assert_eq!(byte[0], expected);
                                break;
                            }
                            drop(line);
                            std::thread::yield_now();
                        }
                    }
                }
            });
        }
    });
    assert_eq!(stdin.lock().waiting_pid(), None);
}
