//! Native terminal cursor geometry and nonblocking keyboard-ring access.
use crate::{error::checked, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub x: u32,
    pub y: u32,
    pub char_width: u32,
    pub char_height: u32,
}
pub fn cursor() -> Result<Cursor> {
    let mut info = libdunit::TerminalCursorInfo {
        x: 0,
        y: 0,
        char_width: 0,
        char_height: 0,
    };
    checked(libdunit::syscall1(
        libdunit::SYSCALL_GET_TERMINAL_CURSOR,
        &mut info as *mut _ as usize,
    ))?;
    Ok(Cursor {
        x: info.x,
        y: info.y,
        char_width: info.char_width,
        char_height: info.char_height,
    })
}
/// Return one raw scancode, including key releases, or None if the ring is empty.
pub fn read_scancode() -> Option<u8> {
    libdunit::get_key()
}
