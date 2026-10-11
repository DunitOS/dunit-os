#![no_std]
#![no_main]

//! PTY master driver for the termios line-discipline smoke test.
//!
//! Owns a pty and spawns `/app/musl_termios` as its slave (its fd 0/1/2 routed
//! to the pty rings). The slave switches itself to canonical + echo through
//! `tcsetattr` and reports readiness; this master then "types" `he<DEL>llo\n`
//! on the master side via `pty_write` and verifies, from the bytes that come
//! back, that the kernel line discipline (1) echoed the input including the
//! erase sequence `\x08 \x08`, and (2) delivered the backspace-edited whole
//! line `hllo` to the slave (the slave reports it as `GOT:hllo`). That is the
//! full tcgetattr/tcsetattr + ICANON/ECHO path end to end. Exits 0 on PASS.

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::println("[TERMIOS] FAIL panic");
    libdunit::exit(101)
}

/// Naive substring search — no alloc, small fixed buffers only.
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if hay.len() < needle.len() {
        return false;
    }
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        if &hay[i..i + needle.len()] == needle {
            return true;
        }
        i += 1;
    }
    false
}

// PLACEHOLDER_START

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let id = libdunit::pty_create();
    if id <= 0 {
        libdunit::println("[TERMIOS] FAIL pty_create");
        libdunit::exit(1);
    }
    let id = id as u32;

    let child = libdunit::pty_spawn("/app/musl_termios", id);
    if child <= 0 {
        libdunit::println("[TERMIOS] FAIL pty_spawn");
        libdunit::exit(1);
    }

    let mut acc = [0u8; 256];
    let mut acc_len = 0usize;
    let mut chunk = [0u8; 128];
    let mut spins = 0u32;
    let mut wrote = false;
    let mut saw_ready = false;
    let mut saw_echo = false;
    let mut saw_line = false;

    // PLACEHOLDER_LOOP
    loop {
        let n = libdunit::pty_read(id, &mut chunk);
        if n == libdunit::EPIPE {
            libdunit::println("[TERMIOS] FAIL slave hung up early");
            libdunit::exit(1);
        }
        if n == libdunit::EAGAIN || n == 0 {
            spins += 1;
            if spins > 200_000 {
                libdunit::println("[TERMIOS] FAIL timeout");
                libdunit::exit(1);
            }
            libdunit::yield_now();
            continue;
        }
        if n < 0 {
            libdunit::println("[TERMIOS] FAIL pty_read error");
            libdunit::exit(1);
        }
        let got = n as usize;
        let space = acc.len() - acc_len;
        let take = if got > space { space } else { got };
        acc[acc_len..acc_len + take].copy_from_slice(&chunk[..take]);
        acc_len += take;
        let view = &acc[..acc_len];
        // PLACEHOLDER_CHECKS
        if !saw_ready && contains(view, b"RDY ") {
            if contains(view, b"tty=0") || contains(view, b"rt=0") {
                libdunit::println("[TERMIOS] FAIL slave isatty/round-trip");
                libdunit::exit(1);
            }
            if contains(view, b"tty=1 rt=1") {
                saw_ready = true;
                libdunit::println(
                    "[TERMIOS] slave isatty + tcgetattr/tcsetattr round-trip ok",
                );
            }
        }

        if saw_ready && !wrote {
            // "Type" he, DEL (0x7f = VERASE), llo, Enter on the master side.
            if libdunit::pty_write(id, b"he\x7fllo\n") <= 0 {
                libdunit::println("[TERMIOS] FAIL pty_write");
                libdunit::exit(1);
            }
            wrote = true;
        }

        if wrote && !saw_echo && contains(view, b"\x08 \x08") {
            saw_echo = true;
            libdunit::println("[TERMIOS] echo + erase bytes observed on master");
        }

        if wrote && !saw_line && contains(view, b"GOT:hllo") {
            saw_line = true;
            libdunit::println("[TERMIOS] canonical line delivered: hllo");
        }

        if saw_echo && saw_line {
            libdunit::pty_close(id);
            libdunit::println("[TERMIOS] PASS");
            libdunit::exit(0);
        }

        if acc_len == acc.len() {
            libdunit::println("[TERMIOS] FAIL buffer full without match");
            libdunit::exit(1);
        }
    }
}

