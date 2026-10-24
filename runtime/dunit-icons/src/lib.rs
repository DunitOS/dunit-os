//! Bounded ZIP access and freedesktop icon-theme lookup, independent of the VFS.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
pub mod archive;
pub mod theme;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Io,
    Invalid,
    Limit,
    Unsupported,
    Missing,
}
pub type Result<T> = core::result::Result<T, Error>;

pub fn safe_path(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 512
        && !s.contains(['\\', '\0', ':'])
        && s.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
