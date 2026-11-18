//! Exclusive native display lease. Drawing requires this owned capability;
//! framebuffer addresses remain private to the SDK and kernel.
use crate::{error::checked, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub struct Display {
    handle: u32,
}
impl Display {
    /// Acquire a display-master lease (EBUSY if another process owns it).
    pub fn acquire() -> Result<Self> {
        let handle = checked(libdunit::handle_display_acquire())?;
        if handle == 0 || handle > u32::MAX as usize {
            return Err(crate::Error(libdunit::EIO));
        }
        Ok(Self {
            handle: handle as u32,
        })
    }
    pub fn dimensions(&self) -> Result<Dimensions> {
        let mut info = libdunit::FbInfo {
            addr: 0,
            width: 0,
            height: 0,
            pitch: 0,
        };
        checked(libdunit::syscall1(
            libdunit::SYSCALL_GET_FRAMEBUFFER,
            &mut info as *mut _ as usize,
        ))?;
        Ok(Dimensions {
            width: info.width,
            height: info.height,
        })
    }
    /// Present packed native RGB pixels through the existing copy-to-display
    /// syscall. The kernel clips at scanout edges and enforces lease ownership.
    pub fn present(
        &mut self,
        pixels: &[u32],
        width: u32,
        height: u32,
        x: u32,
        y: u32,
    ) -> Result<()> {
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or(crate::Error(libdunit::EINVAL))?;
        let bytes = count
            .checked_mul(core::mem::size_of::<u32>())
            .ok_or(crate::Error(libdunit::EINVAL))?;
        if width == 0 || height == 0 || count != pixels.len() || bytes > 64 * 1024 * 1024 {
            return Err(crate::Error(libdunit::EINVAL));
        }
        checked(libdunit::syscall5(
            libdunit::SYSCALL_FB_PRESENT,
            pixels.as_ptr() as usize,
            width as usize,
            height as usize,
            x as usize,
            y as usize,
        ))
        .map(|_| ())
    }
    pub fn draw_pixel(&mut self, x: u32, y: u32, color: u32) -> Result<()> {
        checked(libdunit::syscall3(
            libdunit::SYSCALL_DRAW_PIXEL,
            x as usize,
            y as usize,
            color as usize,
        ))
        .map(|_| ())
    }
    pub fn draw_rect(&mut self, x: u32, y: u32, width: u32, height: u32, color: u32) -> Result<()> {
        checked(libdunit::syscall5(
            libdunit::SYSCALL_DRAW_RECT,
            x as usize,
            y as usize,
            width as usize,
            height as usize,
            color as usize,
        ))
        .map(|_| ())
    }
}
impl Drop for Display {
    fn drop(&mut self) {
        libdunit::handle_close(self.handle);
    }
}
