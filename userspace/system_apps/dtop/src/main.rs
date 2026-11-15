#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use dunit::io::Write;
use dunit::time::{self, Duration};

#[dunit::main]
fn main() -> dunit::Result<()> {
    let args = dunit::args::args()?;
    let once = match args.get(1).map(|arg| arg.as_str()) {
        None => false,
        Some("--once") if args.len() == 2 => true,
        _ => {
            dunit::println!("Usage: dtop [--once]")?;
            return Err(dunit::Error(dunit::errno::EINVAL));
        }
    };
    let mut output = dunit::io::stdout();
    if once {
        return draw(&mut output);
    }
    output.write_all(b"\x1b[2J\x1b[H\x1b[?25l")?;
    let result = monitor(&mut output);
    let restore = output.write_all(b"\x1b[?25h\x1b[2J\x1b[H");
    result.and(restore)
}

fn monitor(output: &mut impl Write) -> dunit::Result<()> {
    loop {
        output.write_all(b"\x1b[H")?;
        draw(output)?;
        if libdunit::get_key().is_some() {
            return Ok(());
        }
        time::sleep(Duration::from_millis(500))?;
    }
}

fn draw(output: &mut impl Write) -> dunit::Result<()> {
    let stats = dunit::system::stats()?;
    output.write_all(b"DUNIT TOP - System Monitor\nCPU unavailable (no usage counter)\n")?;
    let memory = format!(
        "MEM {} MiB / {} MiB ({}%)\n\n",
        stats.pmm_used_bytes / 1024 / 1024,
        stats.pmm_total_bytes / 1024 / 1024,
        stats.memory_used_percent(),
    );
    output.write_all(memory.as_bytes())?;
    let processes = format!(
        "Processes: {} total, {} ready, {} running, {} blocked, {} dead\n",
        stats.process_total, stats.process_ready, stats.process_running,
        stats.process_blocked, stats.process_dead,
    );
    output.write_all(processes.as_bytes())
}
