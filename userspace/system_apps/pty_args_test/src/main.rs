#![no_std]
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    fail("panic")
}
fn fail(why: &str) -> ! {
    libdunit::println(&alloc::format!("[PTY-ARGS] FAIL {}", why));
    libdunit::exit(1)
}

fn receive(id: u32, needle: &[u8]) -> bool {
    let mut output = Vec::new();
    for _ in 0..1000 {
        let mut bytes = [0u8; 512];
        let count = libdunit::pty_read(id, &mut bytes);
        if count > 0 {
            output.extend_from_slice(&bytes[..count as usize]);
            if output.windows(needle.len()).any(|s| s == needle) {
                return true;
            }
            if output.len() > 8192 {
                return false;
            }
        } else if count == libdunit::EPIPE {
            return false;
        } else if count < 0 && count != libdunit::EAGAIN {
            return false;
        }
        libdunit::sleep_ms(10);
    }
    false
}

fn copy_child() {
    let input = libdunit::open("/app/args_test", libdunit::OPEN_READ);
    let output = libdunit::open(
        "/tmp/pty args child",
        libdunit::OPEN_WRITE | libdunit::OPEN_CREATE | libdunit::OPEN_TRUNC,
    );
    if input < 0 || output < 0 {
        fail("copy open");
    }
    loop {
        let mut buffer = [0u8; 4096];
        let count = libdunit::read(input as usize, &mut buffer);
        if count < 0 {
            fail("copy read");
        }
        if count == 0 {
            break;
        }
        if libdunit::write(output as usize, &buffer[..count as usize]) != count {
            fail("copy write");
        }
    }
    libdunit::close(input as usize);
    libdunit::close(output as usize);
}

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: libdunit::RawArgv, envp: libdunit::RawEnvp) -> ! {
    libdunit::init_runtime(argc, argv, envp);
    copy_child();
    if libdunit::chdir("/tmp") != 0 {
        fail("cwd");
    }
    let raw_name = "args_test";
    if libdunit::syscall2(
        libdunit::SYSCALL_SPAWN_PROCESS,
        raw_name.as_ptr() as usize,
        raw_name.len(),
    ) != libdunit::ENOENT
    {
        fail("kernel searched command name");
    }
    if libdunit::process::resolve_program("args_test")
        .ok()
        .as_deref()
        != Some("/app/args_test")
    {
        fail("userspace PATH");
    }
    libdunit::println("[PTY-ARGS] userspace PATH and raw-path-only kernel OK");
    let id = libdunit::pty_create();
    if id < 0 {
        fail("create");
    }
    let id = id as u32;
    let child = libdunit::process::pty_spawn_args(
        "./pty args child",
        id,
        &["child name", "hello world", "", "last"],
        &[],
    );
    if child < 0 {
        fail("explicit argv spawn");
    }
    if !receive(id, b"argv[1]=hello world\nargv[2]=\nargv[3]=last\n") {
        fail("argv/empty preservation");
    }
    libdunit::pty_close(id);
    let mut status = libdunit::WaitStatus::empty();
    while libdunit::wait(child as u32, &mut status) == libdunit::EAGAIN {
        libdunit::yield_now();
    }
    libdunit::println("[PTY-ARGS] explicit argv and empty argument OK");

    let legacy_id = libdunit::pty_create();
    if legacy_id < 0 {
        fail("legacy create");
    }
    let legacy_path = "/app/args_test";
    let legacy_child = libdunit::syscall6(
        libdunit::SYSCALL_PTY_SPAWN,
        legacy_path.as_ptr() as usize,
        legacy_path.len(),
        legacy_id as usize,
        0,
        0,
        usize::MAX,
    );
    if legacy_child < 0 || !receive(legacy_id as u32, b"argc=1\n") {
        fail("legacy ignored sixth register");
    }
    libdunit::pty_close(legacy_id as u32);
    while libdunit::wait(legacy_child as u32, &mut status) == libdunit::EAGAIN {
        libdunit::yield_now();
    }
    libdunit::println("[PTY-ARGS] legacy block ignores unused sixth register OK");

    let shell_id = libdunit::pty_create();
    if shell_id < 0 {
        fail("shell create");
    }
    let shell_id = shell_id as u32;
    let shell = libdunit::pty_spawn("dsh", shell_id);
    if shell < 0 {
        fail("shell spawn");
    }
    if !receive(shell_id, b"dsh") {
        fail("shell ready");
    }
    let command = b"exec './pty args child' 'quoted words' \"\" final\n";
    if libdunit::pty_write(shell_id, command) != command.len() as isize {
        fail("shell input");
    }
    if !receive(shell_id, b"argv[1]=quoted words\nargv[2]=\nargv[3]=final\n") {
        fail("dsh quote/argv forwarding");
    }
    libdunit::kill(shell as u32);
    libdunit::pty_close(shell_id);
    while libdunit::wait(shell as u32, &mut status) == libdunit::EAGAIN {
        libdunit::yield_now();
    }
    libdunit::println("[PTY-ARGS] dsh quoted executable and arguments OK");
    libdunit::println("[PTY-ARGS] DONE");
    libdunit::exit(0)
}
