//! Actual SDK algorithms with host-only thread/VM/futex transport.
extern crate alloc;
#[path = "../../../userspace/libdunit/src/errno_numbers.rs"]
pub mod errno;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub isize);
pub type Result<T> = core::result::Result<T, Error>;
mod error {
    pub fn checked(result: isize) -> crate::Result<usize> {
        if result < 0 {
            Err(crate::Error(result))
        } else {
            Ok(result as usize)
        }
    }
}
#[path = "../src/sync.rs"]
mod sync;
#[path = "../src/thread.rs"]
mod thread;

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use sync::{Condvar, Mutex};
struct CountDrop(Arc<AtomicUsize>);
impl Drop for CountDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn owned_result_capture_and_stack_survive_until_join() {
    let drops = Arc::new(AtomicUsize::new(0));
    let capture = CountDrop(drops.clone());
    let result = CountDrop(drops.clone());
    let worker = thread::spawn(move || {
        drop(capture);
        (String::from("owned result"), result, thread::current_id())
    })
    .unwrap();
    let id = worker.thread_id();
    assert_ne!(id, thread::current_id());
    let (text, result, current) = worker.join().unwrap();
    assert_eq!(current, id);
    assert_eq!(text, "owned result");
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(result);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}
#[test]
fn dropping_handle_joins_and_drops_unclaimed_result() {
    let drops = Arc::new(AtomicUsize::new(0));
    let result = CountDrop(drops.clone());
    drop(thread::spawn(move || result).unwrap());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
#[test]
fn create_failure_reclaims_stack_and_closure() {
    let before = thread::platform::UNMAPPED.load(Ordering::SeqCst);
    let drops = Arc::new(AtomicUsize::new(0));
    let capture = CountDrop(drops.clone());
    thread::platform::FAIL_CREATE.store(true, Ordering::SeqCst);
    let result = thread::spawn(move || drop(capture));
    assert!(matches!(result, Err(Error(errno::EAGAIN))));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        thread::platform::UNMAPPED.load(Ordering::SeqCst),
        before + 1
    );
}
#[test]
fn native_fault_without_completion_is_detected_and_reclaimed() {
    let drops = Arc::new(AtomicUsize::new(0));
    let capture = CountDrop(drops.clone());
    thread::platform::FAULT_NEXT.store(true, Ordering::SeqCst);
    let worker = thread::spawn(move || drop(capture)).unwrap();
    assert_eq!(worker.join(), Err(thread::JoinError { kind: 1, code: -14 }));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
#[test]
fn stack_size_validation_is_checked_before_native_creation() {
    for bytes in [0, 1, 4096, 16 * 1024 - 1, usize::MAX, usize::MAX - 4096] {
        assert!(matches!(
            thread::Builder::new().stack_size(bytes).spawn(|| 1),
            Err(Error(errno::EINVAL))
        ));
    }
    assert_eq!(
        thread::Builder::new()
            .stack_size(16 * 1024 + 1)
            .spawn(|| 42)
            .unwrap()
            .join()
            .unwrap(),
        42
    );
}
#[test]
fn mutex_contention_preserves_nonatomic_payload_and_bounds() {
    let shared = Arc::new(Mutex::new((0usize, 0usize)));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let shared = shared.clone();
            thread::spawn(move || {
                for _ in 0..1000 {
                    let mut pair = shared.lock();
                    assert_eq!(pair.1, pair.0 * 3);
                    pair.0 += 1;
                    pair.1 += 3;
                }
            })
            .unwrap()
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(*shared.lock(), (8000, 24000));
    let guard = shared.lock();
    assert!(shared.try_lock().is_none());
    drop(guard);
    assert!(shared.try_lock().is_some());
}
#[test]
fn condition_predicates_survive_notify_before_wait_and_notify_all() {
    let pair = Arc::new((Mutex::new(0usize), Condvar::new()));
    let ready = Arc::new(AtomicUsize::new(0));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let pair = pair.clone();
            let ready = ready.clone();
            thread::spawn(move || {
                let guard = pair.0.lock();
                ready.fetch_add(1, Ordering::SeqCst);
                let guard = pair.1.wait_while(guard, |value| *value != 77);
                assert_eq!(*guard, 77);
            })
            .unwrap()
        })
        .collect();
    while ready.load(Ordering::SeqCst) != 8 {
        thread::yield_now().unwrap();
    }
    *pair.0.lock() = 77;
    pair.1.notify_all();
    for worker in workers {
        worker.join().unwrap();
    }
    pair.1.notify_one();
    assert_eq!(*pair.1.wait_while(pair.0.lock(), |value| *value != 77), 77);
}
#[test]
fn notify_one_with_repeated_handoff_never_loses_a_generation() {
    let pair = Arc::new((Mutex::new((0usize, false)), Condvar::new()));
    let remote = pair.clone();
    let worker = thread::spawn(move || {
        for step in 1..=1000 {
            let mut state = remote.1.wait_while(remote.0.lock(), |state| !state.1);
            assert_eq!(state.0, step);
            state.1 = false;
            remote.1.notify_one();
        }
    })
    .unwrap();
    for step in 1..=1000 {
        let mut state = pair.1.wait_while(pair.0.lock(), |state| state.1);
        state.0 = step;
        state.1 = true;
        pair.1.notify_one();
    }
    worker.join().unwrap();
    assert_eq!(pair.0.lock().0, 1000);
}
#[test]
fn mutex_exclusive_access_and_positive_thread_traits() {
    fn send<T: Send>() {}
    fn sync<T: Sync>() {}
    send::<thread::JoinHandle<std::cell::Cell<usize>>>();
    sync::<thread::JoinHandle<std::cell::Cell<usize>>>();
    send::<Mutex<std::cell::Cell<usize>>>();
    sync::<Mutex<std::cell::Cell<usize>>>();
    sync::<sync::MutexGuard<'static, usize>>();
    let mut mutex = Mutex::new(4);
    *mutex.get_mut() += 2;
    assert_eq!(mutex.into_inner(), 6);
}

#[test]
fn full_width_thread_identity_is_never_truncated() {
    thread::platform::NEXT.store(u32::MAX as usize + 1, Ordering::SeqCst);
    let worker = thread::spawn(thread::current_id).unwrap();
    let id = worker.thread_id();
    assert_eq!(worker.join().unwrap(), id);
    assert_ne!(id, thread::current_id());
}
