//! Bounded window geometry store, keyed by application ID.
use super::*;
use super::scalar_document::{push_int, push_bool};

pub const SESSION_PATH: &str = "/system/share/dwm/session.toml";
pub const MAX_SESSION: usize = 32;

/// One remembered window: its app id (join key) and last floating geometry plus
/// whether it was maximized. `Copy` so the compositor keeps a plain `Vec` mirror.
#[derive(Clone, Copy)]
pub struct WinGeom {
    pub id: ConfigStr,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub maximized: bool,
}

impl WinGeom {
    pub fn new(id: &str, x: i32, y: i32, w: i32, h: i32, maximized: bool) -> Self {
        let mut s = ConfigStr::new("");
        s.set(id);
        WinGeom {
            id: s,
            x,
            y,
            w,
            h,
            maximized,
        }
    }
}

/// Read the session store into a flat list of remembered windows. Never fails: a
/// missing/empty/garbage file yields an empty list (fresh session). Bounded by
/// `MAX_SESSION` so a malformed store cannot make the compositor allocate wildly.
pub fn load_session() -> Vec<WinGeom> {
    let mut out: Vec<WinGeom> = Vec::new();
    let Some(text) = read_file(SESSION_PATH) else {
        return out;
    };
    let mut cur_id = String::new();
    let mut cur = WinGeom::new("", 0, 0, 0, 0, false);
    let mut have = false;
    let flush = |out: &mut Vec<WinGeom>, id: &str, g: &WinGeom| {
        if !id.is_empty() && g.w > 0 && g.h > 0 && out.len() < MAX_SESSION {
            let mut e = *g;
            e.id.set(id);
            out.push(e);
        }
    };
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if have {
                flush(&mut out, &cur_id, &cur);
            }
            cur_id.clear();
            cur = WinGeom::new("", 0, 0, 0, 0, false);
            have = false;
            // Table header `window.<id>`: the id after the first '.' is the join key.
            let hdr = name.trim();
            if let Some(rest) = hdr.strip_prefix("window.") {
                cur_id.push_str(rest.trim());
                have = true;
            }
            continue;
        }
        if !have {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let rhs = line[eq + 1..].trim();
        match key {
            "x" => {
                if let Some(n) = parse_uint(rhs) {
                    cur.x = n as i32;
                }
            }
            "y" => {
                if let Some(n) = parse_uint(rhs) {
                    cur.y = n as i32;
                }
            }
            "w" => {
                if let Some(n) = parse_uint(rhs) {
                    cur.w = n as i32;
                }
            }
            "h" => {
                if let Some(n) = parse_uint(rhs) {
                    cur.h = n as i32;
                }
            }
            "maximized" => {
                if let Some(b) = parse_bool(rhs) {
                    cur.maximized = b;
                }
            }
            _ => {}
        }
    }
    if have {
        flush(&mut out, &cur_id, &cur);
    }
    out
}

/// Serialize the remembered-window list to `SESSION_PATH`. One `[window.<id>]`
/// table each; the newest entry per id wins (the caller upserts its mirror before
/// calling). Returns whether the write succeeded; never panics.
pub fn save_session(entries: &[WinGeom]) -> bool {
    let mut text = String::new();
    text.push_str("# Dunit DWM session (window geometry; live in RAM this session).\n");
    for e in entries.iter().take(MAX_SESSION) {
        let id = e.id.as_str();
        if id.is_empty() {
            continue;
        }
        text.push_str("\n[window.");
        text.push_str(id);
        text.push_str("]\n");
        push_int(&mut text, "x", e.x as i64);
        push_int(&mut text, "y", e.y as i64);
        push_int(&mut text, "w", e.w as i64);
        push_int(&mut text, "h", e.h as i64);
        push_bool(&mut text, "maximized", e.maximized);
    }
    libdunit::write_string(SESSION_PATH, &text).is_ok()
}
