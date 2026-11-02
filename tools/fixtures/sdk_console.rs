extern crate alloc;
use dunit::io::{Read, Write};

#[dunit::main]
fn main() -> dunit::Result<()> {
    let args = dunit::args::args()?;
    assert_eq!(args.len(), 1);
    let cli = args[0] == "/app/console command";
    assert!(cli || args[0] == "/app/sdk_console_tui");
    let kind = if cli { "cli" } else { "tui" };
    if !cli {
        dunit::io::stdout().write_all(b"\x1b[2J\x1b[H\x1b[32mTUI console\x1b[0m\n")?;
    }
    dunit::println!("[CONSOLE] {} ready", kind)?;
    let mut input = dunit::io::stdin();
    let mut line = alloc::vec::Vec::new();
    loop {
        let mut byte = [0];
        assert_eq!(input.read(&mut byte)?, 1);
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
        assert!(line.len() < 32);
    }
    assert_eq!(line, b"hello");
    dunit::io::stderr().write_all(b"console stderr visible\n")?;
    dunit::println!("[CONSOLE] {} stdin stdout argv OK", kind)?;
    if !cli {
        // The parent must terminate and reap this live child on window close.
        let mut byte = [0];
        input.read(&mut byte)?;
        panic!("TUI should have been terminated on window close");
    }
    Ok(())
}
