//! Owned native PTY masters and bounded input buffering over the native runtime.
use crate::{
    error::checked,
    io::{Read, Write},
    process::Child,
    Error, Result,
};
pub struct Pty {
    id: Option<u32>,
}
impl Pty {
    pub fn open() -> Result<Self> {
        let id = checked(libdunit::pty_create())?;
        if id == 0 {
            return Err(Error(libdunit::EIO));
        }
        Ok(Self {
            id: Some(id as u32),
        })
    }
    pub fn spawn(&self, path: &str, args: &[&str], env: &[&str]) -> Result<Child> {
        Child::from_spawned(libdunit::process::pty_spawn_args(
            path,
            self.id.ok_or(Error(libdunit::EBADF))?,
            args,
            env,
        ))
    }
    pub fn close(mut self) -> Result<()> {
        self.close_inner()
    }
    fn close_inner(&mut self) -> Result<()> {
        if let Some(id) = self.id.take() {
            checked(libdunit::pty_close(id))?;
        }
        Ok(())
    }
}
impl Read for Pty {
    fn read(&mut self, output: &mut [u8]) -> Result<usize> {
        checked(libdunit::pty_read(
            self.id.ok_or(Error(libdunit::EBADF))?,
            output,
        ))
    }
}
impl Write for Pty {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        checked(libdunit::pty_write(
            self.id.ok_or(Error(libdunit::EBADF))?,
            bytes,
        ))
    }
}
impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.close_inner();
    }
}

// Bounded, lossless PTY input. Partial writes retain the remaining suffix.

use alloc::vec::Vec;
pub struct PendingInput {
    bytes: Vec<u8>,
    offset: usize,
    limit: usize,
}
impl PendingInput {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            offset: 0,
            limit,
        }
    }
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.offset = 0;
    }
    pub fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
    pub fn push(&mut self, bytes: &[u8]) -> bool {
        if self.bytes.len() - self.offset + bytes.len() > self.limit {
            return false;
        }
        if self.offset != 0 {
            self.bytes.drain(..self.offset);
            self.offset = 0;
        }
        self.bytes.extend_from_slice(bytes);
        true
    }
    pub fn drain(&mut self, mut write: impl FnMut(&[u8]) -> Result<usize>) -> Result<()> {
        // Bound service time so output and the compositor inbox continue to run.
        for _ in 0..8 {
            if self.is_empty() {
                self.bytes.clear();
                self.offset = 0;
                break;
            }
            let end = (self.offset + 1024).min(self.bytes.len());
            let n = write(&self.bytes[self.offset..end])?;
            if n == 0 {
                break;
            }
            if n > end - self.offset {
                return Err(Error(libdunit::EIO));
            }
            self.offset += n;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suffix_survives_short_write_blocking_and_later_input() {
        let mut q = PendingInput::new(8);
        assert!(q.push(b"abcd"));
        let mut calls = 0;
        assert_eq!(
            q.drain(|_| {
                calls += 1;
                if calls == 1 {
                    Ok(2)
                } else {
                    Err(Error(libdunit::EAGAIN))
                }
            }),
            Err(Error(libdunit::EAGAIN))
        );
        assert!(q.push(b"ef"));
        assert!(!q.push(b"overflow"));
        let mut out = Vec::new();
        q.drain(|bytes| {
            out.extend_from_slice(bytes);
            Ok(bytes.len())
        })
        .unwrap();
        assert_eq!(out, b"cdef");
        assert!(q.is_empty());
        assert!(q.push(b"x"));
        q.drain(|_| Ok(0)).unwrap();
        assert!(!q.is_empty());
        q.clear();
        assert!(q.is_empty());
    }
}

use libdunit::KEYMOD_CTRL;

// PS/2 scancodes (sc & 0x7F) for keys the kernel leaves un-cooked (ascii = 0).
const SC_BACKSPACE: u8 = 0x0E;
const SC_HOME: u8 = 0x47;
const SC_UP: u8 = 0x48;
const SC_LEFT: u8 = 0x4B;
const SC_RIGHT: u8 = 0x4D;
const SC_END: u8 = 0x4F;
const SC_DOWN: u8 = 0x50;
const SC_DELETE: u8 = 0x53;
/// Translate a widened IN_KEY event (scancode + mods + cooked ASCII) into the
/// byte sequence to feed the pty. Returns the number of bytes written to `out`.
///
/// - Ctrl + letter -> the control code (`c & 0x1F`): Ctrl-C=0x03, Ctrl-D=0x04,
///   Ctrl-L=0x0C, etc.
/// - Any other cooked byte (printable, Enter `\n`, Tab `\t`) -> itself.
/// - Backspace (no ASCII) -> 0x08.
/// - Arrows / Home / End / Delete (no ASCII) -> the usual xterm ESC sequences,
///   which `dsh` parses for line editing and history.
pub fn encode_key(scancode: u8, mods: u8, ascii: u8, out: &mut [u8; 4]) -> usize {
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
