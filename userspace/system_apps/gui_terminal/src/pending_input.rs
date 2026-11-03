//! Bounded, lossless PTY input. Partial writes retain the exact remaining suffix.
extern crate alloc;
use alloc::vec::Vec;
pub(crate) struct PendingInput {
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
    pub fn drain(&mut self, mut write: impl FnMut(&[u8]) -> isize) -> Result<(), isize> {
        // Bound service time so output and the compositor inbox continue to run.
        for _ in 0..8 {
            if self.is_empty() {
                self.bytes.clear();
                self.offset = 0;
                break;
            }
            let end = (self.offset + 1024).min(self.bytes.len());
            let n = write(&self.bytes[self.offset..end]);
            if n < 0 {
                return Err(n);
            }
            if n == 0 {
                break;
            }
            if n as usize > end - self.offset {
                return Err(-5);
            }
            self.offset += n as usize;
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
                    2
                } else {
                    -11
                }
            }),
            Err(-11)
        );
        assert!(q.push(b"ef"));
        assert!(!q.push(b"overflow"));
        let mut out = Vec::new();
        q.drain(|bytes| {
            out.extend_from_slice(bytes);
            bytes.len() as isize
        })
        .unwrap();
        assert_eq!(out, b"cdef");
        assert!(q.is_empty());
        assert!(q.push(b"x"));
        q.drain(|_| 0).unwrap();
        assert!(!q.is_empty());
        q.clear();
        assert!(q.is_empty());
    }
}
