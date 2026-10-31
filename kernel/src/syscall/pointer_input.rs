use super::*;
pub(super) fn read(handle: u64, out: u64, len: u64) -> i64 {
    let _irq = crate::sync::InterruptGuard::new();
    // Check ownership before touching user memory or revealing queue state.
    if handle > u32::MAX as u64 {
        return EACCES;
    }
    let authorised = crate::process::current_process()
        .map(|p| p.require_input_master(handle as u32).is_ok())
        .unwrap_or(false);
    if !authorised {
        return EACCES;
    }
    if len != dunit_pointer_abi::RECORD_LEN as u64 {
        return EINVAL;
    }
    if let Err(error) = validate_user_mapping(out, len as usize, true) {
        return error;
    }
    match crate::input::read_pointer_event(|record| copy_buffer_to_user(out as *mut u8, record)) {
        Ok(true) => 0,
        Ok(false) => EAGAIN,
        Err(error) => error,
    }
}
