#![no_std]
#![no_main]

//! Interactive PTY-slave shell for the M4 userspace terminal (Stack B).
//!
//! Spawned by `gui_terminal` through `pty_spawn`, so fd 0/1/2 are routed to the
//! pty rings (not the console). It runs a full line editor over the raw byte
//! stream the terminal forwards, and dispatches a command set that MIRRORS the
//! kernel text-mode shell (`kernel/src/shell.rs`) in syntax and output — the
//! same `ls / cd / pwd / mkdir / touch / cat / rm / tree / echo(>,>>)` FS verbs
//! and the same `uname / whoami / date / uptime / free / ps` system verbs — so
//! the GUI terminal behaves "just like a clean terminal". The FS verbs go
//! through the SDK filesystem APIs (resolved against this process's
//! cwd by the kernel); the system verbs read the read-only counters exposed by
//! `get_system_stats` (syscall 26). No kernel-internal APIs, no hardcoded
//! policy: pure userspace. Any other name is resolved through PATH on a private
//! inner pty whose stdio we bridge until it exits (see `spawn_external`).

extern crate alloc;
mod command_line;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use dunit::{
    env, fs,
    io::{self, Read, Write},
    process,
    pty::{PendingInput, Pty},
    system,
    time::{self, Duration},
    Error, Result,
};

/// Fallback shell prompt when no `PROMPT` is present in the environment. The
/// terminal passes its configured `[terminal] prompt` (default "dsh") via the
/// pty-spawn env, so this only applies when dsh is launched without one.
const PROMPT_DEFAULT: &str = "dsh";

fn out(s: &str) {
    if io::stdout().write_all(s.as_bytes()).is_err() {
        // A disconnected or failed terminal cannot display further errors.
        process::exit(1);
    }
}

fn cwd() -> Result<String> {
    fs::current_dir()
}

/// Join a directory path and a child name into an absolute path.
fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

/// Resolve `path` (absolute or relative to `base`) into a canonical absolute
/// path, folding `.`/`..`/empty components — mirrors the kernel shell's
/// `normalize_at` so `tree`/`cd` behave identically to the text terminal.
fn normalize(base: &str, path: &str) -> String {
    let combined = if path.starts_with('/') {
        String::from(path)
    } else {
        join(base, path)
    };
    let mut stack: Vec<&str> = Vec::new();
    for comp in combined.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            c => stack.push(c),
        }
    }
    let mut result = String::from("/");
    for (i, c) in stack.iter().enumerate() {
        if i > 0 {
            result.push('/');
        }
        result.push_str(c);
    }
    result
}

/// Split a command line into `(command, rest)` on the first run of whitespace;
/// `rest` is the trimmed remainder (a single argument/text/path, exactly like
/// the kernel shell's prefix dispatch).
fn split_cmd(line: &str) -> (&str, &str) {
    match line.find(|c: char| c.is_ascii_whitespace()) {
        Some(i) => (&line[..i], line[i..].trim_start()),
        None => (line, ""),
    }
}

/// Dispatch one command line. FS verbs go through SDK APIs (resolved
/// against this process's cwd by the kernel); system verbs read `get_system_stats`.
/// External commands use the userspace PATH resolver and preserve argument words.
fn run(line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let (cmd, rest) = split_cmd(line);
    match cmd {
        "help" => cmd_help(),
        "echo" => cmd_echo(rest),
        "pwd" => match cwd() {
            Ok(path) => {
                out(&path);
                out("\n");
            }
            Err(error) => out(&format!("pwd: {}\n", error)),
        },
        "ls" => builtin_paths(rest, cmd_ls, true),
        "cd" => builtin_paths(rest, cmd_cd, false),
        "mkdir" => builtin_paths(rest, cmd_mkdir, true),
        "touch" => builtin_paths(rest, cmd_touch, true),
        "cat" => builtin_paths(rest, cmd_cat, true),
        "rm" => builtin_paths(rest, cmd_rm, true),
        "tree" => builtin_paths(rest, cmd_tree, true),
        "clear" => out("\x1b[2J\x1b[H"),
        "uname" => cmd_uname(rest),
        "whoami" => out("root\n"),
        "date" => out("date: RTC unavailable\n"),
        "uptime" => cmd_uptime(),
        "free" => cmd_free(),
        "ps" => cmd_ps(),
        "top" => cmd_top(),
        "poweroff" | "shutdown" => {
            out("shutdown not implemented: ACPI/QEMU shutdown device unavailable\n")
        }
        // Privileged hardware/block/fs-admin diagnostics: a userspace shell can't
        // touch PCI/AHCI/block itself, so these are rendered by the kernel (the
        // owner of those drivers) via the `terminal_diag` gateway and printed
        // verbatim — identical output to kernel Terminal Mode, one source of truth.
        "lspci" | "usb" | "blk" | "lsblk" | "ahci" | "devs" | "blkread" | "blkwrite"
        | "mkfs.dunit" | "mount.dunit" | "install.dunit" => run_diag(line),
        "exit" => process::exit(0),
        "exec" => {
            run_external(rest);
        }
        _ => {
            run_external(line);
        }
    }
}

