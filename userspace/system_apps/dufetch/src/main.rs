#![no_std]
#![no_main]

//! One report for the recovery terminal and the GUI terminal's ordinary PTY.
//! Only existing runtime facts are shown; platform identity is not invented.
mod report;

#[dunit::main]
fn main() -> dunit::Result<()> {
    let stats = dunit::system::stats()?;
    let display = dunit::system::display_info();
    let pid = dunit::process::id();
    let cwd = dunit::fs::current_dir()?;
    report::render(&stats, display, pid, &cwd, &mut dunit::io::stdout())
}
