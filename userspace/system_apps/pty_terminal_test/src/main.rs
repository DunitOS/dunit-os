#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::exit(101)
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let id = libdunit::pty_create();
    if id < 1 {
        libdunit::exit(1);
    }
    let id = id as u32;
    if libdunit::pty_write(id, b"abc") != 3 {
        libdunit::exit(2);
    }
    let child = libdunit::pty_spawn("/app/musl_terminal", id);
    if child < 1 {
        libdunit::exit(3);
    }
    let mut output = [0u8; 128];
    let mut payload_bytes = 0usize;
    let mut status = libdunit::WaitStatus::empty();
    for _ in 0..100_000 {
        // Drain stdout so child cannot fill its terminal channel.
        let read = libdunit::pty_read(id, &mut output);
        if read > 0 {
            payload_bytes += output[..read as usize]
                .iter()
                .filter(|byte| **byte == b'@')
                .count();
        }
        let waited = libdunit::wait(child as u32, &mut status);
        if waited >= 0 && status.kind != libdunit::WAIT_KIND_EMPTY && !status.spawn_prepared() {
            loop {
                let read = libdunit::pty_read(id, &mut output);
                if read <= 0 {
                    break;
                }
                payload_bytes += output[..read as usize]
                    .iter()
                    .filter(|byte| **byte == b'@')
                    .count();
            }
            libdunit::pty_close(id);
            if payload_bytes != 32 * 1024 {
                libdunit::println("[TERMINAL-ABI] FAIL lost output");
                libdunit::exit(6);
            }
            libdunit::println("[TERMINAL-ABI] 32768 bytes delivered without loss");
            if status.exited() && status.code == 0 {
                libdunit::println("[TERMINAL-ABI] native PTY ioctl PASS");
                libdunit::exit(0);
            }
            libdunit::println("[TERMINAL-ABI] FAIL child");
            libdunit::exit(4);
        }
        libdunit::yield_now();
    }
    libdunit::println("[TERMINAL-ABI] FAIL timeout");
    libdunit::exit(5)
}