fn cmd_help() {
    out("dsh — Dunit userspace shell (full parity with text-mode terminal)\n");
    out("filesystem: ls [path]  cd [path]  pwd  mkdir <dir>  touch <file>  cat <file>  rm <path>  tree [path]\n");
    out("text:       echo <text> [> file | >> file]\n");
    out("system:     uname [-a]  whoami  date  uptime  free  ps  top  dufetch\n");
    out("hardware:   lspci  usb  devs  blk  lsblk  ahci  blkread  blkwrite\n");
    out("disk/fs:    mkfs.dunit  mount.dunit  install.dunit\n");
    out("control:    clear  help  exit  poweroff  shutdown\n");
    out("external: name [args] or exec name [args]; PATH search, explicit paths, quoted arguments\n");
}

fn run_external(line: &str) {
    let words = match command_line::words(line) {
        Ok(words) if !words.is_empty() => words,
        Ok(_) => {
            out("exec: missing operand\n");
            return;
        }
        Err(error) => {
            out(&format!("dsh: {}\n", error));
            return;
        }
    };
    let args: Vec<&str> = words.iter().map(|word| word.as_str()).collect();
    match spawn_external(args[0], &args) {
        Ok(()) => {}
        Err(Error(dunit::errno::ENOENT)) => out(&format!("dsh: command not found: {}\n", args[0])),
        Err(error) => out(&format!("dsh: {}: {}\n", args[0], error)),
    }
}

fn builtin_paths(rest: &str, command: fn(&str), multiple: bool) {
    match command_line::words(rest) {
        Ok(paths) if paths.is_empty() => command(""),
        Ok(paths) if multiple || paths.len() == 1 => {
            for path in paths {
                command(&path);
            }
        }
        Ok(_) => out("dsh: expected one path\n"),
        Err(error) => out(&format!("dsh: {}\n", error)),
    }
}
fn cmd_echo(rest: &str) {
    let (arguments, destination) = match dunit_shell::echo_arguments(rest) {
        Ok(value) => value,
        Err(error) => {
            out(&format!("echo: {}\n", error));
            return;
        }
    };
    let data = format!("{}\n", arguments.join(" "));
    if let Some((path, append)) = destination {
        let result = if append {
            (|| -> Result<()> {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .append(true)
                    .create(true)
                    .open(&path)?;
                file.write_all(data.as_bytes())?;
                file.close()
            })()
        } else {
            fs::write(&path, data.as_bytes())
        };
        if result.is_err() {
            out(&format!("echo: {}: write error\n", path));
        }
    } else {
        out(&data);
    }
}

fn cmd_ls(rest: &str) {
    let path = if rest.is_empty() { "." } else { rest };
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            out(&format!("ls: {}: {}\n", path, error));
            return;
        }
    };
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            out("  ");
        }
        out(&entry.name);
    }
    out("\n");
}

fn cmd_cd(rest: &str) {
    let path = if rest.is_empty() { "/" } else { rest };
    if let Err(error) = fs::set_current_dir(path) {
        out(&format!("cd: {}: {}\n", path, error));
    }
}

fn cmd_mkdir(rest: &str) {
    if rest.is_empty() {
        out("mkdir: missing operand\n");
        return;
    }
    if let Err(error) = fs::create_dir(rest) {
        out(&format!("mkdir: {}: {}\n", rest, error));
    }
}

fn cmd_touch(rest: &str) {
    if rest.is_empty() {
        out("touch: missing operand\n");
        return;
    }
    let result = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(rest)
        .and_then(|file| file.close());
    if let Err(error) = result {
        out(&format!("touch: {}: {}\n", rest, error));
    }
}

