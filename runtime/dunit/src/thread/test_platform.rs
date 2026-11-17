//! Host transport for tests of the actual ownership and futex algorithms.
extern crate std;
use self::std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::Duration,
};
use super::Packet;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
type Waiter = Arc<(Mutex<()>, Condvar)>;
struct Thread {
    handle: std::thread::JoinHandle<(i32, i32)>,
    stack: usize,
}
struct Host {
    threads: Mutex<BTreeMap<usize, Thread>>,
    mappings: Mutex<BTreeMap<usize, std::alloc::Layout>>,
    waiters: Mutex<BTreeMap<usize, Waiter>>,
}
fn host() -> &'static Host {
    static HOST: OnceLock<Host> = OnceLock::new();
    HOST.get_or_init(|| Host {
        threads: Mutex::new(BTreeMap::new()),
        mappings: Mutex::new(BTreeMap::new()),
        waiters: Mutex::new(BTreeMap::new()),
    })
}
std::thread_local! { static ID: core::cell::Cell<usize> = const { core::cell::Cell::new(1) }; }
pub(crate) static NEXT: AtomicUsize = AtomicUsize::new(2);
pub(crate) static FAIL_CREATE: AtomicBool = AtomicBool::new(false);
pub(crate) static FAULT_NEXT: AtomicBool = AtomicBool::new(false);
pub(crate) static ACKNOWLEDGED: AtomicUsize = AtomicUsize::new(0);
pub(crate) static UNMAPPED: AtomicUsize = AtomicUsize::new(0);
pub(super) fn map_stack(bytes: usize) -> isize {
    let layout = std::alloc::Layout::from_size_align(bytes, 4096).unwrap();
    let address = unsafe { std::alloc::alloc_zeroed(layout) } as usize;
    assert_ne!(address, 0);
    host().mappings.lock().unwrap().insert(address, layout);
    address as isize
}
pub(super) fn unmap_stack(address: usize, bytes: usize) -> isize {
    assert!(
        host()
            .threads
            .lock()
            .unwrap()
            .values()
            .all(|thread| thread.stack != address),
        "unmap before native stop acknowledgement"
    );
    let layout = host()
        .mappings
        .lock()
        .unwrap()
        .remove(&address)
        .expect("stack unmapped twice");
    assert_eq!(layout.size(), bytes);
    unsafe {
        std::alloc::dealloc(address as *mut u8, layout);
    }
    UNMAPPED.fetch_add(1, Ordering::SeqCst);
    0
}
pub(super) fn create<T: Send + 'static>(packet: &Packet<T>, top: usize) -> isize {
    if FAIL_CREATE.swap(false, Ordering::SeqCst) {
        return crate::errno::EAGAIN;
    }
    let stack = host()
        .mappings
        .lock()
        .unwrap()
        .iter()
        .find_map(|(&address, layout)| (address + layout.size() == top).then_some(address))
        .expect("thread create must receive the stack top");
    let tid = NEXT.fetch_add(1, Ordering::SeqCst);
    let address = packet as *const Packet<T> as usize;
    let fault = FAULT_NEXT.swap(false, Ordering::SeqCst);
    let handle = std::thread::spawn(move || {
        ID.with(|id| id.set(tid));
        if fault {
            return (1, -14);
        }
        unsafe {
            super::run_job::<T>(address);
        }
        (0, 0)
    });
    host()
        .threads
        .lock()
        .unwrap()
        .insert(tid, Thread { handle, stack });
    tid as isize
}
pub(super) fn join(tid: usize) -> (isize, i32, i32) {
    let mut threads = host().threads.lock().unwrap();
    let Some(thread) = threads.get(&tid) else {
        return (-2, -1, 0);
    };
    if !thread.handle.is_finished() {
        return (crate::errno::EAGAIN, -1, 0);
    }
    let thread = threads.remove(&tid).unwrap();
    drop(threads);
    let (kind, code) = thread.handle.join().unwrap_or((1, -14));
    ACKNOWLEDGED.fetch_add(1, Ordering::SeqCst);
    (tid as isize, kind, code)
}
pub(super) fn current_id() -> usize {
    ID.with(|id| id.get())
}
pub(super) fn yield_now() -> isize {
    std::thread::yield_now();
    0
}
fn waiter(word: &AtomicU32) -> Waiter {
    host()
        .waiters
        .lock()
        .unwrap()
        .entry(word as *const AtomicU32 as usize)
        .or_insert_with(|| Arc::new((Mutex::new(()), Condvar::new())))
        .clone()
}
pub(crate) fn futex_wait(word: &AtomicU32, expected: u32, timeout: u64) -> isize {
    let wait = waiter(word);
    let guard = wait.0.lock().unwrap();
    if word.load(Ordering::Acquire) != expected {
        return crate::errno::EAGAIN;
    }
    if timeout == 0 {
        drop(wait.1.wait(guard).unwrap());
        0
    } else {
        let (_guard, result) = wait
            .1
            .wait_timeout(guard, Duration::from_millis(timeout))
            .unwrap();
        if result.timed_out() {
            crate::errno::EAGAIN
        } else {
            0
        }
    }
}
pub(crate) fn futex_wake(word: &AtomicU32, count: usize) -> isize {
    let wait = waiter(word);
    let _guard = wait.0.lock().unwrap();
    if count == 0 {
        wait.1.notify_all();
    } else {
        for _ in 0..count {
            wait.1.notify_one();
        }
    }
    0
}
pub(crate) fn retry_wait(result: isize) -> bool {
    result >= 0 || matches!(result, crate::errno::EAGAIN | crate::errno::EINTR)
}
