#![no_std]
#![no_main]
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use libdunit::wait_set::*;
const MS: u64 = 1_000_000;
const STACK_BYTES: usize = 64 * 1024;
#[repr(align(16))]
struct Stack([u8; STACK_BYTES]);
static mut STACK: Stack = Stack([0; STACK_BYTES]);
static FD: AtomicU32 = AtomicU32::new(0);
static OTHER: AtomicU32 = AtomicU32::new(0);
static OUTPUT: AtomicUsize = AtomicUsize::new(0);
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    fail("panic")
}
fn fail(reason: &str) -> ! {
    libdunit::print("[WAITSET] FAIL ");
    libdunit::println(reason);
    libdunit::exit(1)
}
fn require(ok: bool, reason: &str) {
    if !ok {
        fail(reason);
    }
}
fn now() -> u64 {
    let mut time = [0u64; 2];
    require(
        libdunit::syscall2(
            libdunit::SYSCALL_CLOCK_GET_TIME,
            1,
            time.as_mut_ptr() as usize,
        ) == 0,
        "clock",
    );
    time[0] * 1_000_000_000 + time[1]
}
fn pipe() -> [u32; 2] {
    let mut fds = [0u32; 2];
    require(
        libdunit::syscall2(libdunit::SYSCALL_PIPE, fds.as_mut_ptr() as usize, 0) == 0,
        "pipe",
    );
    fds
}
fn start(mode: usize) -> u32 {
    let top = unsafe { (core::ptr::addr_of_mut!(STACK.0) as *mut u8).add(STACK_BYTES) };
    let tid = libdunit::thread_create(worker, top, mode);
    require(tid > 0, "create thread");
    tid as u32
}
fn join(tid: u32) {
    let mut status = libdunit::WaitStatus::empty();
    loop {
        let result = libdunit::thread_join(tid, &mut status);
        if result == tid as isize {
            require(status.code == 0, "worker status");
            return;
        }
        require(result == libdunit::EAGAIN, "join");
        libdunit::yield_now();
    }
}
extern "C" fn worker(mode: usize) -> ! {
    if mode != 5 {
        libdunit::sleep_ms(30);
    }
    match mode {
        0 | 5 => {
            require(
                libdunit::write(FD.load(Ordering::SeqCst) as usize, b"p") == 1,
                "write",
            );
        }
        1 => {
            require(
                libdunit::ipc_send(libdunit::get_pid(), b"cancel") == 6,
                "send",
            );
        }
        2 => {
            let fd = FD.load(Ordering::SeqCst);
            require(libdunit::close(fd as usize) == 0, "close watched");
            require(
                libdunit::syscall2(
                    libdunit::SYSCALL_DUP,
                    OTHER.load(Ordering::SeqCst) as usize,
                    fd as usize,
                ) == fd as isize,
                "reuse watched",
            );
        }
        3 => {
            // Unrelated pipe traffic forces spurious readiness rescans. A
            // restarted relative timeout must still expire at its first deadline.
            for _ in 0..25 {
                require(
                    libdunit::write(FD.load(Ordering::SeqCst) as usize, b"s") == 1,
                    "spurious write",
                );
                libdunit::sleep_ms(10);
            }
        }
        4 => {
            require(libdunit::kill(FD.load(Ordering::SeqCst)) == 0, "kill child");
        }
        6 => {
            require(
                libdunit::vm_protect(OUTPUT.load(Ordering::SeqCst), 4096, libdunit::VM_PROT_READ)
                    == 0,
                "protect pending output",
            );
            require(
                libdunit::ipc_send(libdunit::get_pid(), b"fault") == 5,
                "fault wake",
            );
        }
        _ => fail("mode"),
    }
    libdunit::thread_exit(0)
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let fds = pipe();
    FD.store(fds[1], Ordering::SeqCst);
    let mut output = [Event::default(); 4];
    let interests = [
        Interest::descriptor(fds[0], READ, 0xfeed_1234_89ab_cdef),
        Interest::ipc(2),
        Interest::timer(now() + 500 * MS, 3),
    ];
    let tid = start(0);
    require(
        wait(&interests, &mut output, INFINITE) == Ok(1)
            && output[0].token == interests[0].token
            && output[0].events == READ,
        "pipe token",
    );
    // Level-triggered, non-consuming, including an already-ready race.
    for _ in 0..100 {
        require(
            wait(&interests, &mut output, 0) == Ok(1) && output[0].token == interests[0].token,
            "level ready",
        );
    }
    let mut byte = [0];
    require(
        libdunit::read(fds[0] as usize, &mut byte) == 1 && byte[0] == b'p',
        "nonconsuming pipe",
    );
    join(tid);
    // Vary scheduling around registration: a producer may finish before or
    // after the waiter checks. Every single write must produce a wake/result.
    for _ in 0..32 {
        let tid = start(5);
        require(
            wait(&interests[..2], &mut output, 200 * MS) == Ok(1),
            "registration race",
        );
        require(libdunit::read(fds[0] as usize, &mut byte) == 1, "race data");
        join(tid);
    }
    libdunit::println("[WAITSET] pipe token level OK");
    let interests = [
        interests[0],
        interests[1],
        Interest::timer(now() + 500 * MS, 3),
    ];
    let tid = start(1);
    require(
        wait(&interests, &mut output, 200 * MS) == Ok(1) && output[0].token == 2,
        "IPC cancellation",
    );
    let mut msg = [0; 16];
    require(
        libdunit::ipc_recv(&mut msg) == 6 && &msg[..6] == b"cancel",
        "nonconsuming IPC",
    );
    join(tid);
    libdunit::println("[WAITSET] IPC cancellation OK");
    let deadline = now() + 30 * MS;
    require(
        wait(&[Interest::timer(deadline, 77)], &mut output, 500 * MS) == Ok(1)
            && output[0].token == 77
            && output[0].events == ELAPSED
            && now() >= deadline,
        "timer",
    );
    let before = now();
    require(
        wait(&[Interest::ipc(2)], &mut output, 30 * MS) == Ok(0) && now() >= before + 30 * MS,
        "timeout",
    );
    let tid = start(3);
    let before = now();
    require(
        wait(&[Interest::ipc(2)], &mut output, 80 * MS) == Ok(0),
        "spurious timeout",
    );
    require(now() < before + 220 * MS, "deadline restarted");
    join(tid);
    // Discard the unrelated writes before checking hangup.
    let mut drain = [0; 32];
    require(libdunit::read(fds[0] as usize, &mut drain) == 25, "drain");
    libdunit::println("[WAITSET] timer timeout spurious OK");
    require(libdunit::close(fds[1] as usize) == 0, "close writer");
    require(
        wait(&[Interest::descriptor(fds[0], 0, 4)], &mut output, 100 * MS) == Ok(1)
            && output[0].events & HANGUP != 0,
        "peer hangup",
    );
    libdunit::close(fds[0] as usize);
    let fds = pipe();
    let replacement = pipe();
    FD.store(fds[0], Ordering::SeqCst);
    OTHER.store(replacement[0], Ordering::SeqCst);
    let tid = start(2);
    require(
        wait(
            &[Interest::descriptor(fds[0], READ, 5)],
            &mut output,
            300 * MS,
        ) == Ok(1)
            && output[0].token == 5
            && output[0].events == HANGUP | ERROR,
        "close reuse",
    );
    join(tid);
    for fd in [fds[0], fds[1], replacement[0], replacement[1]] {
        libdunit::close(fd as usize);
    }
    libdunit::println("[WAITSET] hangup close reuse OK");
    let child = libdunit::spawn("kill_target");
    require(child > 0, "spawn");
    FD.store(child as u32, Ordering::SeqCst);
    let tid = start(4);
    require(
        wait(&[Interest::process(child as u32, 6)], &mut output, 500 * MS) == Ok(1)
            && output[0].events == EXITED,
        "process exit",
    );
    join(tid);
    let mut status = libdunit::WaitStatus::empty();
    require(
        libdunit::wait(child as u32, &mut status) == child,
        "nonreaping child",
    );
    require(
        wait(&[Interest::process(libdunit::get_pid(), 7)], &mut output, 0) == Err(libdunit::ECHILD),
        "child authority",
    );
    libdunit::println("[WAITSET] child nonreaping authority OK");
    let pty = libdunit::pty_create();
    require(pty > 0, "PTY create");
    let child = libdunit::pty_spawn("waitset_pty_peer", pty as u32);
    require(child > 0, "PTY spawn");
    libdunit::sleep_ms(30);
    require(libdunit::pty_write(pty as u32, b"ok\n") == 3, "PTY write");
    require(
        wait(
            &[Interest::process(child as u32, 93)],
            &mut output,
            2_000 * MS,
        ) == Ok(1)
            && output[0].events == EXITED
            && output[0].data == 0,
        "PTY peer exit",
    );
    require(
        libdunit::wait(child as u32, &mut status) == child && status.code == 0,
        "PTY peer status",
    );
    let mut echoed = [0; 64];
    let n = libdunit::pty_read(pty as u32, &mut echoed);
    require(
        n > 0
            && echoed[..n as usize]
                .windows(b"waitset-pty-ok\n".len())
                .any(|bytes| bytes == b"waitset-pty-ok\n"),
        "PTY echo",
    );
    libdunit::pty_close(pty as u32);
    libdunit::println("[WAITSET] PTY readiness OK");
    require(
        wait(&[Interest::descriptor(999, READ, 8)], &mut output, 0) == Err(libdunit::EBADF),
        "bad fd",
    );
    let raw = |a, n, o, c, t, flags| {
        libdunit::syscall6(libdunit::SYSCALL_WAIT_SET_WAIT, a, n, o, c, t, flags)
    };
    require(raw(0, 65, 0, 65, 0, 0) == libdunit::EINVAL, "count bound");
    require(raw(0, 0, 0, 0, 0, 1) == libdunit::EINVAL, "flags");
    require(
        raw(0, 1, output.as_mut_ptr() as usize, 4, 0, 0) == libdunit::EFAULT,
        "input fault",
    );
    require(
        raw(interests.as_ptr() as usize, 1, 0, 1, 0, 0) == libdunit::EFAULT,
        "output fault",
    );
    let mapped = libdunit::vm_map(
        0,
        4096,
        libdunit::VM_PROT_READ | libdunit::VM_PROT_WRITE,
        libdunit::VM_MAP_PRIVATE | libdunit::VM_MAP_ANONYMOUS,
    );
    require(mapped > 0, "output mapping");
    OUTPUT.store(mapped as usize, Ordering::SeqCst);
    let tid = start(6);
    let ipc = Interest::ipc(99);
    require(
        raw(
            &ipc as *const Interest as usize,
            1,
            mapped as usize,
            1,
            (500 * MS) as usize,
            0,
        ) == libdunit::EFAULT,
        "revalidate pending output",
    );
    join(tid);
    require(
        libdunit::ipc_recv(&mut msg) == 5 && &msg[..5] == b"fault",
        "copy fault consumed IPC",
    );
    require(
        libdunit::vm_unmap(mapped as usize, 4096) == 0,
        "output unmap",
    );
    require(
        wait(
            &[Interest::timer(now(), 101), Interest::timer(now(), 102)],
            &mut output,
            0,
        ) == Ok(2)
            && output[0].token == 101
            && output[1].token == 102,
        "multiple ready order",
    );
    libdunit::println("[WAITSET] validation OK");
    libdunit::println("[WAITSET] OK");
    libdunit::exit(0)
}
