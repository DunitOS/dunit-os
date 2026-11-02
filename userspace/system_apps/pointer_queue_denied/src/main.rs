#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::exit(1)
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
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
