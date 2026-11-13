//! A non-escaping atomic view of a validated user word under an AddressSpace borrow.
use crate::memory::vmm::{AddressSpace, PageFlags, VirtualAddress};
use core::marker::PhantomData;
use core::sync::atomic::{AtomicU32, Ordering};

pub(super) struct UserWord<'a> {
    word: &'a AtomicU32,
    _space: PhantomData<&'a AddressSpace>,
}

impl<'a> UserWord<'a> {
    pub(super) fn new(space: &'a AddressSpace, addr: u64) -> Option<Self> {
        let pointer = resolve(space, addr, false)?;
        Some(Self {
            // resolve validates alignment and PRESENT|USER. The caller borrows
            // the locked AddressSpace for the whole view; no Atomic reference
            // or physical pointer is exposed to callers.
            word: unsafe { &*pointer },
            _space: PhantomData,
        })
    }

    pub(super) fn load(&self) -> u32 {
        self.word.load(Ordering::Acquire)
    }

    pub(super) fn clear(space: &AddressSpace, addr: u64) -> bool {
        let Some(pointer) = resolve(space, addr, true) else {
            return false;
        };
        // Publish exit-side writes before joiners are notified via futex wake.
        unsafe { &*pointer }.store(0, Ordering::Release);
        true
    }
}

fn resolve(space: &AddressSpace, addr: u64, writable: bool) -> Option<*mut AtomicU32> {
    if addr == 0 || addr % core::mem::align_of::<AtomicU32>() as u64 != 0 {
        return None;
    }
    let (phys, flags) = space
        .user_page_mapping(VirtualAddress(addr as usize))
        .ok()??;
    let required = PageFlags::PRESENT
        | PageFlags::USER
        | if writable {
            PageFlags::WRITABLE
        } else {
            PageFlags::empty()
        };
    if !flags.contains(required) {
        return None;
    }
    // An aligned u32 cannot cross a page boundary; keep the guard's borrow
    // alive so private mapping translation cannot be invalidated during access.
    Some(crate::memory::vmm::phys_to_virt(phys.as_usize()) as *mut AtomicU32)
}
