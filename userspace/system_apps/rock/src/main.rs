#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
//! Nano-level TUI text editor backed by the shared `dunit_editor` buffer.

extern crate alloc;
#[cfg(not(test))]
mod native {
    use alloc::{format, string::String, vec::Vec};
    use dunit::{
        args, env, errno, fs,
        io::{self, Read, Write},
        time::{self, Duration},
        Error, Result,
    };
    use dunit_editor::DocumentBuffer;

    use crate::tui::{default_dims, EditMode, RockTui};

    fn load_doc(path: Option<&str>) -> Result<DocumentBuffer> {
        let Some(p) = path else {
            return Ok(DocumentBuffer::new());
        };
        match fs::read(p) {
            Ok(bytes) => DocumentBuffer::from_bytes(Some(String::from(p)), &bytes)
                .map_err(|_| Error(errno::EINVAL)),
            Err(Error(errno::ENOENT)) => {
                let mut doc = DocumentBuffer::new();
                doc.set_path(p);
                Ok(doc)
            }
            Err(e) => Err(e),
        }
    }

    fn save_atomic(path: &str, bytes: &[u8]) -> core::result::Result<(), String> {
        let tmp = format!("{}.rock.tmp", path);
        let _ = fs::remove_file(&tmp);
        if fs::write(&tmp, bytes).is_ok() {
            let _ = fs::remove_file(path);
            if fs::rename(&tmp, path).is_ok() {
                return Ok(());
            }
            let _ = fs::remove_file(&tmp);
        }
        fs::write(path, bytes).map_err(|e| format!("{}", e.name()))
    }

    #[dunit::main]
    pub fn main() -> Result<()> {
        let argv = args::args()?;
        let mut target: Option<String> = None;
        let mut script: Option<String> = None;
        let mut verify_report = false;
        let mut start_mode = EditMode::Insert;
        let (mut rows, mut cols) = detect_env_dims();
        let mut idx = 1usize;
        while idx < argv.len() {
            match argv[idx].as_str() {
                "--verify" => verify_report = true,
                "--vim" => start_mode = EditMode::Normal,
                "--nano" => start_mode = EditMode::Insert,
                "--size" if idx + 1 < argv.len() => {
                    idx += 1;
                    if let Some((r, c)) = parse_dims(&argv[idx]) {
                        rows = r;
                        cols = c;
                    }
                }
                "--script" if idx + 1 < argv.len() => {
                    idx += 1;
                    script = Some(argv[idx].clone());
                }
                arg if target.is_none() => target = Some(String::from(arg)),
                _ => {}
            }
            idx += 1;
        }

        let doc = load_doc(target.as_deref())?;
        let mut tui = RockTui::with_mode(doc, rows, cols, start_mode);
        let mut out = io::stdout();

        if let Some(seq) = script {
            let unescaped = decode_script(&seq);
            let _ = tui.feed_bytes(&unescaped, save_atomic);
            let _ = tui.flush_pending_esc();
            if verify_report {
                if let Some(hash) = tui.last_saved_sha256.as_deref() {
                    let path = tui.doc.path().unwrap_or("");
                    dunit::println!(
                        "[ROCK] saved {} bytes={} sha256={}",
                        path,
                        tui.doc.bytes_len(),
                        hash
                    )?;
                }
            }
            return Ok(());
        }

        write_retry(&mut out, b"\x1b[2J\x1b[H")?;
        write_retry(&mut out, tui.render_ansi().as_bytes())?;

        let mut stdin = io::stdin();
        let mut buf = [0u8; 64];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tui.feed_bytes(&buf[..n], save_atomic) {
                        if tui.quit {
                            break;
                        }
                        write_retry(&mut out, tui.render_ansi().as_bytes())?;
                    }
                    if tui.quit {
                        break;
                    }
                }
                Err(Error(errno::EAGAIN)) => {
                    if tui.flush_pending_esc() {
                        write_retry(&mut out, tui.render_ansi().as_bytes())?;
                    }
                    time::sleep(Duration::from_millis(15))?;
                }
                Err(e) => return Err(e),
            }
        }

        write_retry(&mut out, b"\x1b[?25h\x1b[2J\x1b[H")?;
        if verify_report {
            if let Some(hash) = tui.last_saved_sha256.as_deref() {
                let path = tui.doc.path().unwrap_or("");
                dunit::println!(
                    "[ROCK] saved {} bytes={} sha256={}",
                    path,
                    tui.doc.bytes_len(),
                    hash
                )?;
            }
        }
        Ok(())
    }

    fn write_retry(out: &mut io::Stdout, mut bytes: &[u8]) -> Result<()> {
        let mut spins = 0u32;
        while !bytes.is_empty() {
            match out.write(bytes) {
                Ok(0) => return Err(Error(errno::EIO)),
                Ok(n) => {
                    bytes = &bytes[n.min(bytes.len())..];
                    spins = 0;
                }
                Err(Error(errno::EAGAIN)) if spins < 50 => {
                    spins += 1;
                    time::sleep(Duration::from_millis(2))?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn parse_dims(spec: &str) -> Option<(usize, usize)> {
        let (r, c) = spec.split_once('x').or_else(|| spec.split_once('X'))?;
        let rows = r.trim().parse::<usize>().ok()?;
        let cols = c.trim().parse::<usize>().ok()?;
        Some((rows.max(8), cols.max(32)))
    }

    fn detect_env_dims() -> (usize, usize) {
        let (mut rows, mut cols) = default_dims();
        if let Ok(Some(v)) = env::var("LINES") {
            if let Ok(n) = v.trim().parse::<usize>() {
                rows = n.max(8);
            }
        }
        if let Ok(Some(v)) = env::var("COLUMNS") {
            if let Ok(n) = v.trim().parse::<usize>() {
                cols = n.max(32);
            }
        }
        (rows, cols)
    }

    fn decode_script(spec: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(spec.len());
        let bytes = spec.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                match bytes[i + 1] {
                    b'n' => {
                        out.push(b'\n');
                        i += 2;
                        continue;
                    }
                    b'r' => {
                        out.push(b'\r');
                        i += 2;
                        continue;
                    }
                    b'e' => {
                        out.push(0x1B);
                        i += 2;
                        continue;
                    }
                    b't' => {
                        out.push(b'\t');
                        i += 2;
                        continue;
                    }
                    b's' => {
                        out.push(b' ');
                        i += 2;
                        continue;
                    }
                    b'\\' => {
                        out.push(b'\\');
                        i += 2;
                        continue;
                    }
                    _ => {}
                }
            }
            if bytes[i] == b'^' && i + 1 < bytes.len() {
                let c = bytes[i + 1];
                if (b'A'..=b'Z').contains(&c) {
                    out.push(c - b'A' + 1);
                    i += 2;
                    continue;
                }
                if c == b'^' {
                    out.push(0x1E);
                    i += 2;
                    continue;
                }
                if c == b'_' {
                    out.push(0x1F);
                    i += 2;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        out
    }
}

#[cfg(not(test))]
mod tui;

#[cfg(test)]
mod tui;
