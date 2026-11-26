//! Shared pure-Rust (`no_std + alloc`) editor buffer, undo/redo, search/replace,
//! SHA-256 content hashing, and Markdown parser shared by `rock` TUI, `gui_editor`,
//! and `gui_reader`.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod buffer;
pub mod hash;
pub mod markdown;
pub mod syntax;

#[cfg(test)]
mod tests;

pub use buffer::{CursorPos, DocumentBuffer, EditorError, ShortcutHint, SHORTCUT_HINTS};
pub use hash::{fnv1a_64, sha256, sha256_hex};
pub use markdown::{parse_markdown, DocBlock};
pub use syntax::{compute_line_states, highlight_line, HighlightSpan, Language, LineState, TokenKind};