fn cmd_cat(rest: &str) {
    if rest.is_empty() {
        out("cat: missing operand\n");
        return;
    }
    let mut file = match fs::File::open(rest) {
        Ok(file) => file,
        Err(error) => {
            out(&format!("cat: {}: {}\n", rest, error));
            return;
        }
    };
    let mut buf = [0u8; 512];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => match core::str::from_utf8(&buf[..n]) {
                Ok(text) => out(text),
                Err(_) => out("<binary>"),
            },
            Err(error) => {
                out(&format!("cat: {}: {}\n", rest, error));
                break;
            }
        }
    }
    if let Err(error) = file.close() {
        out(&format!("cat: {}: {}\n", rest, error));
    }
    out("\n");
}

fn cmd_rm(rest: &str) {
    if rest.is_empty() {
        out("rm: missing operand\n");
        return;
    }
    if let Err(error) = fs::remove_file(rest) {
        out(&format!("rm: {}: {}\n", rest, error));
    }
}

fn cmd_tree(rest: &str) {
    let cwd = match cwd() {
        Ok(path) => path,
        Err(error) => {
            out(&format!("tree: {}\n", error));
            return;
        }
    };
    let base = normalize(&cwd, if rest.is_empty() { "." } else { rest });
    out(&base);
    out("\n");
    tree_recurse(&base, 1);
}

fn tree_recurse(dir: &str, depth: usize) {
    if depth > 16 {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            out(&format!("tree: {}: {}\n", dir, error));
            return;
        }
    };
    for entry in entries {
        let name = &entry.name;
        for _ in 0..depth {
            out("  ");
        }
        out(name);
        if entry.file_type == fs::FileType::Directory {
            out("/\n");
            tree_recurse(&join(dir, name), depth + 1);
        } else {
            out("\n");
        }
    }
}

fn cmd_uname(_rest: &str) {
    out("uname: system identity interface is not configured\n");
}

fn cmd_uptime() {
    let s = match system::stats() {
        Ok(stats) => stats,
        Err(error) => {
            out(&format!("dsh: system statistics unavailable: {}\n", error));
            return;
        }
    };
    let hz = s.uptime_hz;
    if hz == 0 {
        out("uptime: clock frequency unavailable\n");
        return;
    }
    let total = s.uptime_ticks / hz;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let sec = total % 60;
    out(&format!(
        "up {}h {}m {}s ({} ticks @ {} Hz)\n",
        h, m, sec, s.uptime_ticks, hz
    ));
}

fn cmd_free() {
    let s = match system::stats() {
        Ok(stats) => stats,
        Err(error) => {
            out(&format!("dsh: system statistics unavailable: {}\n", error));
            return;
        }
    };
    let kib = |b: u64| b / 1024;
    out("              total        used        free\n");
    out(&format!(
        "PMM KiB:   {:>10}  {:>10}  {:>10}\n",
        kib(s.pmm_total_bytes),
        kib(s.pmm_used_bytes),
        kib(s.pmm_free_bytes)
    ));
    out(&format!(
        "Heap KiB:  {:>10}  {:>10}  {:>10}\n",
        kib(s.heap_total_bytes),
        kib(s.heap_used_bytes),
        kib(s.heap_free_bytes)
    ));
    out("Swap: unavailable\n");
}

fn cmd_ps() {
    let s = match system::stats() {
        Ok(stats) => stats,
        Err(error) => {
            out(&format!("dsh: system statistics unavailable: {}\n", error));
            return;
        }
    };
    out(&format!(
        "processes: total={} running={} ready={} prepared={} blocked={} dead={} reaped={}\n",
        s.process_total,
        s.process_running,
        s.process_ready,
        s.process_prepared,
        s.process_blocked,
        s.process_dead,
        s.process_reaped
    ));
}

/// A one-shot `top`-style snapshot (dsh has no alternate screen, so it prints
/// once rather than refreshing): tasks + physical/heap memory + IPC + FS, all
/// read from the single `get_system_stats` syscall.
fn cmd_top() {
    let s = match system::stats() {
        Ok(stats) => stats,
        Err(error) => {
            out(&format!("dsh: system statistics unavailable: {}\n", error));
            return;
        }
    };
    let kib = |b: u64| b / 1024;
    out(&format!(
        "tasks: {} total, {} running, {} ready, {} blocked\n",
        s.process_total, s.process_running, s.process_ready, s.process_blocked
    ));
    out(&format!(
        "mem KiB:  {} total, {} used, {} free\n",
        kib(s.pmm_total_bytes),
        kib(s.pmm_used_bytes),
        kib(s.pmm_free_bytes)
    ));
    out(&format!(
        "heap KiB: {} total, {} used, {} free ({} blocks)\n",
        kib(s.heap_total_bytes),
        kib(s.heap_used_bytes),
        kib(s.heap_free_bytes),
        s.heap_free_blocks
    ));
    out(&format!(
        "ipc: {} queues, {} queued, {} shared regions\n",
        s.ipc_queue_count, s.ipc_queued_messages, s.ipc_shared_regions
    ));
    out(&format!(
        "fs: {} files, {} dirs\n",
        s.fs_files, s.fs_directories
    ));
}

