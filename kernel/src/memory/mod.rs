pub mod pmm;
pub mod vmm;
pub(crate) mod tlb;

pub use pmm::MemRegion;

pub(crate) use crate::serial::serial_write;

#[must_use]
pub fn init() -> bool {
    if !pmm::init() {
        serial_write("[MEMORY] FATAL: physical memory manager initialization failed\r\n");
        return false;
    }

    vmm::init();
    crate::allocator::init();
    #[cfg(feature = "boot-smoke-tests")]
    vmm::run_address_space_smoke();
    true
}

#[cfg(feature="boot-smoke-tests")]
mod tlb_smoke;
#[cfg(feature="boot-smoke-tests")]
pub use tlb_smoke::run_tlb_smoke;
