//! Capability-checked, nonblocking pointer event input.
pub use dunit_pointer_abi::{PointerEvent, LOST_BEFORE, RECORD_LEN};
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
