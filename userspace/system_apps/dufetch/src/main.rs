#![no_std]
#![no_main]

//! dufetch — a fastfetch-style system summary for Dunit OS, as a standalone
//! userspace program (`/app/dufetch`).
//!
//! Runs in BOTH terminals: kernel framebuffer Terminal Mode (`exec dufetch`,
//! stdout mirrored to serial) and the GUI terminal via `dsh` (`dufetch`, bridged
//! on an inner pty). It writes ONLY ANSI SGR color codes — the terminal's active
//! theme decides what each color looks like, so dufetch renders in whatever
//! palette the terminal is themed with (the whole point of the theme system).
//! Every fact is read through libdunit syscalls; nothing is hardcoded policy.
//!
//! Layout: logo on the left, system info on the right — just like fastfetch.
//! Uses only printable ASCII (0x20..0x7E) so the GUI terminal renders it.

use core::panic::PanicInfo;

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libdunit::{FbInfo, SystemStats};

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::println("dufetch: PANIC");
    libdunit::exit(101)
}

/// The ASCII-art logo, baked into the ELF at build time — not loaded from VFS.
/// Uses only printable ASCII chars so it renders in both terminals.
const LOGO: &str = include_str!("../../../../assets/gui/dufetch_logo.txt");

// Logical ANSI colors (the terminal's theme maps these to real RGB).
const C_RESET: &str = "\x1b[0m";
const C_GREEN: &str = "\x1b[1;32m"; // bold green  — logo ':' parts
const C_CYAN: &str = "\x1b[1;36m"; // bold cyan   — logo '*' parts
const C_BOLD: &str = "\x1b[1m"; // bold white  — title
const C_LABEL: &str = "\x1b[1;36m"; // bold cyan   — info labels
const C_VALUE: &str = "\x1b[97m"; // bright white — info values
const C_DIM: &str = "\x1b[90m"; // dim         — separator/palette
const C_YELLOW: &str = "\x1b[1;33m"; // bold yellow — logo '.' parts

fn out(s: &str) {
    libdunit::write(1, s.as_bytes());
}

/// Visible width of a string (ASCII-only: 1 cell per byte, skip ESC sequences).
fn visible_len(s: &str) -> usize {
    let mut n = 0usize;
    let mut in_esc = false;
    for c in s.bytes() {
        if in_esc {
            if c >= b'A' && c <= b'Z' || c >= b'a' && c <= b'z' {
                in_esc = false;
            }
            continue;
        }
        if c == 0x1b {
            in_esc = true;
            continue;
        }
        n += 1;
    }
    n
}

fn cwd_string() -> String {
    let mut b = [0u8; 256];
    let n = libdunit::getcwd(&mut b);
    if n > 0 {
        String::from(core::str::from_utf8(&b[..n as usize]).unwrap_or("/"))
    } else {
        String::from("/")
    }
}

/// Colorize one logo line: '.' and ':' in yellow, '\'' in green, '*' in cyan,
/// spaces and other chars as-is. Returns the colored string.
fn colorize_logo_line(line: &str) -> String {
    let mut result = String::new();
    let mut last_color: &str = "";
    for ch in line.bytes() {
        let color = match ch {
            b'.' | b':' => C_YELLOW,
            b'\'' => C_GREEN,
            b'*' => C_CYAN,
            _ => C_RESET,
        };
        if color != last_color {
            result.push_str(color);
            last_color = color;
        }
        result.push(ch as char);
    }
    result.push_str(C_RESET);
    result
}

// APPEND_RENDER

