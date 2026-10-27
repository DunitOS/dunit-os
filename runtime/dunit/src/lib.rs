//! Native Dunit SDK: no_std + alloc over libdunit.
//! Retained components are available with `gui`; widgets and distribution are unfinished.
#![no_std]
extern crate alloc;
pub mod args;
pub mod env;
mod error;
pub mod event;
pub mod fs;
#[cfg(feature = "gui")]
pub mod window;
#[cfg(feature = "gui")]
pub mod ui;
#[cfg(feature = "gui")]
mod ui_theme;
pub mod io;
pub mod process;
pub mod system;
pub mod testing;
pub mod time;
pub use dunit_macros::{main, test};
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
