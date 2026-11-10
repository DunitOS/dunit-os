#![no_std]
#![no_main]

//! Console frontend: ANSI cells, scrollback and document selection.
//! Native SDK owns window buffers, cached glyph masks, PTYs and child status.

extern crate alloc;
use dunit::window::{PixelFormat, Window, WindowEvent};
use dunit::window::{
    IN_DOWN, IN_FOCUS_IN, IN_FOCUS_OUT, IN_KEY, IN_LEAVE, IN_MOVE, IN_SCROLL, IN_UP,
};

mod terminal_transfer;

use alloc::vec::Vec;

use dunit::canvas::{Color, Font, Surface};

const SC_PGUP: u8 = 0x49;
const SC_PGDN: u8 = 0x51;
use dunit::io::{Read, Write};
use dunit::pty::{encode_key, PendingInput, Pty};
const W: u32 = 560;
const H: u32 = 360;
/// Poll cadence: block on compositor IPC at most this long, then drain the pty
/// and repaint. The compositor blits our buffer every tick regardless.
const POLL_MS: u64 = 40;

/// Load the terminal's TTF, falling back to the shipped monospace default in the
/// initrd (/assets/fonts) on any error — assets live on the filesystem, not baked
/// into the ELF. Prefers the per-terminal `[terminal] font` when set; otherwise
/// the shared `[desktop] font`. Both are live config knobs.
fn load_font(tcfg: &dwm_settings::TerminalCfg) -> dunit::Result<Font> {
    let cfg = dwm_settings::load();
    let path = if tcfg.font.as_str().is_empty() {
        cfg.desktop.font.as_str()
    } else {
        tcfg.font.as_str()
    };
    Font::load(&[path, "/assets/fonts/DejaVuSansMono.ttf"])
}
fn log(message: &str) {
    let _ = dunit::println!("{}", message);
}
fn fatal(why: &str, error: isize) -> ! {
    log(&alloc::format!("gui_terminal: FAIL {} ({})", why, error));
    dunit::process::exit(1)
}
fn checked<T>(result: dunit::Result<T>, why: &str) -> T {
    result.unwrap_or_else(|error| fatal(why, error.0))
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
fn render(pixels: &mut [u32], font: &Font, term: &Term, w: u32, h: u32, bg: u32) {
    let mut surf = Surface::new(pixels, w as usize, h as usize);
    surf.clear(col(bg));

    // The document is `scrollback` (evicted rows) followed by the live `grid`.
    // `scroll` counts how many rows the view is lifted above the live bottom
    // (0 = pinned to the bottom, where the cursor lives).
    let sb = term.scrollback.len();
    let start = term.visible_start();
    for (i, idx) in (start..start + term.rows).enumerate() {
        let y = PAD_Y + i as i32 * ROW_PX;
        let row = if idx < sb {
            &term.scrollback[idx]
        } else {
            &term.grid[idx - sb]
        };
        for column in 0..term.cols {
            if term.selected(idx, column) {
                surf.fill_rect(
                    (PAD_X + column as i32 * term.cell_w) as f32,
                    y as f32,
                    term.cell_w as f32,
                    ROW_PX as f32,
                    col(term.palette.ansi[4]),
                );
            }
        }
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

fn paint(window: &mut Window, term: &mut Term, font: &Font, bg: u32) {
    let mut resized = None;
    checked(
        window.draw(|frame| {
            let (w, h) = frame.size();
            let cols = cols_for(w, term.cell_w);
            let rows = visible_rows_for(h);
            if cols != term.cols || rows != term.rows {
                let mut candidate = term.clone();
                candidate.resize(cols, rows);
                render(frame.pixels_mut(), font, &candidate, w, h, bg);
                resized = Some(candidate);
            } else {
                render(frame.pixels_mut(), font, term, w, h, bg);
                resized = None;
            }
        }),
        "present frame",
    );
    if let Some(candidate) = resized {
        *term = candidate;
    }
    term.dirty = false;
}

#[dunit::main]
fn main() -> dunit::Result<()> {
    let owned_arguments = dunit::args::args()?;
    let arguments: Vec<&str> = owned_arguments.iter().map(|s| s.as_str()).collect();
    let command = if arguments.len() == 1 {
        alloc::vec!["dsh"]
    } else if arguments.len() >= 3 && arguments[1] == "--execute" && !arguments[2].is_empty() {
        arguments[2..].to_vec()
    } else {
        log("gui_terminal: FAIL expected --execute executable [arguments]");
        dunit::process::exit(2);
    };
    // 0) Load this app's config (apps/gui_terminal.toml). A missing/garbage file
    // yields the built-in baseline. The resolved values drive the prompt (via the
    // shell's env), the font, the background (with alpha), and the text palette;
    // they also go to serial so a TOML edit is observable headless.
    let tcfg = dwm_settings::TerminalCfg::load("gui_terminal");
    log(&alloc::format!(
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
        PixelFormat::Argb8888
    } else {
        PixelFormat::Xrgb8888
    };
    // The live text palette (default fg + ANSI slots) comes straight from config.
    let palette = Palette {
        fg: tcfg.fg,
        ansi: tcfg.ansi,
        ansi_bright: tcfg.ansi_bright,
    };
    log(&alloc::format!(
        "gui_terminal: argb bg_alpha={} fmt={}",
        tcfg.bg_alpha,
        if fmt == PixelFormat::Argb8888 {
            "argb8888"
        } else {
            "xrgb8888"
        },
    ));

    let font = load_font(&tcfg).unwrap_or_else(|_| fatal("font parse", dunit::errno::EINVAL));
    let cell_w = mono_cell_w(&font);
    let mut window = checked(Window::open_format(W, H, fmt), "window handshake");
    let (width, height) = window.size();
    let mut term = Term::new(
        palette,
        cols_for(width, cell_w),
        visible_rows_for(height),
        cell_w,
    );
    paint(&mut window, &mut term, &font, bg);
    log("gui_terminal: surface presented OK");
    let mut transfers = terminal_transfer::Transfers::start(&mut window);

    // 9) Spawn the shell/command as our PTY slave; keep argv distinct.
    let mut pty = Pty::open()?;
    // Hand the shell its prompt through the environment (desktop policy lives in
    // config, not baked into dsh): `[terminal] prompt` → `PROMPT=<value>`.
    let prompt_env = alloc::format!("PROMPT={}", tcfg.prompt.as_str());
    let mut child = Some(pty.spawn(command[0], &command, &[&prompt_env])?);

    let mut sbuf = [0u8; 256];
    let mut input = PendingInput::new(dunit::transfer::MAX_BODY_BYTES);
    loop {
        let mut closing = false;
        for batch in 0..64 {
            let timeout = if batch != 0 {
                0
            } else if input.is_empty() {
                POLL_MS
            } else {
                5
            };
            match checked(
                window.next_event(dunit::time::Duration::from_millis(timeout)),
                "window event",
            ) {
                None => break,
                Some(WindowEvent::CloseRequested) => {
                    closing = true;
                    break;
                }
                Some(WindowEvent::Resized { .. }) => {
                    transfers.resized();
                    let (w, h) = window.size();
                    term.resize(cols_for(w, cell_w), visible_rows_for(h));
                    term.dirty = true;
                    log("gui_terminal: reconfigured OK");
                }
                Some(WindowEvent::Transfer(event)) => checked(
                    transfers.event(&mut window, event, &mut input),
                    "transfer event",
                ),
                Some(WindowEvent::Input(event)) => match event.kind {
                    IN_FOCUS_IN | IN_FOCUS_OUT => {
                        transfers.focus(event.kind == IN_FOCUS_IN, &mut term)
                    }
                    IN_DOWN | IN_UP | IN_MOVE | IN_LEAVE => checked(
                        transfers.pointer(&mut window, &mut term, &event),
                        "selection",
                    ),
                    IN_SCROLL => term.scroll_wheel(event.x, 3),
                    IN_KEY => {
                        if !checked(transfers.key(&mut window, &term, &event), "clipboard key") {
                            let scan = event.x as u8;
                            if scan == SC_PGUP {
                                term.scroll_by(term.rows.saturating_sub(1), true);
                            } else if scan == SC_PGDN {
                                term.scroll_by(term.rows.saturating_sub(1), false);
                            } else if child.is_some() {
                                let mut seq = [0; 4];
                                let len =
                                    encode_key(scan, event.y as u8, event.value as u8, &mut seq);
                                if len != 0 && !input.push(&seq[..len]) {
                                    log("gui_terminal: input queue full");
                                }
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if closing {
            break;
        }
        if let Err(error) = input.drain(|bytes| pty.write(bytes)) {
            if error.0 == dunit::errno::EPIPE {
                input.clear();
                checked(transfers.read_only(&mut window), "read-only terminal");
            } else if error.0 != dunit::errno::EAGAIN && error.0 != dunit::errno::ENOBUFS {
                fatal("PTY input", error.0);
            }
        }
        // Drain whatever the child has produced since the last tick.
        loop {
            match pty.read(&mut sbuf) {
                Ok(0) => break,
                Ok(n) => term.feed(&sbuf[..n]),
                Err(error) if matches!(error.0, dunit::errno::EAGAIN | dunit::errno::EPIPE) => {
                    break
                }
                Err(error) => return Err(error),
            }
        }
        if let Some(process) = child.as_mut() {
            if let Some(status) = process.try_wait()? {
                child = None;
                input.clear();
                checked(transfers.read_only(&mut window), "read-only terminal");
                log(&alloc::format!(
                    "gui_terminal: child={} exited kind={} code={}",
                    command[0],
                    status.kind(),
                    status.raw_code()
                ));
            }
        }
        if term.dirty {
            paint(&mut window, &mut term, &font, bg);
        }
    }

    if let Some(mut process) = child {
        process.terminate_and_wait()?;
        log(&alloc::format!(
            "gui_terminal: child={} stopped on close",
            command[0]
        ));
    }
    pty.close()?;
    window.close()
}
