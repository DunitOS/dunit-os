//! Native packet authority checks on an ordinary Terminal-launched process.
use alloc::format;
use dunit_packet_abi::QueueOptions;
use libdunit as sys;

const TEST_APP: &str = "/app/abi_test";
const DEVICE_RIGHTS: u32 = sys::RIGHT_READ | sys::RIGHT_WRITE | sys::RIGHT_NET_RAW;

fn fail(message: &str) -> ! {
    sys::println(&format!("[PACKET-AUTH] FAIL {message}"));
    sys::exit(1)
}

fn expect(label: &str, actual: isize, expected: isize) {
    if actual != expected {
        fail(&format!("{label} actual={actual} expected={expected}"));
    }
}

fn owned(label: &str, value: isize) -> u32 {
    if value <= 0 {
        fail(&format!("{label} returned {value}"));
    }
    value as u32
}

fn wait_child(child: u32) {
    let mut status = sys::WaitStatus::empty();
    for _ in 0..128 {
        let result = sys::wait(child, &mut status);
        if result == sys::EAGAIN {
            sys::yield_now();
            continue;
        }
        expect("reap child", result, child as isize);
        if !status.exited() || status.code != 0 {
            fail(&format!("child kind={} code={}", status.kind, status.code));
        }
        return;
    }
    fail("child did not finish in 128 dispatches");
}

pub(crate) fn run() -> ! {
    for syscall in [sys::SYSCALL_NET_DEVICE_GRANT, sys::SYSCALL_NET_QUEUE_OPEN] {
        expect(
            "packet syscall query",
            sys::abi_query(sys::ABI_QUERY_HAS_SYSCALL, syscall),
            1,
        );
    }
    expect(
        "ungranted queue open",
        sys::net_queue_open(0, &QueueOptions.encode()),
        sys::EBADF,
    );
    let child = owned(
        "launch child with forged root environment",
        sys::process::spawn_args_env(
            TEST_APP,
            &[TEST_APP, "--packet-spoof"],
            &["DUNIT_NET_ROOT=1"],
        ),
    );
    wait_child(child);

    for file in ["/system/boot-services/netd.toml", "/system/libexec/netd"] {
        let fd = sys::open(file, sys::OPEN_READ);
        if fd < 0 {
            fail(&format!("boot archive entry missing {file}: {fd}"));
        }
        expect("close protected read", sys::close(fd as usize), 0);
        expect(
            "protected overwrite open",
            sys::open(file, sys::OPEN_WRITE),
            sys::EACCES,
        );
        expect(
            "protected writable capability",
            sys::file_cap::open(file, sys::RIGHT_READ | sys::RIGHT_WRITE),
            sys::EACCES,
        );
    }
    for file in [
        "/system/boot-services/.packet-test",
        "/system/boot-services/../libexec/.packet-test",
    ] {
        expect(
            "protected create",
            sys::open(file, sys::OPEN_CREATE | sys::OPEN_WRITE),
            sys::EACCES,
        );
    }
    let parent = owned(
        "open system directory capability",
        sys::file_cap::open("/system", sys::RIGHT_READ | sys::RIGHT_WRITE),
    );
    for name in ["boot-services", "libexec"] {
        expect(
            "protected writable child capability",
            sys::file_cap::open_child(parent, name, sys::RIGHT_WRITE),
            sys::EACCES,
        );
    }
    expect("close parent capability", sys::handle_close(parent), 0);
    sys::println("[PACKET-AUTH] forged root environment and unrelated object types denied");
    sys::println("[PACKET-AUTH] trusted manifest and service executable mutations denied");
    sys::exit(0)
}

pub(crate) fn spoofed_environment() -> ! {
    if sys::getenv("DUNIT_NET_ROOT") != Some("1") {
        fail("forged root environment missing");
    }
    let child = owned(
        "launch valid grant target",
        sys::process::spawn_args(TEST_APP, &[TEST_APP, "--packet-target"]),
    );
    expect(
        "forged root grant",
        sys::net_device_grant(1, 0, child, DEVICE_RIGHTS),
        sys::EPERM,
    );
    let memory = owned("create memory handle", sys::handle_create_memory(64));
    let display = owned("acquire display handle", sys::handle_display_acquire());
    for other_handle in [memory, display] {
        expect(
            "wrong object grant",
            sys::net_device_grant(other_handle, 0, child, DEVICE_RIGHTS),
            sys::EPERM,
        );
        expect(
            "wrong object queue open",
            sys::net_queue_open(other_handle, &QueueOptions.encode()),
            sys::EPERM,
        );
        expect(
            "close non-packet handle",
            sys::handle_close(other_handle),
            0,
        );
    }
    wait_child(child);
    sys::exit(0)
}
