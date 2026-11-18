//! The handler has completely unwound before this boundary is called with IF=0.
use super::*;
const RETURN_TO_HAL: i64 = 0x0051_5953_4341_4C4C;

#[no_mangle]
pub extern "C" fn syscall_return_boundary(result: i64) -> i64 {
    let Some(generation) = crate::cpu::reschedule::pending() else { return result; };
    if unsafe { crate::hal::syscall_get_escape_active() } == 0 { return result; }
    // Exit/fault/block/yield already captured their own context. Never replace
    // a syscall restart or exit status with the ordinary return result.
    if result == RETURN_TO_HAL || crate::cpu::runtime().exit_requested.load(Ordering::Acquire)
        || crate::cpu::runtime().yield_requested.load(Ordering::Acquire) {
        crate::cpu::reschedule::capture(generation);
        return RETURN_TO_HAL;
    }
    let Some(pid) = current_entity_id() else { return result; };
    if with_task_record(pid, |process, record| {
        if record.state != ProcessState::Running || process.is_kernel {
            return Err(ProcessError::NotRunnable);
        }
        unsafe { crate::hal::syscall_capture_user_context(&mut process.context, result as u64); }
        record.state = ProcessState::Ready;
        process.state = ProcessState::Ready;
        Ok(())
    }).is_err() { return result; }
    crate::cpu::runtime().yield_requested.store(true, Ordering::Release);
    crate::cpu::reschedule::capture(generation);
    RETURN_TO_HAL
}

/// CPL3 IPI may hand back execution; a CPL0 IPI always IRETs to its handler.
pub(crate) fn reschedule_user_interrupt(frame: &crate::interrupts::InterruptFrame) -> bool {
    if frame.cs & 3 != 3 || unsafe { crate::hal::syscall_get_escape_active() } == 0 {
        return false;
    }
    let Some(generation) = crate::cpu::reschedule::pending() else { return false; };
    if crate::cpu::runtime().exit_requested.load(Ordering::Acquire)
        || crate::cpu::runtime().yield_requested.load(Ordering::Acquire) { return false; }
    let Some(pid) = current_entity_id() else { return false; };
    if with_task_record(pid, |process, record| {
        if record.state != ProcessState::Running || process.is_kernel {
            return Err(ProcessError::NotRunnable);
        }
        process.context = CpuContext {
            rax:frame.rax, rbx:frame.rbx, rcx:frame.rcx, rdx:frame.rdx,
            rsi:frame.rsi, rdi:frame.rdi, rbp:frame.rbp, rsp:frame.rsp,
            r8:frame.r8, r9:frame.r9, r10:frame.r10, r11:frame.r11,
            r12:frame.r12, r13:frame.r13, r14:frame.r14, r15:frame.r15,
            rip:frame.rip, rflags:frame.rflags,
        };
        process.state = ProcessState::Ready;
        record.state = ProcessState::Ready;
        Ok(())
    }).is_err() { return false; }
    crate::cpu::runtime().yield_requested.store(true, Ordering::Release);
    crate::cpu::reschedule::capture(generation);
    true
}
