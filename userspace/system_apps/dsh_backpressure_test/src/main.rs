#![no_std]
#![no_main]
extern crate alloc;

use alloc::vec::Vec;
use dunit::{
    io::{Read, Write},
    pty::Pty,
    time::{self, Duration, Instant},
    Error, Result,
};

// Capture only a bounded suffix while draining the shell's cooked echo.
fn drain(pty: &mut Pty, tail: &mut Vec<u8>) -> Result<bool> {
    let mut buffer = [0; 512];
    let mut progressed = false;
    for _ in 0..32 {
        match pty.read(&mut buffer) {
            Ok(0) | Err(Error(dunit::errno::EAGAIN)) => break,
            Ok(n) => {
                progressed = true;
                tail.extend_from_slice(&buffer[..n]);
                if tail.len() > 256 {
                    tail.drain(..tail.len() - 256);
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(progressed)
}

fn pause(start: Instant) -> Result<()> {
    if start.elapsed()? >= Duration::from_secs(60) {
        return Err(Error(dunit::errno::EIO));
    }
    time::sleep(Duration::from_millis(1))
}

fn send(pty: &mut Pty, tail: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let start = Instant::now()?;
    let mut sent = 0;
    while sent < bytes.len() {
        let mut progressed = drain(pty, tail)?;
        match pty.write(&bytes[sent..bytes.len().min(sent + 128)]) {
            Ok(n) => {
                sent += n;
                progressed |= n != 0;
            }
            Err(Error(dunit::errno::EAGAIN)) => {}
            Err(error) => return Err(error),
        }
        if start.elapsed()? >= Duration::from_secs(60) {
            return Err(Error(dunit::errno::EIO));
        }
        if !progressed {
            pause(start)?;
        }
    }
    Ok(())
}

fn wait(pty: &mut Pty, tail: &mut Vec<u8>, needle: &[u8]) -> Result<()> {
    let start = Instant::now()?;
    loop {
        drain(pty, tail)?;
        if tail.windows(needle.len()).any(|bytes| bytes == needle) {
            return Ok(());
        }
        pause(start)?;
    }
}

fn exercise(pty: &mut Pty) -> Result<()> {
    let mut tail = Vec::new();
    wait(pty, &mut tail, b"dsh ")?;
    send(pty, &mut tail, b"kill_target\n")?;
    // Let the shell submit the command and enter its cooked child bridge.
    time::sleep(Duration::from_millis(100))?;
    let mut lines = alloc::vec![b'x'; 8194];
    lines[4096] = b'\n';
    lines[8193] = b'\n';
    // The child never reads. Two valid 4096-byte lines plus LF exceed its
    // 8192-byte ring; the pending suffix must not hide terminal control input.
    send(pty, &mut tail, &lines)?;
    tail.clear();
    send(pty, &mut tail, b"\x1b[\x03")?;
    wait(pty, &mut tail, b"dsh ")?;
    tail.clear();
    send(pty, &mut tail, b"echo BACKPRESSURE_RECOVERED\n")?;
    wait(pty, &mut tail, b"\nBACKPRESSURE_RECOVERED\n")?;
    Ok(())
}

#[dunit::main]
fn main() -> Result<()> {
    let mut pty = Pty::open()?;
    let mut shell = pty.spawn("/app/dsh", &["dsh"], &[])?;
    let result = exercise(&mut pty);
    let reaped = shell.terminate_and_wait();
    let closed = pty.close();
    result?;
    reaped?;
    closed?;
    dunit::println!(
        "[DSH-PRESSURE] full child ring, interrupted CSI, Ctrl-C and shell recovery OK"
    )?;
    Ok(())
}