/// Render a PRIVILEGED diagnostic by asking the kernel (which owns the hardware)
/// through the `terminal_diag` gateway, then print its text verbatim — identical
/// output to kernel Terminal Mode. 16 KiB covers the largest listing.
fn run_diag(line: &str) {
    let mut buf = alloc::vec![0u8; 16384];
    match system::terminal_diagnostic(line, &mut buf) {
        Ok(n) => {
            if io::stdout().write_all(&buf[..n]).is_err() {
                process::exit(1);
            }
        }
        Err(error) => out(&format!(
            "dsh: {}: diagnostic unavailable (err {})\n",
            line,
            error.raw_os_error()
        )),
    }
}

/// Run an external program `path` (typically `/app/<name>`) on a private inner
/// pty and bridge its stdio to ours until it exits. dsh becomes the MASTER of
/// the inner pty; the child's stdout is forwarded to our stdout (the terminal).
///
/// On the input side dsh applies a COOKED LINE DISCIPLINE identical to kernel
/// Terminal Mode (`kernel/src/lib.rs::terminal_collect_foreground_input`):
/// classic terminal-mode programs (e.g. `calc`) do NOT echo their own keystrokes
/// and expect to receive whole lines terminated by '\n'. So dsh echoes printable
/// keystrokes back to our stdout, erases on Backspace, swallows ESC/CSI cursor
/// sequences, and only forwards a complete line (plus the terminating '\n') to
/// the child when Enter is pressed. Ctrl-C (0x03) kills the child and drops the
/// pending line. Keeping this discipline in dsh (userspace policy) is what makes
/// the GUI terminal behave "just like a clean terminal" WITHOUT the kernel pty
/// (a raw byte mechanism) growing any line discipline of its own. Spawn and
/// bridge errors retain their native errno; every started child is reaped.
fn is_raw_tui_program(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    base == "rock"
}

fn spawn_external(path: &str, argv: &[&str]) -> Result<()> {
    let mut inner = Pty::open()?;
    let mut env_owned = Vec::new();
    for key in ["PATH", "LINES", "COLUMNS"] {
        if let Some(val) = env::var(key)? {
            env_owned.push(format!("{}={}", key, val));
        }
    }
    let env_refs: Vec<&str> = env_owned.iter().map(|s| s.as_str()).collect();
    let mut child = inner.spawn(path, argv, &env_refs)?;
    let raw = is_raw_tui_program(path);
    let result = bridge(&mut inner, &mut child, raw);
    // Always reap, including bridge failures and the exit/kill race.
    let reaped = if result.is_ok() {
        child.wait()
    } else {
        child.terminate_and_wait()
    };
    let closed = inner.close();
    result.and(reaped.map(|_| ())).and(closed)
}

