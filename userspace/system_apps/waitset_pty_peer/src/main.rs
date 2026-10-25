#![no_std]
#![no_main]
use core::panic::PanicInfo;
use libdunit::wait_set::*;
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::exit(101)
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut result = [Event::default()];
    if wait(
        &[Interest::descriptor(0, READ, 91)],
        &mut result,
        2_000_000_000,
    ) != Ok(1)
        || result[0].token != 91
        || result[0].events & READ == 0
    {
        libdunit::exit(1);
    }
    let mut input = [0; 8];
    if libdunit::read(0, &mut input) != 3 || &input[..3] != b"ok\n" {
        libdunit::exit(2);
    }
    if wait(&[Interest::descriptor(1, WRITE, 92)], &mut result, 0) != Ok(1)
        || result[0].events & WRITE == 0
    {
        libdunit::exit(3);
    }
    libdunit::write(1, b"waitset-pty-ok\n");
    libdunit::exit(0)
}
