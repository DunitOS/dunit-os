#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    fail("panic");
}

fn fail(reason: &str) -> ! {
    libdunit::print("[POINTER] FAIL ");
    libdunit::println(reason);
    libdunit::exit(1);
}

fn mode(width: u32, height: u32) {
    if libdunit::set_video_mode(width, height) != 0 {
        fail("modeset");
    }
}

fn expect(x: u32, y: u32, buttons: u32) {
    let state = libdunit::get_mouse_state();
    if (state.x, state.y, state.buttons) != (x, y, buttons) {
        fail("unexpected pointer state after modeset");
    }
}

fn await_state(marker: &str, x: u32, y: u32, buttons: u32) {
    libdunit::println(marker);
    for _ in 0..2000 {
        let state = libdunit::get_mouse_state();
        if (state.x, state.y, state.buttons) == (x, y, buttons) {
            return;
        }
        libdunit::sleep_ms(10);
    }
    fail("injected pointer state not received");
}

#[no_mangle]
pub extern "C" fn _start(
    argc: usize,
    argv: libdunit::RawArgv,
    envp: libdunit::RawEnvp,
) -> ! {
    libdunit::init_runtime(argc, argv, envp);
    if libdunit::handle_display_acquire() < 0 {
        fail("display acquire");
    }
    mode(1920, 1080);
    await_state("[POINTER] WAIT top-left", 0, 0, 0);
    await_state("[POINTER] WAIT top-right", 1919, 0, 0);
    await_state("[POINTER] WAIT bottom-right", 1919, 1079, 0);
    await_state("[POINTER] WAIT bottom-left", 0, 1079, 0);
    await_state("[POINTER] WAIT click-target", 1919, 1079, 0);
    libdunit::println("[POINTER] PASS four corners");
    await_state("[POINTER] WAIT left-down", 1919, 1079, 1);
    await_state("[POINTER] WAIT left-up", 1919, 1079, 0);
    await_state("[POINTER] WAIT right-down", 1919, 1079, 2);
    await_state("[POINTER] WAIT right-up", 1919, 1079, 0);
    libdunit::println("[POINTER] PASS bottom-right buttons");
    mode(1280, 720);
    expect(1279, 719, 0);
    libdunit::println("[POINTER] PASS shrink clamp");
    mode(1920, 1080);
    expect(1279, 719, 0);
    await_state("[POINTER] WAIT grown-bottom-right", 1919, 1079, 0);
    libdunit::println("[POINTER] PASS grow reaches edge");
    libdunit::println("[POINTER] PASS all");
    libdunit::exit(0);
}
