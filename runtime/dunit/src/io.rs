//! Stream helpers preserve partial transfers and buffered input between lines.
use crate::{error::checked, Error, Result};
use alloc::{string::String, vec::Vec};
use core::fmt;
// Deliberately bounded transfers, below the native per-call user-copy limit.
// Larger logical reads/writes are completed by Read/Write's existing loops.
pub(crate) const TRANSFER_CHUNK: usize = 4096;

pub trait Read {
    fn read(&mut self, output: &mut [u8]) -> Result<usize>;
    /// Premature EOF is EIO; partial data already read remains in output.
    fn read_exact(&mut self, mut output: &mut [u8]) -> Result<()> {
        while !output.is_empty() {
            let count = self.read(output)?;
            if count == 0 || count > output.len() {
                return Err(Error(libdunit::EIO));
            }
            output = &mut output[count..];
        }
        Ok(())
    }
    fn read_to_end(&mut self, output: &mut Vec<u8>) -> Result<usize> {
        let start = output.len();
        let mut chunk = [0; TRANSFER_CHUNK];
        loop {
            let count = self.read(&mut chunk)?;
            if count > chunk.len() {
                return Err(Error(libdunit::EIO));
            }
            if count == 0 {
                return Ok(output.len() - start);
            }
            output.extend_from_slice(&chunk[..count]);
        }
    }
}
pub trait Write {
    fn write(&mut self, bytes: &[u8]) -> Result<usize>;
    /// A zero-length write before completion is EIO, never silent success.
    fn write_all(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            let count = self.write(bytes)?;
            if count == 0 || count > bytes.len() {
                return Err(Error(libdunit::EIO));
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }
    fn write_fmt(&mut self, arguments: fmt::Arguments<'_>) -> Result<()> {
        struct Adapter<'a, W: Write + ?Sized> {
            writer: &'a mut W,
            error: Option<Error>,
        }
        impl<W: Write + ?Sized> fmt::Write for Adapter<'_, W> {
            fn write_str(&mut self, text: &str) -> fmt::Result {
                self.writer.write_all(text.as_bytes()).map_err(|error| {
                    self.error = Some(error);
                    fmt::Error
                })
            }
        }
        let mut adapter = Adapter {
            writer: self,
            error: None,
        };
        fmt::write(&mut adapter, arguments)
            .map_err(|_| adapter.error.unwrap_or(Error(libdunit::EIO)))
    }
}

/// Stateful line reader. Multiple lines from a single read are never discarded.
pub struct BufReader<R> {
    inner: R,
    buffer: [u8; 1024],
    position: usize,
    length: usize,
}
impl<R: Read> BufReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buffer: [0; 1024],
            position: 0,
            length: 0,
        }
    }
    /// Return a line without LF, or None at clean EOF. Invalid UTF-8 is EINVAL.
    pub fn read_line(&mut self) -> Result<Option<String>> {
        let mut bytes = Vec::new();
        loop {
            if self.position == self.length {
                self.length = self.inner.read(&mut self.buffer)?;
                self.position = 0;
                if self.length > self.buffer.len() {
                    return Err(Error(libdunit::EIO));
                }
                if self.length == 0 {
                    return if bytes.is_empty() {
                        Ok(None)
                    } else {
                        String::from_utf8(bytes)
                            .map(Some)
                            .map_err(|_| Error(libdunit::EINVAL))
                    };
                }
            }
            let rest = &self.buffer[self.position..self.length];
            if let Some(end) = rest.iter().position(|byte| *byte == b'\n') {
                bytes.extend_from_slice(&rest[..end]);
                self.position += end + 1;
                return String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| Error(libdunit::EINVAL));
            }
            bytes.extend_from_slice(rest);
            self.position = self.length;
        }
    }
}
impl<R: Read> Read for BufReader<R> {
    fn read(&mut self, output: &mut [u8]) -> Result<usize> {
        if self.position < self.length {
            let count = output.len().min(self.length - self.position);
            output[..count].copy_from_slice(&self.buffer[self.position..self.position + count]);
            self.position += count;
            Ok(count)
        } else {
            self.inner.read(output)
        }
    }
}

pub struct Stdin;
pub struct Stdout;
pub struct Stderr;
pub fn stdin() -> Stdin {
    Stdin
}
pub fn stdout() -> Stdout {
    Stdout
}
pub fn stderr() -> Stderr {
    Stderr
}
fn wait_ready(fd: u32, requested: u32) -> Result<()> {
    use libdunit::wait_set::*;
    let mut result = [Event::default()];
    wait(
        &[Interest::descriptor(fd, requested, 0)],
        &mut result,
        INFINITE,
    )
    .map_err(Error)?;
    Ok(())
}
impl Read for Stdin {
    fn read(&mut self, output: &mut [u8]) -> Result<usize> {
        loop {
            let count = output.len().min(TRANSFER_CHUNK);
            let result = libdunit::read(0, &mut output[..count]);
            if result != libdunit::EAGAIN {
                return checked(result);
            }
            wait_ready(0, libdunit::wait_set::READ)?;
        }
    }
}
fn write_stream(fd: u32, bytes: &[u8]) -> Result<usize> {
    loop {
        let result = libdunit::write(fd as usize, &bytes[..bytes.len().min(TRANSFER_CHUNK)]);
        if result != libdunit::EAGAIN {
            return checked(result);
        }
        wait_ready(fd, libdunit::wait_set::WRITE)?;
    }
}
impl Write for Stdout {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        write_stream(1, bytes)
    }
}
impl Write for Stderr {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        write_stream(2, bytes)
    }
}
pub fn print(arguments: fmt::Arguments<'_>) -> Result<()> {
    stdout().write_fmt(arguments)
}
