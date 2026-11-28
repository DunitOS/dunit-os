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
    #[cfg(feature = "boot-smoke-tests")]
    let mut probe = PresentProbe::new();
    let dirty_rect = Some((dst_x, dst_y, width as u32, height as u32));
    let result = driver::present(pid, dirty_rect, |framebuffer| {
        #[cfg(feature = "boot-smoke-tests")]
        { probe.start = crate::clock::pit_interrupts(); }
        // Preserve validation of the entire supplied image, including clipped
        // pixels. Each pin covers at most one source page, never a whole frame.
        let mut checked = 0;
        while checked < bytes {
            let address = src as u64 + checked as u64;
            let count = (4096 - (address as usize & 4095)).min(bytes - checked);
            user_memory::validate_user_pages(address, count, false)?;
            checked += count;
        }
        let (x, y) = (dst_x as usize, dst_y as usize);
        let copy_width = width.min((framebuffer.width as usize).saturating_sub(x));
        let copy_height = height.min((framebuffer.height as usize).saturating_sub(y));
        // Kernel-owned staging outlives each mapping guard. Device writes run
        // after the guard restores IF; ACCESS keeps the display lease/geometry
        // stable across every chunk without masking IRQs. A concurrent unmap
        // can return EFAULT after earlier chunks reached scanout; present does
        // not promise transactional frame contents.
        let mut staging = [0u32; 1024];
        for row in 0..copy_height {
            let mut column = 0;
            while column < copy_width {
                let count = staging.len().min(copy_width - column);
                let address = src as u64 + ((row * width + column) * 4) as u64;
                user_memory::with_mapping(address, count * 4, false, |mapping| {
                    unsafe {
                        mapping.copy_words_volatile(0, staging.as_mut_ptr(), count);
                    }
                    Ok(())
                })?;
                #[cfg(feature = "boot-smoke-tests")]
                probe.chunk(count * 4);
                let offset = (y + row) * framebuffer.pitch as usize + (x + column) * 4;
                let destination =
                    unsafe { (framebuffer.address as *mut u8).add(offset) } as *mut u32;
                let mut i = 0usize;
                if (destination as usize) & 3 == 0 {
                    if count > 0 && (destination as usize) & 7 != 0 {
                        unsafe {
                            destination.write_volatile(staging[0]);
                        }
                        i = 1;
                    }
                    while i + 1 < count {
                        let word = (staging[i] as u64) | ((staging[i + 1] as u64) << 32);
                        unsafe {
                            (destination.add(i) as *mut u64).write_volatile(word);
                        }
                        i += 2;
                    }
                    if i < count {
                        unsafe {
                            destination.add(i).write_volatile(staging[i]);
                        }
                    }
                } else {
                    for (index, pixel) in staging[..count].iter().enumerate() {
                        unsafe {
                            destination.add(index).write_volatile(*pixel);
                        }
                    }
                }
                column += count;
            }
        }
        Ok::<(), i64>(())
    })
    .map_err(display_errno)
    .and_then(|result| result)
    .map(|()| 0)
    .unwrap_or_else(|error| error);
    #[cfg(feature = "boot-smoke-tests")]
    if result == 0 && bytes >= 1024 * 1024 {
        probe.report();
    }
    result
}

// Private feature-only evidence from real presents; no public ABI or hot-path
// output in production. Report only after all source/device guards have ended.
#[cfg(feature = "boot-smoke-tests")]
struct PresentProbe {
    start: u64,
    max_bytes: usize,
    enabled_chunks: usize,
    chunks: usize,
}
#[cfg(feature = "boot-smoke-tests")]
impl PresentProbe {
    fn new() -> Self {
        Self {
            start: crate::clock::pit_interrupts(),
            max_bytes: 0,
            enabled_chunks: 0,
            chunks: 0,
        }
    }
    fn chunk(&mut self, bytes: usize) {
        self.max_bytes = self.max_bytes.max(bytes);
        self.chunks += 1;
        if crate::sync::interrupts_enabled() {
            self.enabled_chunks += 1;
        }
    }
    fn report(self) {
        use core::sync::atomic::{AtomicBool, Ordering};
        static REPORTED: AtomicBool = AtomicBool::new(false);
        let ticks = crate::clock::pit_interrupts().wrapping_sub(self.start);
        if ticks == 0 || REPORTED.swap(true, Ordering::Relaxed) {
            return;
        }
        crate::serial_write("[FB-PRESENT] mapping_max_bytes=");
        crate::serial::write_dec(self.max_bytes as u64);
        crate::serial_write(" chunks=");
        crate::serial::write_dec(self.chunks as u64);
        crate::serial_write(" irq_enabled_chunks=");
        crate::serial::write_dec(self.enabled_chunks as u64);
        crate::serial_write(" pit_ticks=");
        crate::serial::write_dec(ticks);
        crate::serial_write("\r\n");
    }
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
