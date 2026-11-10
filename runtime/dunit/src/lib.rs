//! Native Dunit SDK: no_std + alloc over libdunit.
//! Retained components, native windows and shared canvas resources use `gui`.
#![no_std]
extern crate alloc;
pub mod args;
#[cfg(feature = "gui")]
pub mod canvas;
pub mod env;
mod error;
pub mod event;
pub mod fs;
pub mod io;
pub mod process;
pub mod pty;
pub mod system;
pub mod testing;
#[cfg(feature = "gui")]
pub mod text_clipboard;
pub mod time;
#[cfg(feature = "gui")]
pub mod ui;
#[cfg(feature = "gui")]
mod ui_theme;
#[cfg(feature = "gui")]
pub mod window;
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

#[cfg(feature = "gui")]
pub use dunit_transfer as transfer;
