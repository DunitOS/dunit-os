#![no_std]
#![no_main]

//! Untrusted terminal client for the M4 userspace DWM (Stack B).
//!
//! The last legacy GUI app reborn on the userspace stack. It owns a PTY, spawns
//! the `dsh` shell as the slave, and bridges three streams: keystrokes the
//! compositor routes to the focused window (IN_KEY) are written into the pty
//! master; the shell's stdout is drained from the master into an on-screen
//! scrollback; that scrollback is painted into our own shared buffer via the M4
//! UI Runtime (DUI + DSS + TTF). Same capability + wire-protocol path as
//! gui_client/gui_stat — the compositor owns display/input, we own our surface.

use core::panic::PanicInfo;

extern crate alloc;
use gui_protocol_v1::control::{IN_KEY, IN_SCROLL, IN_QUIT};

use alloc::vec::Vec;

use gui_protocol_v1::wire::{Request, FEATURE_ARGB8888, MAGIC};

use dunit_render::Surface;
use dunit_style::value::Color;
use dunit_text::Font;

// Modifier bit in the IN_KEY `mods` field (mirrors libdunit::KEYMOD_CTRL).
const KEYMOD_CTRL: u8 = 1 << 1;

// PS/2 scancodes (sc & 0x7F) for keys the kernel leaves un-cooked (ascii = 0).
const SC_BACKSPACE: u8 = 0x0E;
const SC_HOME: u8 = 0x47;
const SC_UP: u8 = 0x48;
const SC_LEFT: u8 = 0x4B;
const SC_RIGHT: u8 = 0x4D;
const SC_END: u8 = 0x4F;
const SC_DOWN: u8 = 0x50;
const SC_DELETE: u8 = 0x53;
// Page keys are handled locally for scrollback (never forwarded to the pty).
const SC_PGUP: u8 = 0x49;
const SC_PGDN: u8 = 0x51;

/// Translate a widened IN_KEY event (scancode + mods + cooked ASCII) into the
/// byte sequence to feed the pty. Returns the number of bytes written to `out`.
///
/// - Ctrl + letter -> the control code (`c & 0x1F`): Ctrl-C=0x03, Ctrl-D=0x04,
///   Ctrl-L=0x0C, etc.
/// - Any other cooked byte (printable, Enter `\n`, Tab `\t`) -> itself.
/// - Backspace (no ASCII) -> 0x08.
/// - Arrows / Home / End / Delete (no ASCII) -> the usual xterm ESC sequences,
///   which `dsh` parses for line editing and history.
fn encode_key(scancode: u8, mods: u8, ascii: u8, out: &mut [u8; 4]) -> usize {
    if mods & KEYMOD_CTRL != 0 && ascii.is_ascii_alphabetic() {
        out[0] = ascii & 0x1F;
        return 1;
    }
    if ascii != 0 {
        out[0] = ascii;
        return 1;
    }
    // Extended / navigation keys arrive with ascii == 0.
    let esc: &[u8] = match scancode {
        SC_BACKSPACE => &[0x08],
        SC_UP => &[0x1B, b'[', b'A'],
        SC_DOWN => &[0x1B, b'[', b'B'],
        SC_RIGHT => &[0x1B, b'[', b'C'],
        SC_LEFT => &[0x1B, b'[', b'D'],
        SC_HOME => &[0x1B, b'[', b'H'],
        SC_END => &[0x1B, b'[', b'F'],
        SC_DELETE => &[0x1B, b'[', b'3', b'~'],
        _ => &[],
    };
    out[..esc.len()].copy_from_slice(esc);
    esc.len()
}

const SURFACE: u64 = 1;
const BUFFER: u64 = 2;
const W: u32 = 560;
const H: u32 = 360;
const FMT_XRGB8888: u32 = 1;
/// ARGB8888: the buffer carries a straight-alpha channel. Selected instead of
/// XRGB when `[terminal] bg_alpha < 255`, so the compositor blends the terminal's
/// translucent background over the desktop. Must match the protocol's
/// `FORMAT_ARGB8888`; the compositor validates the imported buffer's format
/// against the CreateSurface format on every commit.
const FMT_ARGB8888: u32 = 2;

/// Poll cadence: block on compositor IPC at most this long, then drain the pty
/// and repaint. The compositor blits our buffer every tick regardless.
const POLL_MS: u64 = 40;

// APPEND_MARKER

