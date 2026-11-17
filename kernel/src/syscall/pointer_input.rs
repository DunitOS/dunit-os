use super::*;
pub(super) fn read(handle: u64, out: u64, len: u64) -> i64 {
    // Check ownership before touching user memory or revealing queue state.
    if handle > u32::MAX as u64 {
        return EACCES;
    }
    let token =
        crate::process::with_current_process(|p| p.require_input_master(handle as u32).ok())
            .flatten();
    let Some(token) = token else {
        return EACCES;
    };
    if len != dunit_pointer_abi::RECORD_LEN as u64
        && len != dunit_pointer_abi::RECORD_WITH_MODIFIERS_LEN as u64
        && len != dunit_pointer_abi::RECORD_WITH_MOTION_LEN as u64
    {
        return EINVAL;
    }
    // Pin/validate VM before input; the copy callback takes no other locks.
    let result = user_memory::with_mapping(out, len as usize, true, |mapping| {
        if len == dunit_pointer_abi::RECORD_WITH_MOTION_LEN as u64 {
            crate::input::read_pointer_event_with_motion(token, |record| {
                copy_record(&mapping, record)
            })
        } else if len == dunit_pointer_abi::RECORD_WITH_MODIFIERS_LEN as u64 {
            crate::input::read_pointer_event_with_modifiers(token, |record| {
                copy_record(&mapping, record)
            })
        } else {
            crate::input::read_pointer_event(token, |record| copy_record(&mapping, record))
        }
    });
    match result {
        Ok(true) => 0,
        Ok(false) => EAGAIN,
        Err(error) => error,
    }
}

pub(super) fn snapshot(out: u64) -> i64 {
    user_memory::with_mapping(out, 16, true, |mapping| {
        crate::input::read_mouse_state(|record| copy_record(&mapping, record))
    })
    .map(|()| 0)
    .unwrap_or_else(|error| error)
}

fn copy_record(mapping: &user_memory::UserMapping<'_>, record: &[u8]) -> Result<(), i64> {
    for (offset, byte) in record.iter().enumerate() {
        mapping.write_byte(offset, *byte);
    }
    Ok(())
}
