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
//!
//! Layout: logo on the left, system info on the right — just like fastfetch.
//!
//! Every displayed fact is read through libdunit syscalls at runtime — nothing
//! is hardcoded policy. No fake user@host, no pretend hostname, no invented
//! subsystem names. If the kernel doesn't expose it, dufetch doesn't show it.

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
const LOGO: &str = include_str!("../../../../assets/gui/dufetch_logo.txt");

// Logical ANSI colors (the terminal's theme maps these to real RGB).
const C_RESET: &str = "\x1b[0m";
const C_GREEN: &str = "\x1b[1;32m"; // bold green  — logo '▓' parts
const C_CYAN: &str = "\x1b[1;36m";  // bold cyan   — logo '▒' parts
const C_LABEL: &str = "\x1b[1;36m"; // bold cyan   — info labels
const C_VALUE: &str = "\x1b[97m";   // bright white — info values
const C_DIM: &str = "\x1b[90m";     // dim         — separator/palette

fn out(s: &str) {
    libdunit::write(1, s.as_bytes());
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

/// Colorize one logo line: '▓' in bold green, '▒' in bold cyan,
/// spaces and other chars as-is.
fn colorize_logo_line(line: &str) -> String {
    let mut result = String::new();
    let mut last_color: &str = "";
    for ch in line.chars() {
        let color = match ch {
            '▓' => C_GREEN,
            '▒' => C_CYAN,
            '█' => C_GREEN,
            _ => C_RESET,
        };
        if color != last_color {
            result.push_str(color);
            last_color = color;
        }
        result.push(ch);
    }
    result.push_str(C_RESET);
    result
}

/// Format uptime from ticks and hz into a human-readable string.
fn format_uptime(ticks: u64, hz: u64) -> String {
    if hz == 0 {
        return String::from("unknown");
    }
    let total_secs = ticks / hz;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

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
    while logo_lines.last().map_or(false, |l| l.trim().is_empty()) {
        logo_lines.pop();
    }

    // Find logo width (max visible column count)
    let logo_w: usize = logo_lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let pad = 2usize; // gap between logo and info

    // Build info lines — only facts the kernel actually exposes via syscalls.
    // No hardcoded policy, no fake identifiers.
    let mut info: Vec<String> = Vec::new();

    // Separator
    let mut sep = String::new();
    sep.push_str(C_DIM);
    for _ in 0..24 { sep.push('-'); }
    sep.push_str(C_RESET);
    info.push(sep);

    // Uptime (from real kernel tick counter)
    if s.uptime_available != 0 {
        let uptime = format_uptime(s.uptime_ticks, s.uptime_hz);
        info.push(format!("{}Uptime{}: {}{}{}", C_LABEL, C_RESET, C_VALUE, uptime, C_RESET));
    }

    // Display (from real framebuffer info)
    if fb.width > 0 && fb.height > 0 {
        info.push(format!("{}Display{}: {}{}x{}{}", C_LABEL, C_RESET, C_VALUE, fb.width, fb.height, C_RESET));
    }

    // Memory (from real PMM stats)
    info.push(format!(
        "{}Memory{}: {}{} KiB / {} KiB ({}%){}",
        C_LABEL, C_RESET, C_VALUE,
        s.pmm_used_bytes / 1024, s.pmm_total_bytes / 1024, s.memory_used_percent(),
        C_RESET
    ));

    // Heap (from real kernel heap stats)
    info.push(format!(
        "{}Heap{}: {}{} KiB / {} KiB{}",
        C_LABEL, C_RESET, C_VALUE,
        s.heap_used_bytes / 1024, s.heap_total_bytes / 1024,
        C_RESET
    ));

    // Processes (from real scheduler)
    info.push(format!(
        "{}Processes{}: {}{} total, {} running{}",
        C_LABEL, C_RESET, C_VALUE,
        s.process_total, s.process_running,
        C_RESET
    ));

    // Filesystem (from real VFS stats)
    info.push(format!(
        "{}FS{}: {}{} files, {} dirs, {} KiB{}",
        C_LABEL, C_RESET, C_VALUE,
        s.fs_files, s.fs_directories, s.fs_bytes / 1024,
        C_RESET
    ));

    // Open handles
    info.push(format!(
        "{}Open FDs{}: {}{}{}",
        C_LABEL, C_RESET, C_VALUE,
        s.fs_open_handles,
        C_RESET
    ));

    // IPC (from real IPC subsystem)
    if s.ipc_queue_count > 0 || s.ipc_shared_regions > 0 {
        info.push(format!(
            "{}IPC{}: {}{} queues, {} msgs, {} shm regions{}",
            C_LABEL, C_RESET, C_VALUE,
            s.ipc_queue_count, s.ipc_queued_messages, s.ipc_shared_regions,
            C_RESET
        ));
    }

    // Network (from real NIC detection)
    if s.net_total_nics > 0 {
        info.push(format!(
            "{}NICs{}: {}{} detected, {} supported{}",
            C_LABEL, C_RESET, C_VALUE,
            s.net_total_nics, s.net_supported_nics,
            C_RESET
        ));
    }

    // PID (from real syscall)
    info.push(format!("{}PID{}: {}{}{}", C_LABEL, C_RESET, C_VALUE, pid, C_RESET));

    // CWD (from real syscall)
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
        // Logo part (padded to logo_w visible columns)
        if i < logo_lines.len() {
            let line = logo_lines[i];
            let colored = colorize_logo_line(line);
            out(&colored);
            let vis = line.chars().count();
            if vis < logo_w {
                for _ in 0..(logo_w - vis) {
                    out(" ");
                }
            }
        } else {
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
