//! Owned capability mappings are staged/imported independently of the displayed
//! texture. Only the reference server's accepted committed frame selects pixels.
use alloc::vec::Vec;
use gui_protocol_v1::{frame::Frame, server::Server};

struct Mapping {
    object: u64,
    handle: u32,
    address: usize,
    bytes: usize,
}
impl Drop for Mapping {
    fn drop(&mut self) {
        libdunit::shared_buffer::release(self.handle, self.address, self.bytes);
    }
}
pub struct Buffers {
    announced: Option<Mapping>,
    imports: Vec<Mapping>,
    pixels: Vec<u32>,
    pub frame: Option<Frame>,
    pub copied: u64,
}
impl Buffers {
    pub const fn new() -> Self {
        Self {
            announced: None,
            imports: Vec::new(),
            pixels: Vec::new(),
            frame: None,
            copied: 0,
        }
    }
    pub fn announce(&mut self, object: u64, handle: u32) {
        if self.imports.iter().any(|m| m.handle == handle)
            || self.announced.as_ref().is_some_and(|m| m.handle == handle)
        {
            return;
        }
        let bytes = libdunit::handle_shared_len(handle);
        if bytes <= 0 {
            return;
        }
        let address = libdunit::handle_map(handle, 0, bytes as usize);
        if address <= 0 {
            libdunit::handle_close(handle);
            return;
        }
        self.announced = Some(Mapping {
            object,
            handle,
            address: address as usize,
            bytes: bytes as usize,
        });
    }
    pub fn capability(&self, object: u64) -> Option<u64> {
        self.announced
            .as_ref()
            .filter(|m| m.object == object)
            .map(|m| m.bytes as u64)
    }
    pub fn import(&mut self, object: u64, accepted: bool) {
        if let Some(mapping) = self.announced.take() {
            if accepted && mapping.object == object {
                self.imports.push(mapping);
            }
        }
    }
    pub fn destroy(&mut self, object: u64) {
        self.imports.retain(|m| m.object != object);
    }
    pub fn clear(&mut self) {
        self.announced = None;
        self.imports.clear();
        self.frame = None;
        self.pixels.clear();
    }
    pub fn sync(&mut self, server: &Server, conn: u32, surface: u64) -> bool {
        let Some(frame) = server.frame(conn, surface) else {
            self.frame = None;
            return true;
        };
        let changed = self.frame.map_or(true, |old| {
            old.commit != frame.commit || old.buffer != frame.buffer
        });
        if !changed && !frame.damage.continuous {
            return true;
        }
        let Some(mapping) = self.imports.iter().find(|m| m.object == frame.buffer) else {
            return false;
        };
        let count = frame.width as usize * frame.height as usize;
        let full = self.frame.map_or(true, |old| {
            old.width != frame.width || old.height != frame.height || old.format != frame.format
        });
        if self.pixels.len() != count {
            if self
                .pixels
                .try_reserve(count.saturating_sub(self.pixels.len()))
                .is_err()
            {
                return false;
            }
            self.pixels.resize(count, 0);
        }
        let source =
            unsafe { core::slice::from_raw_parts(mapping.address as *const u8, mapping.bytes) };
        let Some(copied) = frame.copy_into(source, &mut self.pixels, full) else {
            return false;
        };
        self.copied = self.copied.saturating_add(copied);
        self.frame = Some(frame);
        true
    }
    pub fn image(&self) -> (*const u8, usize) {
        if self.frame.is_none() {
            (core::ptr::null(), 0)
        } else {
            (self.pixels.as_ptr() as *const u8, self.pixels.len() * 4)
        }
    }
}
