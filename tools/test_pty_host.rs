//! Compile with rustc --test; exercises the actual PTY table against process hooks.
extern crate alloc;
#[allow(dead_code)] #[path = "../kernel/src/sync.rs"] mod actual_sync;
mod sync {
    use crate::actual_sync::{SpinLock, SpinLockGuard};
    use std::ops::{Deref, DerefMut};

    pub struct IrqSafeSpinLock<T>(SpinLock<T>);
    pub struct IrqSafeSpinLockGuard<'a, T>(SpinLockGuard<'a, T>);

    impl<T> IrqSafeSpinLock<T> {
        pub const fn new(value: T) -> Self { Self(SpinLock::new(value)) }
        pub fn lock(&self) -> IrqSafeSpinLockGuard<'_, T> {
            let guard = self.0.lock();
            super::GUARD_DEPTH.with(|depth| depth.set(depth.get() + 1));
            IrqSafeSpinLockGuard(guard)
        }
    }
    impl<T> Deref for IrqSafeSpinLockGuard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T { &self.0 }
    }
    impl<T> DerefMut for IrqSafeSpinLockGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut T { &mut self.0 }
    }
    impl<T> Drop for IrqSafeSpinLockGuard<'_, T> {
        fn drop(&mut self) {
            super::GUARD_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
}
use std::{alloc::{GlobalAlloc, Layout, System}, cell::Cell};
thread_local! {
    static GUARD_DEPTH: Cell<usize> = const { Cell::new(0) };
    static CHECK_ALLOC: Cell<bool> = const { Cell::new(false) };
}
fn check_alloc_scope() {
    if CHECK_ALLOC.try_with(Cell::get).unwrap_or(false)
        && GUARD_DEPTH.try_with(Cell::get).unwrap_or(0) != 0
    {
        std::process::abort();
    }
}
struct CheckingAllocator;
#[global_allocator]
static ALLOCATOR: CheckingAllocator = CheckingAllocator;
unsafe impl GlobalAlloc for CheckingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        check_alloc_scope();
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        check_alloc_scope();
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, len: usize) -> *mut u8 {
        check_alloc_scope();
        System.realloc(ptr, layout, len)
    }
}
mod process {
    pub type ProcessId = u64;
    use std::sync::atomic::{AtomicBool,AtomicU64,Ordering};
    pub static PRESENT: AtomicBool = AtomicBool::new(true);
    pub static PROBE: AtomicU64 = AtomicU64::new(0);
    pub static WAKES: AtomicU64 = AtomicU64::new(0);
    pub fn process_exists(pid: ProcessId) -> bool {
        // Reentry catches a PTY -> registry edge even for readiness snapshots.
        let _ = crate::pty::master_read(PROBE.load(Ordering::SeqCst) as u32, 1, &mut []);
        pid != 900 || PRESENT.load(Ordering::SeqCst)
    }
    pub fn wake_poll_waiters() {
        WAKES.fetch_add(1, Ordering::SeqCst);
        let _ = crate::pty::master_read(PROBE.load(Ordering::SeqCst) as u32, 1, &mut []);
    }
}
#[path = "../kernel/src/pty/mod.rs"] mod pty;

#[test]
fn wrapped_ring_grows_and_retires_storage_outside_the_guard() {
    let id = pty::create(7);
    let endpoint = pty::attach_slave(id, 7).unwrap();
    let mut output = [0; 64];
    CHECK_ALLOC.with(|check| check.set(true));
    assert_eq!(pty::master_write(id, 7, b"0123456789abcdef"), Some(16));
    assert!(matches!(pty::slave_read_stdin(&endpoint, &mut output[..12]), Some(pty::SlaveRead::Data(12))));
    assert_eq!(pty::master_write(id, 7, b"ABCDEFGHIJKL"), Some(12));
    assert_eq!(pty::master_write(id, 7, &[255; 20]), Some(20));
    assert!(matches!(pty::slave_read_stdin(&endpoint, &mut output), Some(pty::SlaveRead::Data(36))));
    CHECK_ALLOC.with(|check| check.set(false));
    assert_eq!(&output[..16], b"cdefABCDEFGHIJKL");
    assert_eq!(&output[16..36], &[255; 20]);
    assert!(pty::close(id, 7));
}