fn render() {
    let mut s = SystemStats::default();
    libdunit::get_system_stats(&mut s);
    let mut fb = FbInfo { addr: 0, width: 0, height: 0, pitch: 0 };
    libdunit::get_framebuffer(&mut fb);
    let pid = libdunit::get_pid();
    let cwd = cwd_string();

    // Collect logo lines, skip leading/trailing blank lines
    let mut logo_lines: Vec<&str> = Vec::new();
    let mut started = false;
    for line in LOGO.lines() {
        if !started && line.trim().is_empty() {
            continue;
        }
        started = true;
        logo_lines.push(line);
    }
    // Trim trailing blank lines
    while logo_lines.last().map_or(false, |l| l.trim().is_empty()) {
        logo_lines.pop();
    }

    // Find logo width (max visible length)
    let logo_w: usize = logo_lines.iter().map(|l| l.len()).max().unwrap_or(0);
    let pad = 2usize; // gap between logo and info

    // Build info lines in fastfetch style
    let mut info: Vec<String> = Vec::new();

    // Title: user@host
    info.push(format!("{}root{}{}@{}{}dunit{}", C_BOLD, C_RESET, C_BOLD, C_RESET, C_BOLD, C_RESET));

    // Separator
    let mut sep = String::new();
    sep.push_str(C_DIM);
    for _ in 0..16 { sep.push('-'); }
    sep.push_str(C_RESET);
    info.push(sep);

    // OS
    info.push(format!("{}OS{}: {}Dunit OS x86_64{}", C_LABEL, C_RESET, C_VALUE, C_RESET));

    // Kernel
    info.push(format!("{}Kernel{}: {}1.0.0 Green Tea{}", C_LABEL, C_RESET, C_VALUE, C_RESET));

    // Shell
    info.push(format!("{}Shell{}: {}dsh{}", C_LABEL, C_RESET, C_VALUE, C_RESET));

    // Display
    if fb.width > 0 && fb.height > 0 {
        info.push(format!("{}Display{}: {}{}x{}{}", C_LABEL, C_RESET, C_VALUE, fb.width, fb.height, C_RESET));
    } else {
        info.push(format!("{}Display{}: {}framebuffer{}", C_LABEL, C_RESET, C_VALUE, C_RESET));
    }

    // WM
    info.push(format!("{}WM{}: {}DWM (Dunit Window Manager){}", C_LABEL, C_RESET, C_VALUE, C_RESET));

    // Terminal
    info.push(format!("{}Terminal{}: {}gui_terminal / kernel fb{}", C_LABEL, C_RESET, C_VALUE, C_RESET));

    // CPU (arch info)
    info.push(format!("{}CPU{}: {}x86_64{}", C_LABEL, C_RESET, C_VALUE, C_RESET));

    // Memory
    info.push(format!(
        "{}Memory{}: {}{} KiB / {} KiB{}",
        C_LABEL, C_RESET, C_VALUE,
        s.pmm_used_bytes / 1024, s.pmm_total_bytes / 1024,
        C_RESET
    ));

    // Disk / FS
    info.push(format!("{}Disk{}: {}MemFS over VFS{}", C_LABEL, C_RESET, C_VALUE, C_RESET));

    // Tasks
    info.push(format!(
        "{}Processes{}: {}{} ({} running){}",
        C_LABEL, C_RESET, C_VALUE,
        s.process_total, s.process_running,
        C_RESET
    ));

    // PID
    info.push(format!("{}PID{}: {}{}{}", C_LABEL, C_RESET, C_VALUE, pid, C_RESET));

    // CWD
    info.push(format!("{}CWD{}: {}{}{}", C_LABEL, C_RESET, C_VALUE, cwd, C_RESET));

    // Empty line before color swatches
    info.push(String::new());

    // Color swatches (2 rows of 8 colors each)
    let mut colors1 = String::new();
    for i in 0..8u8 {
        colors1.push_str(&format!("\x1b[4{}m   ", i));
    }
    colors1.push_str(C_RESET);
    info.push(colors1);

    let mut colors2 = String::new();
    for i in 0..8u8 {
        colors2.push_str(&format!("\x1b[10{}m   ", i));
    }
    colors2.push_str(C_RESET);
    info.push(colors2);

    // Output side-by-side: logo left, info right
    let total = if logo_lines.len() > info.len() { logo_lines.len() } else { info.len() };

    out("\n");
    for i in 0..total {
        // Logo part (padded to logo_w)
        if i < logo_lines.len() {
            let line = logo_lines[i];
            let colored = colorize_logo_line(line);
            out(&colored);
            // Pad to logo_w
            let vis = line.len();
            if vis < logo_w {
                for _ in 0..(logo_w - vis) {
                    out(" ");
                }
            }
        } else {
            // Empty logo area
            for _ in 0..logo_w {
                out(" ");
            }
        }

        // Gap
        for _ in 0..pad {
            out(" ");
        }

        // Info part
        if i < info.len() {
            out(&info[i]);
        }

        out("\n");
    }
    out("\n");
}

#[no_mangle]
pub extern "C" fn _start(
    argc: usize,
    argv: *const *const u8,
    envp: *const *const u8,
) -> ! {
    libdunit::init_runtime(argc, argv, envp);
    render();
    libdunit::exit(0)
}