fn bridge(inner: &mut Pty, child: &mut process::Child, raw: bool) -> Result<()> {
    let mut output = io::stdout();
    let mut input = io::stdin();
    let mut out_buf = [0u8; 512];
    let mut in_buf = [0u8; 64];
    let mut line = dunit_shell::CookedLine::new(4096);
    // Bound queued complete lines while continuing to service controls when
    // the child stops reading its native PTY ring.
    let mut pending = PendingInput::new(8192);
    let mut interrupted = false;
    let mut child_exited = false;
    // ESC-sequence state: normal, ESC, CSI. Child input stays cooked unless raw=true.
    let mut esc = 0u8;
    loop {
        let mut progressed = false;
        for _ in 0..8 {
            match inner.read(&mut out_buf) {
                Ok(0) => break,
                Ok(n) => {
                    output.write_all(&out_buf[..n])?;
                    progressed = true;
                }
                Err(Error(dunit::errno::EPIPE)) => return Ok(()),
                Err(Error(dunit::errno::EAGAIN)) => break,
                Err(error) => return Err(error),
            }
        }
        if !interrupted && !child_exited {
            child_exited = child.try_wait()?.is_some();
        }
        if !interrupted && !child_exited {
            let had_pending = !pending.is_empty();
            match pending.drain(|bytes| inner.write(bytes)) {
                Ok(()) => progressed |= had_pending && pending.is_empty(),
                Err(Error(dunit::errno::EAGAIN)) => {}
                Err(error) => return Err(error),
            }
            // Always service controls, even when the inner ring and pending
            // queue are full. Reading only one byte in cooked mode preserves stdin
            // ownership when the bridge hands control back to the shell editor.
            let max_read = if raw { in_buf.len() } else { 1 };
            match input.try_read(&mut in_buf[..max_read]) {
                Ok(0) => return Err(Error(dunit::errno::EPIPE)),
                Ok(n) => {
                    if raw {
                        let _ = pending.push(&in_buf[..n]);
                        let _ = pending.drain(|bytes| inner.write(bytes));
                    } else {
                        let b = in_buf[0];
                        if b == 0x03 {
                            // Ctrl-C preempts incomplete ESC/CSI sequences.
                            esc = 0;
                        } else {
                            match esc {
                                1 => {
                                    esc = if b == b'[' { 2 } else { 0 };
                                    continue;
                                }
                                2 => {
                                    if (0x40..=0x7e).contains(&b) {
                                        esc = 0;
                                    }
                                    continue;
                                }
                                _ => {}
                            }
                            if b == 0x1b {
                                esc = 1;
                                continue;
                            }
                        }
                        match line.feed(b) {
                            dunit_shell::CookedEvent::Echo(ch) => {
                                let mut bytes = [0; 4];
                                output.write_all(ch.encode_utf8(&mut bytes).as_bytes())?;
                            }
                            dunit_shell::CookedEvent::Erase => output.write_all(b"\x08 \x08")?,
                            dunit_shell::CookedEvent::Submit(bytes) => {
                                output.write_all(b"\n")?;
                                if !pending.push(&bytes) {
                                    output.write_all(b"input queue full; line discarded\n")?;
                                }
                            }
                            dunit_shell::CookedEvent::Rejected => {
                                output.write_all(b"\ninput line too long; discarded\n")?
                            }
                            dunit_shell::CookedEvent::Interrupt => {
                                pending.clear();
                                child.terminate_and_wait()?;
                                // Drain the child's output without stealing the
                                // next shell command after this interrupt byte.
                                interrupted = true;
                            }
                            dunit_shell::CookedEvent::None => {}
                        }
                    }
                    progressed = true;
                }
                Err(Error(dunit::errno::EAGAIN)) => {}
                Err(error) => return Err(error),
            }
        }
        if !progressed {
            time::sleep(Duration::from_millis(5))?;
        }
    }
}

/// ANSI escape-parser state for the line editor: bytes arriving from the
/// terminal may be raw ASCII or `ESC [ … final` control sequences (arrows,
/// Home/End/Delete) synthesized by `gui_terminal::encode_key`.
enum EscState {
    Normal,
    Esc,
    Csi,
}

/// A one-line editor with history and a real caret. It reprints the whole line
/// on every edit (`\r` + prompt + text + `ESC[K`, then `ESC[nD` to place the
/// caret), so the terminal only needs CR / EL / CUB — no absolute addressing.
struct Ed {
    line: String,
    cursor: usize,
    prompt: String,
    history: Vec<String>,
    hist_pos: usize,
    esc: EscState,
    csi_param: usize,
    utf8: dunit_shell::Utf8Decoder,
}

impl Ed {
    fn new(prompt_word: &str) -> Self {
        Ed {
            line: String::new(),
            cursor: 0,
            prompt: format!("{} ", prompt_word),
            history: Vec::new(),
            hist_pos: 0,
            esc: EscState::Normal,
            csi_param: 0,
            utf8: dunit_shell::Utf8Decoder::default(),
        }
    }

    fn start(&self) {
        out(&self.prompt);
    }

    fn feed(&mut self, b: u8) {
        match self.esc {
            EscState::Normal => self.feed_normal(b),
            EscState::Esc => {
                if b == b'[' {
                    self.esc = EscState::Csi;
                    self.csi_param = 0;
                } else {
                    self.esc = EscState::Normal;
                }
            }
            EscState::Csi => self.feed_csi(b),
        }
    }

