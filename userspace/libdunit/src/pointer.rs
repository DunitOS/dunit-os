//! Capability-checked, nonblocking pointer event input.
pub use dunit_pointer_abi::{
    PointerEvent, PointerEventWithModifiers, LOST_BEFORE, MODIFIER_MASK, RECORD_LEN,
    RECORD_WITH_MODIFIERS_LEN,
};
/// Read one event owned by INPUT_MASTER. None means EAGAIN, never a snapshot.
pub fn read(handle: u32) -> Result<Option<PointerEvent>, isize> {
    let mut record = [0; RECORD_LEN];
    let result = crate::syscall3(
        crate::SYSCALL_READ_POINTER_EVENT,
        handle as usize,
        record.as_mut_ptr() as usize,
        RECORD_LEN,
    );
    if result == crate::EAGAIN {
        return Ok(None);
    }
    if result < 0 {
        return Err(result);
    }
    PointerEvent::decode(&record).map(Some).ok_or(crate::EINVAL)
}
/// Read a v2 event with the physical modifiers captured when it was enqueued.
/// Old kernels return EINVAL for the explicit 40-byte request; no modifiers are
/// approximated from a later keyboard sample.
pub fn read_with_modifiers(handle: u32) -> Result<Option<PointerEventWithModifiers>, isize> {
    let mut record = [0; RECORD_WITH_MODIFIERS_LEN];
    let result = crate::syscall3(
        crate::SYSCALL_READ_POINTER_EVENT,
        handle as usize,
        record.as_mut_ptr() as usize,
        RECORD_WITH_MODIFIERS_LEN,
    );
    if result == crate::EAGAIN {
        return Ok(None);
    }
    if result < 0 {
        return Err(result);
    }
    PointerEventWithModifiers::decode(&record)
        .map(Some)
        .ok_or(crate::EINVAL)
}

/// Read v3 device motion; the old coordinate/modifier records stay unchanged.
pub fn read_with_motion(
    handle: u32,
) -> Result<Option<dunit_pointer_abi::PointerEventWithMotion>, isize> {
    let mut record = [0; dunit_pointer_abi::RECORD_WITH_MOTION_LEN];
    let result = crate::syscall3(
        crate::SYSCALL_READ_POINTER_EVENT,
        handle as usize,
        record.as_mut_ptr() as usize,
        record.len(),
    );
    if result == crate::EAGAIN {
        return Ok(None);
    }
    if result < 0 {
        return Err(result);
    }
    dunit_pointer_abi::PointerEventWithMotion::decode(&record)
        .map(Some)
        .ok_or(crate::EINVAL)
}
