//! Native Dunit SDK: no_std + alloc over libdunit.
//! Declarative components and cargo tooling remain unfinished M7 layers.
#![no_std]
extern crate alloc;
pub mod args;
pub mod env;
mod error;
pub mod event;
pub mod fs;
pub mod io;
pub mod process;
pub mod system;
pub mod time;
pub use dunit_macros::main;
pub use error::{Error, Result};
pub use libdunit::errno_numbers as errno;
#[doc(hidden)]
#[path = "entry.rs"]
pub mod __private;

/// Formatted stdout write. Returns a Result so callers can propagate I/O errors.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::io::print(::core::format_args!($($arg)*)) };
}
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::io::print(::core::format_args!("{}\n", ::core::format_args!($($arg)*))) };
}
