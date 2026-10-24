//! Lifetime operations shared by mapped GUI buffers. Closing a capability does
//! not unmap its pages; both references must be released independently.

/// Release this process's mapping, then its owning capability. The address and
/// length must describe a live handle_map mapping owned by the caller. Existing
/// aliases in other processes remain valid. Return the first syscall error.
pub fn release(handle: u32, address: usize, length: usize) -> isize {
    let unmapped = if address != 0 && length != 0 {
        match length.checked_add(4095) {
            Some(size) => crate::vm_unmap(address, size & !4095),
            None => crate::EINVAL,
        }
    } else {
        0
    };
    let closed = if handle != 0 {
        crate::handle_close(handle)
    } else {
        0
    };
    if unmapped < 0 {
        unmapped
    } else {
        closed
    }
}