/// Load the terminal's TTF, falling back to the shipped monospace default in the
/// initrd (/assets/fonts) on any error — assets live on the filesystem, not baked
/// into the ELF. Prefers the per-terminal `[terminal] font` when set; otherwise
/// the shared `[desktop] font`. Both are live config knobs.
fn load_font(tcfg: &dwm_settings::TerminalCfg) -> Result<Font, ()> {
    let cfg = dwm_settings::load();
    let path = if tcfg.font.as_str().is_empty() {
        cfg.desktop.font.as_str()
    } else {
        tcfg.font.as_str()
    };
    if let Some(bytes) = libdunit::read_binary(path, 4 * 1024 * 1024) {
        if let Ok(f) = Font::parse(bytes) {
            return Ok(f);
        }
    }
    libdunit::read_binary("/assets/fonts/DejaVuSansMono.ttf", 4 * 1024 * 1024)
        .ok_or(())
        .and_then(|b| Font::parse(b).map_err(|_| ()))
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::println("gui_terminal: PANIC");
    libdunit::exit(101)
}

fn u64_at(p: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&p[off..off + 8]);
    u64::from_le_bytes(a)
}

/// Fixed layout metric: the point size the scrollback renders at, paired with
/// `ROW_PX`. This is a rendering invariant (not user palette), so it stays a
/// const; the *colors* below are config-driven via `TerminalCfg`.
const FONT_PX: f32 = 13.0;
const ROW_PX: i32 = 16;
/// Padding from the surface edge to the first cell (both axes).
const PAD_X: i32 = 8;
const PAD_Y: i32 = 8;

/// Visible text rows that fit in a surface `h` px tall: an 8px top pad, then one
/// `ROW_PX`-tall row each. At least one row so a tiny surface still paints.
fn visible_rows_for(h: u32) -> usize {
    (((h as i32 - PAD_Y) / ROW_PX).max(1)) as usize
}

/// Columns that fit in a surface `w` px wide at the monospace `cell_w`.
fn cols_for(w: u32, cell_w: i32) -> usize {
    (((w as i32 - 2 * PAD_X) / cell_w).max(1)) as usize
}

/// Monospace cell width in px: the advance of a representative glyph at `FONT_PX`.
/// For a monospace font every glyph shares this advance, so it defines the grid.
fn mono_cell_w(font: &Font) -> i32 {
    let (_g, adv) = font.layout_line("M", FONT_PX);
    terminal_model::cell_pitch(adv)
}

mod terminal_model;
use terminal_model::{Cell, Palette, Term};

/// ARGB8888 -> render Color.
fn col(argb: u32) -> Color {
    Color::rgba(
        ((argb >> 16) & 0xFF) as u8,
        ((argb >> 8) & 0xFF) as u8,
        (argb & 0xFF) as u8,
        ((argb >> 24) & 0xFF) as u8,
    )
}

/// Paint the cell grid into the mapped ARGB8888 buffer: fill the background,
/// then draw the visible window of the document (scrollback rows followed by the
/// live grid rows) as monospace cells, and finally a block caret at the cursor.
/// `w`/`h` are the surface's CURRENT size — the compositor can resize us via a
/// server-pushed CONFIGURE, so paint against the live geometry, not the consts.
/// `bg` is the config-resolved background as `0xAARRGGBB`: its alpha (from
/// `[terminal] bg_alpha`) is written straight into every background pixel via
/// `clear` — NOT `fill_rect`, which would blend over the previous frame and drift
/// the alpha toward opaque. Glyphs draw opaque on top, so text stays crisp while
/// the background lets the desktop show through when `bg_alpha < 255`.
fn render(px: *mut u8, font: &Font, term: &Term, w: u32, h: u32, bg: u32) {
    let pixels = unsafe { core::slice::from_raw_parts_mut(px as *mut u32, (w * h) as usize) };
    let mut surf = Surface::new(pixels, w as usize, h as usize);
    surf.clear(col(bg));

    // The document is `scrollback` (evicted rows) followed by the live `grid`.
    // `scroll` counts how many rows the view is lifted above the live bottom
    // (0 = pinned to the bottom, where the cursor lives).
    let sb = term.scrollback.len();
    let total = sb + term.rows;
    let visible = term.rows.min(total);
    let bottom = (total - 1).saturating_sub(term.scroll);
    let start = (bottom + 1).saturating_sub(visible);
    for (i, idx) in (start..=bottom).enumerate() {
        let y = PAD_Y + i as i32 * ROW_PX;
        let row = if idx < sb {
            &term.scrollback[idx]
        } else {
            &term.grid[idx - sb]
        };
        draw_row(&mut surf, font, term.cell_w, y, row);
    }

    // Block caret: only when pinned to the bottom (scroll == 0), drawn as a
    // translucent overlay so it reads as a cursor without re-rendering a glyph.
    if term.scroll == 0 {
        let cx = PAD_X + term.ccol.min(term.cols - 1) as i32 * term.cell_w;
        let cy = PAD_Y + term.crow as i32 * ROW_PX;
        let c = col(term.fg);
        surf.fill_rect(
            cx as f32,
            cy as f32,
            term.cell_w as f32,
            ROW_PX as f32,
            Color::rgba(c.r, c.g, c.b, 150),
        );
    }
}

