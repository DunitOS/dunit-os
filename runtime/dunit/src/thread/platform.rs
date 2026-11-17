use super::Packet;
use core::sync::atomic::AtomicU32;
pub(super) fn map_stack(bytes: usize) -> isize {
    libdunit::vm_map(
        0,
        bytes,
        libdunit::VM_PROT_READ | libdunit::VM_PROT_WRITE,
        libdunit::VM_MAP_PRIVATE | libdunit::VM_MAP_ANONYMOUS | libdunit::VM_MAP_GUARD,
    )
}
pub(super) fn unmap_stack(address: usize, bytes: usize) -> isize {
    libdunit::vm_unmap(address, bytes)
}
pub(super) fn create<T: Send + 'static>(packet: &Packet<T>, top: usize) -> isize {
    libdunit::thread_create(
        super::entry::<T>,
        top as *mut u8,
        packet as *const Packet<T> as usize,
    )
}
pub(super) fn join(tid: usize) -> (isize, i32, i32) {
    let mut status = libdunit::WaitStatus::empty();
    (
        libdunit::syscall2(
            libdunit::SYSCALL_THREAD_JOIN,
            tid,
            &mut status as *mut libdunit::WaitStatus as usize,
        ),
        status.kind,
        status.code,
    )
}
pub(super) fn current_id() -> usize {
    libdunit::syscall0(libdunit::SYSCALL_GET_TID) as usize
}
pub(super) fn yield_now() -> isize {
    libdunit::yield_now()
}
pub(crate) fn futex_wait(word: &AtomicU32, expected: u32, timeout: u64) -> isize {
    libdunit::futex_wait(word, expected, timeout)
}
pub(crate) fn futex_wake(word: &AtomicU32, count: usize) -> isize {
    libdunit::futex_wake(word, count)
}
pub(crate) fn retry_wait(result: isize) -> bool {
    result >= 0 || matches!(result, libdunit::EAGAIN | libdunit::EINTR)
}