#[test]
fn endpoint_geometry_ownership_readiness_and_close_eof() {
    let id = pty::create(1);
    assert!(pty::attach_slave(id, 99).is_none());
    let endpoint = pty::attach_slave(id, 1).unwrap();
    assert!(pty::channel_count() >= 1);
    assert_eq!(pty::slave_readiness(&endpoint), Some((false, false)));
    assert_eq!(pty::slave_dimensions(&endpoint), Some([0; 8]));
    let geometry = [24, 0, 80, 0, 0, 0, 0, 0];
    assert!(pty::set_slave_dimensions(&endpoint, geometry));
    assert_eq!(pty::slave_dimensions(&endpoint), Some(geometry));
    assert_eq!(pty::master_write(id, 99, b"wrong"), None);
    assert_eq!(pty::master_write(id, 1, b"hello"), Some(5));
    assert_eq!(pty::slave_pending_input(&endpoint), Some(5));
    assert_eq!(pty::slave_readiness(&endpoint), Some((true, false)));
    assert_eq!(pty::slave_write_stdout(&endpoint, b"out"), Some(Ok(3)));
    let mut output = [0; 16];
    assert_eq!(pty::master_read(id, 1, &mut output), Some((3, false)));
    assert_eq!(&output[..3], b"out");
    let large = [7u8; pty::PTY_RING_CAP + 1];
    assert_eq!(pty::slave_write_stdout(&endpoint, &large), Some(Ok(pty::PTY_RING_CAP)));
    assert_eq!(pty::slave_output_readiness(&endpoint), Some((false, false)));
    assert_eq!(pty::slave_write_stdout(&endpoint, b"overflow"), Some(Ok(0)));
    assert_eq!(pty::master_read(id, 1, &mut output), Some((16, false)));
    assert_eq!(pty::slave_output_readiness(&endpoint), Some((true, false)));
    assert!(!pty::close(id, 99));
    assert!(pty::close(id, 1));
    assert_eq!(pty::master_write(id, 1, b"late"), None);
    assert_eq!(pty::slave_write_stdout(&endpoint, b"late"), Some(Err(())));
    assert_eq!(pty::slave_readiness(&endpoint), Some((true, true)));
    assert!(matches!(pty::slave_read_stdin(&endpoint, &mut output), Some(pty::SlaveRead::Data(5))));
    assert_eq!(&output[..5], b"hello");
    assert!(matches!(pty::slave_read_stdin(&endpoint, &mut output), Some(pty::SlaveRead::Eof)));
    let inherited = endpoint.clone();
    drop(endpoint);
    assert_eq!(pty::slave_dimensions(&inherited), Some(geometry));
    drop(inherited);
    assert_eq!(pty::master_read(id, 1, &mut output), None);
    pty::on_process_exit(1);
}

#[test]
fn replacement_survives_old_endpoint_destructor() {
    let id = pty::create(3);
    let old = std::sync::Arc::try_unwrap(pty::attach_slave(id, 3).unwrap()).unwrap();
    // Strong count is zero while the old object's destructor has not run yet.
    let replacement = pty::attach_slave(id, 3).unwrap();
    assert!(pty::close(id, 3));
    drop(old);
    assert_eq!(pty::slave_dimensions(&replacement), Some([0; 8]));
    assert!(matches!(pty::slave_read_stdin(&replacement, &mut []), Some(pty::SlaveRead::Eof)));
    drop(replacement);
    assert!(!pty::channel_exists(id));
}

#[test]
fn moved_endpoint_still_retires_its_attachment() {
    let id = pty::create(4);
    let endpoint = std::sync::Arc::try_unwrap(pty::attach_slave(id, 4).unwrap()).unwrap();
    // A private endpoint may still be moved by Arc::try_unwrap. Its identity
    // must survive that move, instead of depending on the object's address.
    assert!(pty::close(id, 4));
    drop(endpoint);
    assert!(!pty::channel_exists(id));
}

#[test]
fn detached_master_eof_precedes_delayed_resource_cleanup() {
    use std::sync::atomic::Ordering;
    let id = pty::create(900);
    let endpoint = pty::attach_slave(id, 900).unwrap();
    assert_eq!(pty::master_write(id, 900, b"queued"), Some(6));
    process::PRESENT.store(false, Ordering::SeqCst);
    assert_eq!(pty::slave_readiness(&endpoint), Some((true, true)));
    assert_eq!(pty::slave_write_stdout(&endpoint, b"late"), Some(Err(())));
    let mut buf = [0; 8];
    assert!(matches!(pty::slave_read_stdin(&endpoint, &mut buf), Some(pty::SlaveRead::Data(6))));
    assert!(matches!(pty::slave_read_stdin(&endpoint, &mut buf), Some(pty::SlaveRead::Eof)));
    pty::on_process_exit(900);
    drop(endpoint);
}

#[test]
fn concurrent_bidirectional_stream_and_notifications_reenter() {
    use std::sync::atomic::Ordering;
    let id = pty::create(100);
    let endpoint = pty::attach_slave(id, 100).unwrap();
    process::PROBE.store(id as u64, Ordering::SeqCst);
    let start = process::WAKES.load(Ordering::SeqCst);
    let rounds = 100_000;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for n in 0..rounds {
                while pty::master_write(id, 100, &[n as u8]) == Some(0) { std::thread::yield_now(); }
            }
        });
        scope.spawn(|| {
            for n in 0..rounds {
                let mut byte = [0];
                loop {
                    match pty::slave_read_stdin(&endpoint, &mut byte) {
                        Some(pty::SlaveRead::Data(1)) => break,
                        Some(pty::SlaveRead::WouldBlock) => std::thread::yield_now(),
                        _ => panic!("unexpected read"),
                    }
                }
                assert_eq!(byte[0], n as u8);
            }
        });
        scope.spawn(|| {
            for n in 0..rounds {
                while pty::slave_write_stdout(&endpoint, &[n as u8]) == Some(Ok(0)) { std::thread::yield_now(); }
            }
        });
        scope.spawn(|| {
            for n in 0..rounds {
                let mut byte = [0];
                while pty::master_read(id, 100, &mut byte) == Some((0, false)) { std::thread::yield_now(); }
                assert_eq!(byte[0], n as u8);
            }
        });
    });
    assert!(process::WAKES.load(Ordering::SeqCst) >= start + rounds * 4);
    assert!(pty::close(id, 100));
    drop(endpoint);
    process::PROBE.store(0, Ordering::SeqCst);
}
