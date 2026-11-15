//! Display ABI adapters. User mapping validation precedes scanout ownership.
use super::*;
use crate::drivers::display::{self as driver, DisplayError};

#[repr(C)]
pub struct FbInfo {
    pub addr: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}

fn display_errno(error: DisplayError) -> i64 {
    match error {
        DisplayError::Unsupported => EOPNOTSUPP,
        DisplayError::InvalidMode | DisplayError::Unavailable => EINVAL,
        DisplayError::Io | DisplayError::DeviceLost => EIO,
        DisplayError::AccessDenied => EACCES,
    }
}

fn caller_pid() -> Result<u64, i64> {
    crate::process::with_current_process(|process| process.pid.0).ok_or(EINVAL)
}

pub(super) fn sys_get_framebuffer(info: *mut FbInfo) -> i64 {
    let pid = match caller_pid() {
        Ok(pid) => pid,
        Err(error) => return error,
    };
    let framebuffer = match driver::get_framebuffer(pid) {
        Ok(framebuffer) => framebuffer,
        Err(error) => return display_errno(error),
    };
    // Preserve the 24-byte native record, initializing its trailing C padding.
    const _: () = {
        assert!(core::mem::size_of::<FbInfo>() == 24);
        assert!(core::mem::offset_of!(FbInfo, addr) == 0);
        assert!(core::mem::offset_of!(FbInfo, width) == 8);
        assert!(core::mem::offset_of!(FbInfo, height) == 12);
        assert!(core::mem::offset_of!(FbInfo, pitch) == 16);
    };
    let mut bytes = [0u8; 24];
    bytes[..8].copy_from_slice(&(framebuffer.address as u64).to_le_bytes());
    bytes[8..12].copy_from_slice(&framebuffer.width.to_le_bytes());
    bytes[12..16].copy_from_slice(&framebuffer.height.to_le_bytes());
    bytes[16..20].copy_from_slice(&framebuffer.pitch.to_le_bytes());
    copy_buffer_to_user(info as *mut u8, &bytes)
        .map(|()| 0)
        .unwrap_or_else(|error| error)
}

pub(super) fn sys_get_terminal_cursor(info: *mut TerminalCursorInfo) -> i64 {
    let Some(cursor) = crate::terminal::get_cursor_info() else {
        return EINVAL;
    };
    let user_info = TerminalCursorInfo {
        x: cursor.x,
        y: cursor.y,
        char_width: cursor.char_width,
        char_height: cursor.char_height,
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            &user_info as *const TerminalCursorInfo as *const u8,
            core::mem::size_of::<TerminalCursorInfo>(),
        )
    };
    if let Err(error) = copy_buffer_to_user(info as *mut u8, bytes) {
        return error;
    }
    0
}

pub(super) fn sys_draw_pixel(x: u32, y: u32, color: u32) -> i64 {
    let pid = match caller_pid() {
        Ok(pid) => pid,
        Err(error) => return error,
    };
    driver::with_framebuffer(pid, |framebuffer| {
        if x < framebuffer.width && y < framebuffer.height {
            let offset = y as usize * (framebuffer.pitch as usize / 4) + x as usize;
            unsafe {
                (framebuffer.address as *mut u32)
                    .add(offset)
                    .write_volatile(color);
            }
        }
    })
    .map(|()| 0)
    .unwrap_or_else(display_errno)
}

pub(super) fn sys_draw_rect(x: u32, y: u32, width: u32, height: u32, color: u32) -> i64 {
    let pid = match caller_pid() {
        Ok(pid) => pid,
        Err(error) => return error,
    };
    driver::with_framebuffer(pid, |framebuffer| {
        let width = width.min(framebuffer.width.saturating_sub(x));
        let height = height.min(framebuffer.height.saturating_sub(y));
        let stride = framebuffer.pitch as usize / 4;
        for row in 0..height as usize {
            let offset = (y as usize + row) * stride + x as usize;
            for column in 0..width as usize {
                unsafe {
                    (framebuffer.address as *mut u32)
                        .add(offset + column)
                        .write_volatile(color);
                }
            }
        }
    })
    .map(|()| 0)
    .unwrap_or_else(display_errno)
}

pub(super) fn sys_fb_present(
    src: *const u8,
    width: u32,
    height: u32,
    dst_x: u32,
    dst_y: u32,
) -> i64 {
    let pid = match caller_pid() {
        Ok(pid) => pid,
        Err(error) => return error,
    };
    if pid == 0 || driver::owner() != pid {
        return EACCES;
    }
    if driver::framebuffer_snapshot().is_none() {
        return EINVAL;
    }
    let (width, height) = (width as usize, height as usize);
    let Some(bytes) = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        return EINVAL;
    };
    const MAX_FB_PRESENT: usize = 64 * 1024 * 1024;
    if bytes == 0 || bytes > MAX_FB_PRESENT {
        return EINVAL;
    }
    if !is_valid_user_pointer(src as u64, bytes) {
        return EFAULT;
    }
    // VM -> display, never display -> VM. The mapping pin spans validation,
    // copying and flush; display ownership is revalidated after that pin.
    user_memory::with_mapping(src as u64, bytes, false, |mapping| {
        driver::present(pid, None, |framebuffer| {
            let (x, y) = (dst_x as usize, dst_y as usize);
            let copy_width = width.min((framebuffer.width as usize).saturating_sub(x));
            let copy_height = height.min((framebuffer.height as usize).saturating_sub(y));
            if copy_width == 0 {
                return;
            }
            for row in 0..copy_height {
                let offset = (y + row) * framebuffer.pitch as usize + x * 4;
                let destination =
                    unsafe { (framebuffer.address as *mut u8).add(offset) } as *mut u32;
                unsafe {
                    mapping.copy_words_volatile(row * width * 4, destination, copy_width);
                }
            }
        })
        .map_err(display_errno)
    })
    .map(|()| 0)
    .unwrap_or_else(|error| error)
}

pub(super) fn sys_set_video_mode(width: u32, height: u32) -> i64 {
    let pid = match caller_pid() {
        Ok(pid) => pid,
        Err(error) => return error,
    };
    driver::set_mode(pid, width, height)
        .map(|()| 0)
        .unwrap_or_else(display_errno)
}
