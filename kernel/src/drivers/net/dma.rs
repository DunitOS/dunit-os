//! Private contiguous DMA backing for the E1000 bring-up path.
//! Publication transfers access to hardware; Drop cannot free published pages.
use crate::memory::{
    pmm::{get_pmm, PhysicalAddress},
    vmm,
};

pub(super) struct DmaRegion {
    physical: usize,
    virtual_address: usize,
    pages: usize,
    published: bool,
}
impl DmaRegion {
    pub(super) fn allocate(bytes: usize) -> Option<Self> {
        let pages = bytes.checked_add(4095)? / 4096;
        if pages == 0 {
            return None;
        }
        let physical = get_pmm()?.alloc_contiguous(pages)?.as_usize();
        let virtual_address = vmm::phys_to_virt(physical);
        unsafe {
            core::ptr::write_bytes(virtual_address as *mut u8, 0, pages * 4096);
        }
        Some(Self {
            physical,
            virtual_address,
            pages,
            published: false,
        })
    }
    pub(super) fn physical(&self) -> usize {
        self.physical
    }
    pub(super) fn virtual_address(&self) -> usize {
        self.virtual_address
    }
    pub(super) fn publish(&mut self) {
        // Initialize every descriptor/buffer before enabling device access.
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.published = true;
    }
    /// The device owner must have observed reset completion, disabled bus
    /// mastering, and completed the required device readback before this call.
    pub(super) unsafe fn retire_after_reset(&mut self) {
        self.published = false;
    }
}
impl Drop for DmaRegion {
    fn drop(&mut self) {
        if self.published {
            // Uncertain device ownership deliberately retains the allocation.
            // Controller recovery/final DMA retirement is a separate N0 gate.
            return;
        }
        if let Some(pmm) = get_pmm() {
            for page in 0..self.pages {
                pmm.free_frame(PhysicalAddress(self.physical + page * 4096));
            }
        }
    }
}
