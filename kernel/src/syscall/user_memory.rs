//! User mapping ownership spans validation and every byte/pixel access.
use super::*;

/// Verify every page spanned by `[ptr, ptr+size)` is present, user-accessible
/// and (optionally) writable. Unlike [`validate_user_mapping`] this does NOT
/// impose the small [`MAX_USER_COPY`] cap, so it suits large bulk transfers
/// (e.g. `fb_present`) whose own upper bound is enforced by the caller. The
/// address range and mapping are checked together by the pinned view.
pub(super) fn validate_user_pages(ptr: u64, size: usize, writable: bool) -> Result<(), i64> {
    with_mapping(ptr, size, writable, |_| Ok(()))
}

pub fn user_copy_is_range_checked_only() -> bool {
    false
}

pub(crate) fn copy_string_from_user_len(
    ptr: *const u8,
    len: usize,
    max_len: usize,
) -> Result<String, i64> {
    if max_len == 0 || max_len > MAX_USER_COPY {
        syscall_log!("[SYSCALL] invalid user string max_len={}\r\n", max_len);
        return Err(EINVAL);
    }

    if len > max_len {
        syscall_log!(
            "[SYSCALL] user string too large: len={}, max={}\r\n",
            len,
            max_len
        );
        return Err(ENAMETOOLONG);
    }

    validate_user_range(ptr as u64, len)?;
    with_mapping(ptr as u64, len, false, |mapping| {
        let mut bytes = Vec::new();
        for offset in 0..len {
            let byte = mapping.read_byte(offset);
            if byte == 0 {
                return Err(EINVAL);
            }
            bytes.push(byte);
        }

        String::from_utf8(bytes).map_err(|_| EINVAL)
    })
}

pub(crate) fn copy_buffer_from_user(ptr: *const u8, len: usize) -> Result<Vec<u8>, i64> {
    if len == 0 {
        return Ok(Vec::new());
    }

    validate_user_range(ptr as u64, len)?;
    with_mapping(ptr as u64, len, false, |mapping| {
        let mut out = Vec::new();
        out.reserve(len);
        for offset in 0..len {
            let byte = mapping.read_byte(offset);
            out.push(byte);
        }

        Ok(out)
    })
}

pub(crate) fn copy_buffer_to_user(ptr: *mut u8, data: &[u8]) -> Result<(), i64> {
    if data.is_empty() {
        return Ok(());
    }

    validate_user_range(ptr as u64, data.len())?;
    with_mapping(ptr as u64, data.len(), true, |mapping| {
        for (offset, byte) in data.iter().enumerate() {
            mapping.write_byte(offset, *byte);
        }

        Ok(())
    })
}

use crate::memory::vmm::{AddressSpace, PageFlags, VirtualAddress};
use core::marker::PhantomData;
enum Backing<'a> {
    User(&'a AddressSpace),
    #[cfg(feature = "boot-smoke-tests")]
    Bootstrap {
        _irq: &'a crate::sync::InterruptGuard,
    },
    Empty,
}
/// A CPU-bound raw view whose mapping owner stays locked through the callback.
/// Only owned bytes/scalars leave it; no user slice or AS guard is exported.
pub(super) struct UserMapping<'a> {
    ptr: u64,
    len: usize,
    writable: bool,
    backing: Backing<'a>,
    _cpu_bound: PhantomData<*mut ()>,
}
impl UserMapping<'_> {
    fn flags(&self, page: u64) -> Result<PageFlags, i64> {
        let address = VirtualAddress::from_usize(page as usize);
        match &self.backing {
            Backing::User(space) => space.user_page_flags(address),
            #[cfg(feature = "boot-smoke-tests")]
            Backing::Bootstrap { .. } => crate::memory::vmm::active_user_page_flags(address),
            Backing::Empty => return Err(EFAULT),
        }
        .map_err(|_| EFAULT)?
        .ok_or(EFAULT)
    }
    fn validate(&self) -> Result<(), i64> {
        if self.len == 0 {
            return Ok(());
        }
        let mut page = self.ptr & !4095;
        let last = (self.ptr + self.len as u64 - 1) & !4095;
        loop {
            let flags = self.flags(page)?;
            if !flags.contains(PageFlags::PRESENT | PageFlags::USER)
                || (self.writable && !flags.contains(PageFlags::WRITABLE))
            {
                return Err(EFAULT);
            }
            if page == last {
                break;
            }
            page += 4096;
        }
        Ok(())
    }
    pub(super) fn executable(&self) -> bool {
        self.len != 0
            && self
                .flags(self.ptr & !4095)
                .map(|flags| !flags.contains(PageFlags::NO_EXECUTE))
                .unwrap_or(false)
    }
    pub(super) fn read_byte(&self, offset: usize) -> u8 {
        assert!(offset < self.len);
        unsafe { core::ptr::read_volatile((self.ptr as *const u8).add(offset)) }
    }
    pub(super) fn write_byte(&self, offset: usize, value: u8) {
        assert!(self.writable && offset < self.len);
        unsafe {
            core::ptr::write_volatile((self.ptr as *mut u8).add(offset), value);
        }
    }
    /// Destination must be a valid kernel-owned span of `count` u32 words.
    /// Validate the source once per row, preserving forward volatile blit
    /// semantics without adding a bounds check to every framebuffer pixel.
    pub(super) unsafe fn copy_words_volatile(
        &self,
        offset: usize,
        destination: *mut u32,
        count: usize,
    ) {
        assert!(count
            .checked_mul(4)
            .and_then(|bytes| offset.checked_add(bytes))
            .is_some_and(|end| end <= self.len));
        let source = (self.ptr as usize + offset) as *const u8;
        if source as usize & 3 == 0 {
            for i in 0..count {
                core::ptr::write_volatile(
                    destination.add(i),
                    core::ptr::read_volatile((source as *const u32).add(i)),
                );
            }
        } else {
            for i in 0..count {
                let at = source.add(i * 4);
                let word = u32::from_le_bytes([
                    core::ptr::read_volatile(at),
                    core::ptr::read_volatile(at.add(1)),
                    core::ptr::read_volatile(at.add(2)),
                    core::ptr::read_volatile(at.add(3)),
                ]);
                core::ptr::write_volatile(destination.add(i), word);
            }
        }
    }
}
pub(super) fn with_mapping<R>(
    ptr: u64,
    len: usize,
    writable: bool,
    op: impl for<'a> FnOnce(UserMapping<'a>) -> Result<R, i64>,
) -> Result<R, i64> {
    if len == 0 {
        return op(UserMapping {
            ptr,
            len,
            writable,
            backing: Backing::Empty,
            _cpu_bound: PhantomData,
        });
    }
    if !is_valid_user_pointer(ptr, len) {
        return Err(EFAULT);
    }
    if let Some(space) = crate::process::active_address_space_pin() {
        let guard = space.lock();
        let view = UserMapping {
            ptr,
            len,
            writable,
            backing: Backing::User(&guard),
            _cpu_bound: PhantomData,
        };
        view.validate()?;
        op(view)
    } else {
        // Only the early ring-3 bootstrap tests use user pages in the permanent
        // kernel root without a dispatched Process. No production fallback.
        #[cfg(feature = "boot-smoke-tests")]
        {
            let irq = crate::sync::InterruptGuard::new();
            let view = UserMapping {
                ptr,
                len,
                writable,
                backing: Backing::Bootstrap { _irq: &irq },
                _cpu_bound: PhantomData,
            };
            view.validate()?;
            op(view)
        }
        #[cfg(not(feature = "boot-smoke-tests"))]
        {
            Err(EFAULT)
        }
    }
}
