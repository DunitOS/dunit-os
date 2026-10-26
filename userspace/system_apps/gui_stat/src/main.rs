#![no_std]
#![no_main]

//! Untrusted system-monitor client for the M4 userspace DWM (Stack B).
//!
//! The legacy kernel-GUI `gui_stats` reborn as a gui-v1 client. Same capability
//! + wire-protocol path as `gui_client`, and it paints through the same M4 UI
//! Runtime (DUI + DSS + TTF text). The difference is the event loop: instead of
//! only reacting to pointer input, it blocks on IPC with a timeout and repaints
//! fresh native system counters each time it wakes. The SDK owns bootstrap,
//! buffer publication, resize, input and teardown; display master stays with
//! the server.

extern crate alloc;
use dunit::time::Duration;
use dunit::window::{Frame, Window, WindowEvent};

use alloc::string::String;

use dunit_render::{paint, Surface};
use dunit_style::cascade::{Cascade, NodeStyle};
use dunit_style::parse as parse_dss;
use dunit_text::Font;
use dunit_ui::layout::layout_measured;
use dunit_ui::parse as parse_dui;
use dunit_ui::tree::{Kind, NodeId};
use dunit_widgets::{intrinsic_size, FontMeasure, Widget};

const W: u32 = 460;
const H: u32 = 300;

/// Poll cadence: block on IPC at most this long, then repaint fresh counters.
const POLL_MS: u64 = 500;

/// Load the configured TTF (`[desktop] font`) from the VFS, falling back to the
/// shipped /assets default font on any error — the desktop font is a live config knob.
fn load_font() -> Result<Font, ()> {
    let cfg = dwm_settings::load();
    if let Some(bytes) = read_font(cfg.desktop.font.as_str()) {
        if let Ok(f) = Font::parse(bytes) {
            return Ok(f);
        }
    }
    read_font("/assets/fonts/DejaVuSans.ttf")
        .ok_or(())
        .and_then(|b| Font::parse(b).map_err(|_| ()))
}

fn read_font(path: &str) -> Option<alloc::vec::Vec<u8>> {
    use dunit::io::Read;
    let mut file = dunit::fs::File::open(path).ok()?;
    let size = usize::try_from(file.metadata().ok()?.len).ok()?;
    if size > 4 * 1024 * 1024 {
        return None;
    }
    let mut bytes = alloc::vec![0; size];
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

/// Human-readable byte size (B / KiB / MiB), matching the legacy stats window.
fn fmt_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        alloc::format!("{}MiB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        alloc::format!("{}KiB", bytes / 1024)
    } else {
        alloc::format!("{}B", bytes)
    }
}

/// Append a `Text "…"` child, stripping characters that would break DUI parsing.
fn push_line(d: &mut String, text: &str) {
    d.push_str("Text \"");
    for c in text.chars() {
        if c != '"' && c != '{' && c != '}' {
            d.push(c);
        }
    }
    d.push_str("\" ");
}

/// Build the DUI document for the current stats snapshot. Each row is one Text
/// child of a Column so the runtime stacks them vertically.
fn build_dui(s: &dunit::system::SystemStats) -> String {
    let mut d = String::new();
    d.push_str("Column#win { Text#title \"System Stats\" ");
    push_line(
        &mut d,
        &alloc::format!(
            "proc: total {} run {} ready {} blocked {}",
            s.process_total,
            s.process_running,
            s.process_ready,
            s.process_blocked
        ),
    );
    push_line(
        &mut d,
        &alloc::format!("dead {} reaped {}", s.process_dead, s.process_reaped),
    );
    push_line(
        &mut d,
        &alloc::format!(
            "pmm: {} / {}",
            fmt_size(s.pmm_used_bytes),
            fmt_size(s.pmm_total_bytes)
        ),
    );
    push_line(
        &mut d,
        &alloc::format!(
            "heap: {} used  {} free-blocks",
            fmt_size(s.heap_used_bytes),
            s.heap_free_blocks
        ),
    );
    push_line(
        &mut d,
        &alloc::format!(
            "ipc: queues {} msgs {} shared {}",
            s.ipc_queue_count,
            s.ipc_queued_messages,
            s.ipc_shared_regions
        ),
    );
    push_line(
        &mut d,
        &alloc::format!(
            "fs: files {} dirs {} {}",
            s.fs_files,
            s.fs_directories,
            fmt_size(s.fs_bytes)
        ),
    );
    push_line(
        &mut d,
        &alloc::format!(
            "net: nics {} supported {}",
            s.net_total_nics,
            s.net_supported_nics
        ),
    );
    push_line(&mut d, &alloc::format!("uptime: {}s", s.uptime_ticks / 100));
    d.push('}');
    d
}

