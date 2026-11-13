#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const STACK_BYTES: usize = 64 * 1024;
#[repr(align(16))]
struct Stack([u8; STACK_BYTES]);
static mut STACK_W: Stack = Stack([0; STACK_BYTES]);
static mut STACK_W2: Stack = Stack([0; STACK_BYTES]);
static mut STACK_W3: Stack = Stack([0; STACK_BYTES]);

/// Shared futex word the worker parks on. Starts at OLD, main flips it to NEW.
const OLD: u32 = 0;
const NEW: u32 = 0xC0FFEE;
static FUTEX_WORD: AtomicU32 = AtomicU32::new(OLD);
/// Worker signals it is about to park / has finished here.
static WORKER_READY: AtomicU64 = AtomicU64::new(0);

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn fail(reason: &str) -> ! {
    libdunit::print("futex_test: FAIL ");
    libdunit::println(reason);
    libdunit::exit(1)
}

/// Worker parks until the shared word becomes NEW, then confirms the value.
/// Exit code 0 = success; other codes flag a specific failure.
extern "C" fn worker(_arg: usize) -> ! {
    WORKER_READY.fetch_add(1, Ordering::SeqCst);
    // Loop because a wake is only a hint: recheck the word each time. The
    // compare-and-block is atomic, so if main already flipped the word before
    // we park, futex_wait returns EAGAIN and we observe NEW without blocking.
    let mut spins = 0u64;
    while FUTEX_WORD.load(Ordering::SeqCst) != NEW {
        let r = libdunit::futex_wait(&FUTEX_WORD, OLD, 0);
        // 0 = woken, EAGAIN = value already changed. Anything else is a bug.
        if r != 0 && r != libdunit::EAGAIN {
            libdunit::thread_exit(2);
        }
        spins += 1;
        if spins > 1_000_000 {
            libdunit::thread_exit(3);
        }
    }
    if FUTEX_WORD.load(Ordering::SeqCst) != NEW {
        libdunit::thread_exit(4);
    }
    libdunit::thread_exit(0)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    libdunit::println("futex_test: start");

    // 1) EAGAIN: word value != expected returns immediately.
    let mismatch = AtomicU32::new(5);
    if libdunit::futex_wait(&mismatch, 7, 0) != libdunit::EAGAIN {
        fail("expected EAGAIN on value mismatch");
    }

    // Use raw syscalls for invalid pointers: never create an invalid Rust ref.
    let mapped = libdunit::vm_map(
        0,
        4096,
        libdunit::VM_PROT_READ | libdunit::VM_PROT_WRITE,
        libdunit::VM_MAP_PRIVATE | libdunit::VM_MAP_ANONYMOUS,
    );
    if mapped <= 0 {
        fail("mapping");
    }
    let addr = mapped as usize;
    let raw_wait = |pointer: usize, expected: usize| {
        libdunit::syscall3(libdunit::SYSCALL_FUTEX_WAIT, pointer, expected, 10)
    };
    if raw_wait(addr + 1, 0) != libdunit::EINVAL {
        fail("unaligned errno");
    }
    if libdunit::vm_protect(addr, 4096, libdunit::VM_PROT_READ) < 0 {
        fail("read-only protect");
    }
    if raw_wait(addr + 4092, 1) != libdunit::EAGAIN {
        fail("read-only end-of-page compare");
    }
    if libdunit::vm_protect(addr, 4096, 0) < 0 {
        fail("none protect");
    }
    if raw_wait(addr, 0) != libdunit::EFAULT {
        fail("PROT_NONE errno");
    }
    if libdunit::vm_unmap(addr, 4096) < 0 {
        fail("unmap");
    }
    if raw_wait(addr, 0) != libdunit::EFAULT {
        fail("unmapped errno");
    }
    libdunit::println("futex_test: mapping permissions and errno OK");

    // 2) Timeout: a matching word with a small timeout must return, not hang.
    let timed = AtomicU32::new(42);
    let r = libdunit::futex_wait(&timed, 42, 50);
    if r < 0 {
        fail("timeout wait returned error");
    }

    // 3) Wake: worker parks on FUTEX_WORD == OLD; main flips it and wakes.
    let top_w = unsafe { (core::ptr::addr_of_mut!(STACK_W) as *mut u8).add(STACK_BYTES) };
    let w = libdunit::thread_create(worker, top_w, 0);
    if w <= 0 {
        fail("thread create");
    }
    while WORKER_READY.load(Ordering::SeqCst) != 1 {
        libdunit::yield_now();
    }
    // Give the worker time to actually park in futex_wait.
    for _ in 0..64 {
        libdunit::yield_now();
    }
    FUTEX_WORD.store(NEW, Ordering::SeqCst);
    let woken = libdunit::futex_wake(&FUTEX_WORD, 0);
    if woken < 0 {
        fail("futex_wake error");
    }

    join(w);

    // Both workers must be registered before testing a bounded wake. Their
    // readiness flags precede wait, so yield enough for both to actually park.
    WORKER_READY.store(0, Ordering::SeqCst);
    FUTEX_WORD.store(OLD, Ordering::SeqCst);
    let top2 = unsafe { (core::ptr::addr_of_mut!(STACK_W2) as *mut u8).add(STACK_BYTES) };
    let top3 = unsafe { (core::ptr::addr_of_mut!(STACK_W3) as *mut u8).add(STACK_BYTES) };
    let w2 = libdunit::thread_create(worker, top2, 0);
    let w3 = libdunit::thread_create(worker, top3, 0);
    if w2 <= 0 || w3 <= 0 {
        fail("bounded wake thread create");
    }
    while WORKER_READY.load(Ordering::SeqCst) != 2 {
        libdunit::yield_now();
    }
    for _ in 0..64 {
        libdunit::yield_now();
    }
    FUTEX_WORD.store(NEW, Ordering::SeqCst);
    if libdunit::futex_wake(&FUTEX_WORD, 1) != 1 {
        fail("bounded wake count");
    }
    if libdunit::futex_wake(&FUTEX_WORD, 0) != 1 {
        fail("remaining wake count");
    }
    join(w2);
    join(w3);
    if libdunit::futex_wake(&FUTEX_WORD, 0) != 0 {
        fail("stale registration");
    }
    libdunit::println("futex_test: bounded wake and final drain OK");

    libdunit::println("futex_test: OK");
    libdunit::exit(0)
}

fn join(w: isize) {
    let mut status = libdunit::WaitStatus::empty();
    let mut joined = false;
    for _ in 0..10000 {
        let result = libdunit::thread_join(w as u32, &mut status);
        if result == w as isize {
            if !status.exited() || status.code != 0 {
                fail("worker status");
            }
            joined = true;
            break;
        }
        if result != libdunit::EAGAIN {
            fail("thread join");
        }
        libdunit::yield_now();
    }
    if !joined {
        fail("worker timeout");
    }
}
