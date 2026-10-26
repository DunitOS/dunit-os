//! Deterministic regression for a kernel interrupt in the fault-return window.
//! Compiled only into the existing boot-smoke-tests profile.
use core::sync::atomic::{AtomicBool, Ordering};
static VERIFIED: AtomicBool = AtomicBool::new(false);

pub(super) fn verify() {
    if !super::user_fault_escape_requested() {
        return;
    }
    // The userspace trampoline has returned, but the runner has not consumed
    // PROCESS_EXIT_REQUESTED yet. Previously any IRQ here re-entered an inactive
    // escape frame and halted. INT3 exercises the real interrupt entry/IRET path
    // at CPL0 without faking a timer tick or altering the user's exit request.
    assert_eq!(unsafe { crate::hal::syscall_get_escape_active() }, 0);
    unsafe { core::arch::asm!("int3"); }
    assert!(super::user_fault_escape_requested(), "kernel interrupt consumed the pending user fault");
    if !VERIFIED.swap(true, Ordering::SeqCst) {
        crate::memory::serial_write("[USER-FAULT-IRQ] OK kernel interrupt preserved fault return\r\n");
    }
}
