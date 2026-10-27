#![no_std]
#![no_main]

//! Untrusted System Monitor: typed component state/view over the native SDK.
extern crate alloc;
use alloc::{string::String, vec};
use dunit::ui::{Component, ComponentWindow, View};
use dunit::{time::Duration, window::WindowEvent};

const W: u32 = 460;
const H: u32 = 300;
const POLL_MS: u64 = 500;

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

struct Monitor {
    stats: dunit::system::SystemStats,
}
#[derive(Clone)]
enum Msg {
    Refresh(dunit::system::SystemStats),
}
impl Component for Monitor {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        let Msg::Refresh(stats) = message;
        self.stats = stats;
    }
    fn view(&self) -> View<Msg> {
        let s = &self.stats;
        View::column(
            "win",
            vec![
                View::text("title", "System Stats"),
                View::text(
                    "processes",
                    alloc::format!(
                        "proc: total {} run {} ready {} blocked {}",
                        s.process_total,
                        s.process_running,
                        s.process_ready,
                        s.process_blocked
                    ),
                ),
                View::text(
                    "exits",
                    alloc::format!("dead {} reaped {}", s.process_dead, s.process_reaped),
                ),
                View::text(
                    "pmm",
                    alloc::format!(
                        "pmm: {} / {}",
                        fmt_size(s.pmm_used_bytes),
                        fmt_size(s.pmm_total_bytes)
                    ),
                ),
                View::text(
                    "heap",
                    alloc::format!(
                        "heap: {} used  {} free-blocks",
                        fmt_size(s.heap_used_bytes),
                        s.heap_free_blocks
                    ),
                ),
                View::text(
                    "ipc",
                    alloc::format!(
                        "ipc: queues {} msgs {} shared {}",
                        s.ipc_queue_count,
                        s.ipc_queued_messages,
                        s.ipc_shared_regions
                    ),
                ),
                View::text(
                    "files",
                    alloc::format!(
                        "fs: files {} dirs {} {}",
                        s.fs_files,
                        s.fs_directories,
                        fmt_size(s.fs_bytes)
                    ),
                ),
                View::text(
                    "network",
                    alloc::format!(
                        "net: nics {} supported {}",
                        s.net_total_nics,
                        s.net_supported_nics
                    ),
                ),
                View::text(
                    "uptime",
                    alloc::format!("uptime: {}s", s.uptime_ticks / 100),
                ),
            ],
        )
    }
}

#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        W,
        H,
        Monitor {
            stats: dunit::system::stats()?,
        },
    )?;
    dunit::println!("gui_stat: surface presented OK")?;
    dunit::println!("gui_stat: typed component presented OK")?;
    let mut verified_refresh = false;
    loop {
        let previous_ticks = ui.component().stats.uptime_ticks;
        let event = ui.next_event(Duration::from_millis(POLL_MS))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        if event.is_none() || matches!(event, Some(WindowEvent::Resized { .. })) {
            ui.update(Msg::Refresh(dunit::system::stats()?))?;
            if event.is_none()
                && ui.component().stats.uptime_ticks > previous_ticks
                && !verified_refresh
            {
                dunit::println!("gui_stat: SDK wait-set refresh OK")?;
                verified_refresh = true;
            }
            if matches!(event, Some(WindowEvent::Resized { .. })) {
                dunit::println!("gui_stat: SDK reconfigured OK")?;
                if ui.size() == (W, H) {
                    dunit::println!("gui_stat: SDK restored size=460x300")?;
                }
            }
        }
    }
    ui.close()?;
    dunit::println!("gui_stat: SDK quit OK")?;
    Ok(())
}