/// Draw each character at its cell origin, independent of foreground changes
/// and fractional advances in the configured font.
fn draw_row(surf: &mut Surface, font: &Font, cell_w: i32, y: i32, row: &[Cell]) {
    // Every cell has its own origin. Grouping text into naturally spaced runs
    // drifts from the integer cell grid even with a monospace font's fractional
    // advance, and becomes incorrect for a configured proportional font.
    let baseline = y as f32 + font.line_metrics(FONT_PX).ascent;
    for (column, cell) in row.iter().enumerate() {
        if cell.ch == ' ' {
            continue;
        }
        if let Some(bitmap) = font.rasterize(font.glyph_index(cell.ch), FONT_PX) {
            let x = PAD_X + column as i32 * cell_w;
            surf.blit_glyph(
                &bitmap,
                x + bitmap.left,
                (baseline + 0.5) as i32 - bitmap.top,
                col(cell.fg),
            );
        }
    }
}

// APPEND_START

fn release_buffer(handle: u32, address: usize, bytes: usize) {
    if libdunit::shared_buffer::release(handle, address, bytes) < 0 {
        libdunit::println("gui_terminal: failed to release shared mapping");
    }
}

/// Re-negotiate the surface at a new size after a server-pushed CONFIGURE: back
/// a FRESH kernel buffer of `new_w*new_h`, render the scrollback into it at the
/// new geometry, then replay the buffer half of the handshake with a NEW object
/// id (the compositor's freshness gate rejects a re-used id). Returns the new
/// `(handle, mapped ptr)`; the OLD buffer handle is closed. `serial` advances so
/// every request keeps a unique, increasing client serial. This is the terminal
/// half of maximize/restore — the compositor owns the geometry, we re-buffer.
#[allow(clippy::too_many_arguments)]
fn resize_surface(
    compositor: u32,
    old_buf: u32,
    old_address: usize,
    old_bytes: usize,
    font: &Font,
    term: &Term,
    new_w: u32,
    new_h: u32,
    obj: u64,
    token: u64,
    serial: &mut u64,
    bg: u32,
    fmt: u32,
) -> Option<(u32, *mut u8)> {
    let bytes = (new_w as usize)
        .checked_mul(new_h as usize)?
        .checked_mul(4)?;
    let nb = libdunit::handle_create_shared(bytes);
    if nb <= 0 {
        return None;
    }
    let nb = nb as u32;
    let mapped = libdunit::handle_map(nb, 0, bytes);
    if mapped <= 0 {
        libdunit::handle_close(nb);
        return None;
    }
    let npx = mapped as usize as *mut u8;
    render(npx, font, term, new_w, new_h, bg);

    // Transfer the fresh buffer (read-only) and announce {handle, size, object}.
    let dup = libdunit::handle_dup(
        nb,
        libdunit::RIGHT_READ | libdunit::RIGHT_MAP | libdunit::RIGHT_TRANSFER,
    );
    let ch = if dup > 0 {
        libdunit::handle_transfer(dup as u32, compositor)
    } else {
        -1
    };
    if ch <= 0 {
        if dup > 0 {
            libdunit::handle_close(dup as u32);
        }
        release_buffer(nb, npx as usize, bytes);
        return None;
    }
    // Commit the configure only after allocation, mapping and transfer succeed.
    let ack = Request::AckConfigure { configure: token }.encode(SURFACE, *serial);
    *serial += 1;
    libdunit::ipc_send(compositor, &ack);
    let ann = gui_protocol_v1::control::Capability {
        handle: ch as u32,
        advertised_bytes: bytes as u32,
        object: obj as u32,
    }
    .encode();
    libdunit::ipc_send(compositor, &ann);

    // IMPORT (fresh object id) / ATTACH / COMMIT (new token).
    let import = Request::ImportBuffer {
        width: new_w,
        height: new_h,
        stride: new_w * 4,
        format: fmt,
        offset: 0,
    }
    .encode(obj, *serial);
    *serial += 1;
    libdunit::ipc_send(compositor, &import);
    let attach = Request::AttachBuffer {
        buffer: obj,
        damage: Vec::new(),
    }
    .encode(SURFACE, *serial);
    *serial += 1;
    libdunit::ipc_send(compositor, &attach);
    let commit = Request::Commit {
        configure: token,
        frame_callback: 1,
    }
    .encode(SURFACE, *serial);
    *serial += 1;
    libdunit::ipc_send(compositor, &commit);

    // Release the old backing buffer; the compositor now blits the new one.
    release_buffer(old_buf, old_address, old_bytes);
    Some((nb, npx))
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // 0) Load this app's config (apps/gui_terminal.toml). A missing/garbage file
    // yields the built-in baseline. The resolved values drive the prompt (via the
    // shell's env), the font, the background (with alpha), and the text palette;
    // they also go to serial so a TOML edit is observable headless.
    let tcfg = dwm_settings::TerminalCfg::load("gui_terminal");
    libdunit::println(&alloc::format!(
        "gui_terminal: cfg from_file={} theme={} prompt={} fg={:#010X} bg={:#010X} bg_alpha={} font={}",
        tcfg.from_file as u32,
        tcfg.theme.as_str(),
        tcfg.prompt.as_str(),
        tcfg.fg,
        tcfg.bg,
        tcfg.bg_alpha,
        if tcfg.font.as_str().is_empty() { "<desktop>" } else { tcfg.font.as_str() },
    ));
    // Resolve the presentation format from the configured background opacity.
    // `bg_alpha == 255` keeps the opaque XRGB fast path (compositor straight-copy);
    // anything less presents ARGB so the compositor blends our background over the
    // desktop. `bg` folds that alpha into the CONFIGURED background RGB and is
    // written into every background pixel each frame via `Surface::clear`.
    let bg = (tcfg.bg_alpha << 24) | (tcfg.bg & 0x00FF_FFFF);
    let fmt = if tcfg.bg_alpha < 255 {
        FMT_ARGB8888
    } else {
        FMT_XRGB8888
    };
    // The live text palette (default fg + ANSI slots) comes straight from config.
    let palette = Palette {
        fg: tcfg.fg,
        ansi: tcfg.ansi,
        ansi_bright: tcfg.ansi_bright,
    };
    libdunit::println(&alloc::format!(
        "gui_terminal: argb bg_alpha={} fmt={}",
        tcfg.bg_alpha,
        if fmt == FMT_ARGB8888 {
            "argb8888"
        } else {
            "xrgb8888"
        },
    ));

    // 1) Handshake: compositor pid + our client id (id is a tint hint only).
    let mut m = [0u8; 8];
    if libdunit::ipc_recv_blocking(&mut m, 0) < 8 {
        libdunit::println("gui_terminal: FAIL recv handshake");
        libdunit::exit(1);
    }
    let compositor = u32::from_le_bytes([m[0], m[1], m[2], m[3]]);

    // 2) Create + map our own shared buffer and paint the initial (empty) frame.
    let bytes = (W * H * 4) as usize;
    let buf = libdunit::handle_create_shared(bytes);
    if buf <= 0 {
        libdunit::println("gui_terminal: FAIL create buffer");
        libdunit::exit(2);
    }
    let buf = buf as u32;
    let mapped = libdunit::handle_map(buf, 0, bytes);
    if mapped <= 0 {
        libdunit::println("gui_terminal: FAIL map buffer");
        libdunit::exit(3);
    }
    let mut px = mapped as usize as *mut u8;
    let font = match load_font(&tcfg) {
        Ok(f) => f,
        Err(_) => {
            libdunit::println("gui_terminal: FAIL font parse");
            libdunit::exit(7);
        }
    };
    let cell_w = mono_cell_w(&font);
    let mut term = Term::new(palette, cols_for(W, cell_w), visible_rows_for(H), cell_w);
    // The surface's live geometry. The compositor may resize us via a
    // server-pushed CONFIGURE, so track it rather than reusing the W/H consts.
    let mut cur_buf = buf;
    let mut cur_w = W;
    let mut cur_h = H;
    render(px, &font, &term, cur_w, cur_h, bg);

    let mut rx = [0u8; 256];

    // 3) HELLO -> WELCOME.
    let hello = Request::Hello {
        min_minor: 0,
        max_minor: 0,
        offered_features: FEATURE_ARGB8888,
        required_features: 0,
    }
    .encode(0, 1);
    libdunit::ipc_send(compositor, &hello);
    if libdunit::ipc_recv_blocking(&mut rx, 0) <= 0 {
        libdunit::println("gui_terminal: FAIL welcome");
        libdunit::exit(4);
    }

    // 4) CREATE_SURFACE -> RESULT + CONFIGURE (capture the configure token).
    let create = Request::CreateSurface {
        role: 1,
        width: W,
        height: H,
        format: fmt,
    }
    .encode(SURFACE, 2);
    libdunit::ipc_send(compositor, &create);
    libdunit::ipc_recv_blocking(&mut rx, 0); // RESULT
    let n = libdunit::ipc_recv_blocking(&mut rx, 0); // CONFIGURE
    let token = if n >= 32 { u64_at(&rx, 24) } else { 0 };

    // 5) ACK_CONFIGURE -> RESULT.
    let ack = Request::AckConfigure { configure: token }.encode(SURFACE, 3);
    libdunit::ipc_send(compositor, &ack);
    libdunit::ipc_recv_blocking(&mut rx, 0);

    // 6) Transfer the buffer capability (read-only) and announce it.
    let dup = libdunit::handle_dup(
        buf,
        libdunit::RIGHT_READ | libdunit::RIGHT_MAP | libdunit::RIGHT_TRANSFER,
    );
    let ch = if dup > 0 {
        libdunit::handle_transfer(dup as u32, compositor)
    } else {
        -1
    };
    if ch <= 0 {
        libdunit::println("gui_terminal: FAIL cap transfer");
        libdunit::exit(5);
    }
    let ann = gui_protocol_v1::control::Capability {
        handle: ch as u32,
        advertised_bytes: bytes as u32,
        object: BUFFER as u32,
    }
    .encode();
    libdunit::ipc_send(compositor, &ann);

    // 7) IMPORT / ATTACH / COMMIT (each -> RESULT).
    let import = Request::ImportBuffer {
        width: W,
        height: H,
        stride: W * 4,
        format: fmt,
        offset: 0,
    }
    .encode(BUFFER, 4);
    libdunit::ipc_send(compositor, &import);
    libdunit::ipc_recv_blocking(&mut rx, 0);
    let attach = Request::AttachBuffer {
        buffer: BUFFER,
        damage: alloc::vec::Vec::new(),
    }
    .encode(SURFACE, 5);
    libdunit::ipc_send(compositor, &attach);
    libdunit::ipc_recv_blocking(&mut rx, 0);
    let commit = Request::Commit {
        configure: token,
        frame_callback: 1,
    }
    .encode(SURFACE, 6);
    libdunit::ipc_send(compositor, &commit);
    libdunit::ipc_recv_blocking(&mut rx, 0);

    // 8) FRAME_DONE from the compositor's composition tick.
    let n = libdunit::ipc_recv_blocking(&mut rx, 0);
    let ok = n >= 32 && u16::from_le_bytes([rx[8], rx[9]]) == 0x8021;
    if ok {
        libdunit::println("gui_terminal: surface presented OK");
    } else {
        libdunit::println("gui_terminal: FAIL no frame done");
        libdunit::handle_close(buf);
        libdunit::exit(6);
    }

    // 9) Spawn the shell as our pty slave: dsh's stdin/stdout are the pty rings.
    let pty = libdunit::pty_create();
    if pty <= 0 {
        libdunit::println("gui_terminal: FAIL pty create");
        libdunit::handle_close(buf);
        libdunit::exit(8);
    }
    let pty = pty as u32;
    // Hand the shell its prompt through the environment (desktop policy lives in
    // config, not baked into dsh): `[terminal] prompt` → `PROMPT=<value>`.
    let prompt_env = alloc::format!("PROMPT={}", tcfg.prompt.as_str());
    if libdunit::pty_spawn_env("dsh", pty, &[&prompt_env]) <= 0 {
        libdunit::println("gui_terminal: FAIL pty spawn");
        libdunit::pty_close(pty);
        libdunit::handle_close(buf);
        libdunit::exit(9);
    }

    // 10) Bridge loop: block on compositor IPC (short timeout). Forward IN_KEY
    //     bytes into the pty master; on any wake, drain the shell's stdout into
    //     the scrollback and repaint if it changed. A server-pushed CONFIGURE
    //     (maximize/restore) re-buffers us at the new geometry. Exit on IN_QUIT.
    let mut sbuf = [0u8; 256];
    let mut next_obj: u64 = 3; // fresh buffer object id per resize (> import id 2)
    let mut serial: u64 = 7; // client request serial, continues past the handshake
    loop {
        let n = libdunit::ipc_recv_blocking(&mut rx, POLL_MS);
        let magic = if n >= 8 {
            u32::from_le_bytes([rx[0], rx[1], rx[2], rx[3]])
        } else {
            0
        };
        // Server-pushed CONFIGURE (opcode 0x8010): the compositor resized us.
        // Re-negotiate a fresh buffer at the new geometry, then keep rendering
        // the scrollback into it (visible rows recomputed from the new height).
        if magic == MAGIC && n >= 48 && u16::from_le_bytes([rx[8], rx[9]]) == 0x8010 {
            let tok = u64_at(&rx, 24);
            let nw = u32::from_le_bytes([rx[32], rx[33], rx[34], rx[35]]);
            let nh = u32::from_le_bytes([rx[36], rx[37], rx[38], rx[39]]);
            if nw == 0 || nh == 0 || (nw == cur_w && nh == cur_h) {
                let ack = Request::AckConfigure { configure: tok }.encode(SURFACE, serial);
                serial += 1;
                libdunit::ipc_send(compositor, &ack);
            } else {
                // Prepare a separate reflowed model. Allocation/transfer failure
                // preserves both the existing grid and its backing surface.
                let mut candidate = term.clone();
                candidate.resize(cols_for(nw, cell_w), visible_rows_for(nh));
                if let Some((nb, npx)) = resize_surface(
                    compositor,
                    cur_buf,
                    px as usize,
                    (cur_w as usize) * (cur_h as usize) * 4,
                    &font,
                    &candidate,
                    nw,
                    nh,
                    next_obj,
                    tok,
                    &mut serial,
                    bg,
                    fmt,
                ) {
                    term = candidate;
                    cur_buf = nb;
                    px = npx;
                    cur_w = nw;
                    cur_h = nh;
                    next_obj += 1;
                    libdunit::println("gui_terminal: reconfigured OK");
                } else {
                    libdunit::println(
                        "gui_terminal: resize failed; previous grid and surface retained",
                    );
                }
            }
        } else if let Some(input) = gui_protocol_v1::control::Input::decode(&rx[..n.max(0) as usize]) {
            match input.kind {
                IN_QUIT => break,
                IN_SCROLL => {
                    // Wheel delta (i32) in the lx field: positive = scroll up
                    // into the backlog, negative = back toward the live bottom.
                    let delta = input.x;
                    if delta != 0 {
                        term.scroll_by(delta.unsigned_abs() as usize * 3, delta > 0);
                    }
                }
                IN_KEY => {
                    // IN_KEY carries scancode, modifiers and cooked ASCII. Decode
                    // into a pty byte sequence: control codes for Ctrl+letter,
                    // the cooked byte for printables/Enter/Tab, and xterm ESC
                    // sequences for the extended nav keys (arrows/Home/End/Del).
                    let scancode = input.x as u8;
                    let mods = input.y as u8;
                    let ascii = input.value as u8;
                    // PgUp/PgDn scroll the local view a page at a time and are
                    // never forwarded to the pty.
                    if scancode == SC_PGUP {
                        term.scroll_by(term.rows.saturating_sub(1), true);
                    } else if scancode == SC_PGDN {
                        term.scroll_by(term.rows.saturating_sub(1), false);
                    } else {
                        let mut seq = [0u8; 4];
                        let len = encode_key(scancode, mods, ascii, &mut seq);
                        if len > 0 {
                            libdunit::pty_write(pty, &seq[..len]);
                        }
                    }
                }
                _ => {}
            }
        }
        // Drain whatever the shell has produced since the last tick.
        loop {
            let r = libdunit::pty_read(pty, &mut sbuf);
            if r > 0 {
                term.feed(&sbuf[..r as usize]);
            } else {
                break; // EAGAIN / EOF / EPIPE — nothing more this tick
            }
        }
        if term.dirty {
            render(px, &font, &term, cur_w, cur_h, bg);
            term.dirty = false;
        }
    }

    libdunit::pty_close(pty);
    release_buffer(
        cur_buf,
        px as usize,
        (cur_w as usize) * (cur_h as usize) * 4,
    );
    libdunit::exit(0)
}
