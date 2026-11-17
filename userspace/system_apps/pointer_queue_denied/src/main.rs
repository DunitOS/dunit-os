#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::exit(1)
}
#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: libdunit::RawArgv, _envp: libdunit::RawEnvp) -> ! {
    if unsafe { libdunit::argv_get(argc, argv, 1) } == Some("lease") {
        let mut request = [0; 4];
        let mut parent = 0;
        assert_eq!(
            libdunit::ipc_recv_blocking_from(&mut request, &mut parent, 5000),
            4
        );
        assert!(request == *b"exit" || request == *b"kill");
        let first = libdunit::handle_input_acquire();
        let second = libdunit::handle_input_acquire();
        assert!(first > 0 && second > 0 && first != second);
        let reset = libdunit::pointer::read(first as u32).unwrap().unwrap();
        assert_eq!(reset.flags, libdunit::pointer::LOST_BEFORE);
        assert_eq!(libdunit::pointer::read(second as u32), Ok(None));
        assert_eq!(libdunit::ipc_send(parent, b"ready"), 5);
        if request == *b"kill" {
            loop {
                libdunit::sleep_ms(1000);
            }
        }
        // Deliberately retain both handles so process cleanup releases them.
        libdunit::exit(0);
    }
    // Probe all small numeric IDs, including the parent's singleton handle.
    for handle in 0..32 {
        assert_eq!(libdunit::pointer::read(handle), Err(libdunit::EACCES));
        assert_eq!(
            libdunit::pointer::read_with_modifiers(handle),
            Err(libdunit::EACCES)
        );
    }
    assert!(libdunit::handle_input_acquire() < 0);
    libdunit::exit(0)
}