/// Paint the current stats snapshot into the private XRGB8888 frame using the
/// M4 UI Runtime (DUI tree -> DSS cascade -> content-measured layout -> render).
fn render_stats(frame: &mut Frame<'_>, font: &Font, s: &dunit::system::SystemStats) {
    let (width, height) = frame.size();
    let dui = build_dui(s);
    let tree = match parse_dui(&dui) {
        Ok(t) => t,
        Err(_) => return,
    };
    let mut dss = String::new();
    dss.push_str("Column#win { background: #121820; padding: 10; }\n");
    dss.push_str("Text { color: #cdd6f4; font-size: 13; padding: 2; }\n");
    dss.push_str("Text#title { color: #2f8fbd; font-size: 18; padding: 4; }\n");
    let sheet = match parse_dss(&dss) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut cas = Cascade::new();
    cas.push(sheet);

    let style_of = |nid: NodeId| {
        let node = tree.node(nid);
        let tag = node.kind.tag();
        let ns = match node.name.as_deref() {
            Some(name) => NodeStyle {
                element: tag,
                id: Some(name),
                classes: &[],
                states: &[],
            },
            None => NodeStyle::element(tag),
        };
        cas.resolve(&ns)
    };

    let fm = FontMeasure { font };
    let measure_fn = |nid: NodeId| {
        let node = tree.node(nid);
        if let Kind::Element(tag) = &node.kind {
            if let Some(w) = Widget::from_tag(tag) {
                return intrinsic_size(w, node.text.as_deref(), &style_of(nid), &fm);
            }
        }
        (0.0, 0.0)
    };
    let lay = layout_measured(&tree, width as f32, height as f32, &measure_fn);

    let mut surface = Surface::new(frame.pixels_mut(), width as usize, height as usize);
    paint(&tree, &lay, &style_of, font, &mut surface);
}

#[dunit::main]
fn main() -> dunit::Result<()> {
    let font = load_font().map_err(|_| dunit::Error(dunit::errno::EIO))?;
    let mut window = Window::open(W, H)?;
    let mut stats = dunit::system::stats()?;
    window.draw(|frame| render_stats(frame, &font, &stats))?;
    dunit::println!("gui_stat: surface presented OK")?;
    let mut verified_refresh = false;
    loop {
        let previous_ticks = stats.uptime_ticks;
        let event = window.next_event(Duration::from_millis(POLL_MS))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        stats = dunit::system::stats()?;
        if event.is_none() && stats.uptime_ticks > previous_ticks && !verified_refresh {
            dunit::println!("gui_stat: SDK wait-set refresh OK")?;
            verified_refresh = true;
        }
        if event.is_none() || matches!(event, Some(WindowEvent::Resized { .. })) {
            window.draw(|frame| render_stats(frame, &font, &stats))?;
            if matches!(event, Some(WindowEvent::Resized { .. })) {
                dunit::println!("gui_stat: SDK reconfigured OK")?;
                if window.size() == (W, H) {
                    dunit::println!("gui_stat: SDK restored size=460x300")?;
                }
            }
        }
    }
    window.close()?;
    dunit::println!("gui_stat: SDK quit OK")?;
    Ok(())
}