    fn feed_normal(&mut self, byte: u8) {
        let Some(ch) = self.utf8.push(byte) else {
            return;
        };
        if !ch.is_ascii() {
            if !ch.is_control() {
                self.insert(ch);
            }
            return;
        }
        let b = ch as u8;
        match b {
            0x1B => self.esc = EscState::Esc,
            b'\r' | b'\n' => self.submit(),
            0x08 | 0x7F => self.backspace(),
            0x03 => self.cancel(),       // Ctrl-C
            0x0C => self.clear_screen(), // Ctrl-L
            0x01 => self.move_home(),    // Ctrl-A
            0x05 => self.move_end(),     // Ctrl-E
            0x15 => {
                // Ctrl-U: kill the whole line
                self.line.clear();
                self.cursor = 0;
                self.redraw();
            }
            0x20..=0x7E => self.insert(b as char),
            _ => {}
        }
    }

    fn feed_csi(&mut self, b: u8) {
        if b.is_ascii_digit() {
            self.csi_param = self.csi_param.saturating_mul(10) + (b - b'0') as usize;
            return;
        }
        match b {
            b'A' => self.history_prev(),
            b'B' => self.history_next(),
            b'C' => self.move_right(),
            b'D' => self.move_left(),
            b'H' => self.move_home(),
            b'F' => self.move_end(),
            b'~' => match self.csi_param {
                1 | 7 => self.move_home(),
                4 | 8 => self.move_end(),
                3 => self.delete_forward(),
                _ => {}
            },
            _ => {}
        }
        self.esc = EscState::Normal;
    }

    fn insert(&mut self, c: char) {
        self.line.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.redraw();
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor = dunit_shell::previous_boundary(&self.line, self.cursor);
            self.line.remove(self.cursor);
            self.redraw();
        }
    }

    fn delete_forward(&mut self) {
        if self.cursor < self.line.len() {
            self.line.remove(self.cursor);
            self.redraw();
        }
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = dunit_shell::previous_boundary(&self.line, self.cursor);
            self.redraw();
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.line.len() {
            self.cursor = dunit_shell::next_boundary(&self.line, self.cursor);
            self.redraw();
        }
    }

    fn move_home(&mut self) {
        self.cursor = 0;
        self.redraw();
    }

    fn move_end(&mut self) {
        self.cursor = self.line.len();
        self.redraw();
    }

    fn history_prev(&mut self) {
        if self.hist_pos > 0 {
            self.hist_pos -= 1;
            self.line = self.history[self.hist_pos].clone();
            self.cursor = self.line.len();
            self.redraw();
        }
    }

    fn history_next(&mut self) {
        if self.hist_pos < self.history.len() {
            self.hist_pos += 1;
            if self.hist_pos == self.history.len() {
                self.line.clear();
            } else {
                self.line = self.history[self.hist_pos].clone();
            }
            self.cursor = self.line.len();
            self.redraw();
        }
    }

    fn cancel(&mut self) {
        out("\n");
        self.line.clear();
        self.cursor = 0;
        self.hist_pos = self.history.len();
        out(&self.prompt);
    }

    fn clear_screen(&mut self) {
        out("\x1b[2J\x1b[H");
        self.redraw();
    }

    fn submit(&mut self) {
        out("\n");
        let entered = core::mem::take(&mut self.line);
        self.cursor = 0;
        let trimmed = entered.trim();
        if !trimmed.is_empty() && self.history.last().map(|s| s.as_str()) != Some(entered.as_str())
        {
            self.history.push(entered.clone());
        }
        self.hist_pos = self.history.len();
        run(&entered);
        out(&self.prompt);
    }

    fn redraw(&self) {
        out("\r");
        out(&self.prompt);
        out(&self.line);
        out("\x1b[K");
        let back = self.line[self.cursor..].chars().count();
        if back > 0 {
            out(&format!("\x1b[{}D", back));
        }
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let prompt_word = env::var("PROMPT")?;
    let mut ed = Ed::new(prompt_word.as_deref().unwrap_or(PROMPT_DEFAULT));
    ed.start();

    // Submission can hand stdin to a synchronous child. Do not prefetch the
    // next command/child input into an editor-private buffer before that handoff.
    let mut input = io::stdin();
    let mut buf = [0u8; 1];
    loop {
        match input.read(&mut buf)? {
            0 => return Ok(()),
            _ => ed.feed(buf[0]),
        }
    }
}
