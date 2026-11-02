#![no_std]
#![no_main]
use core::panic::PanicInfo;
use libdunit::{pointer, syscall3, SYSCALL_READ_POINTER_EVENT};
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::println("[POINTERQ] FAIL");
    libdunit::exit(1)
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    libdunit::println("[POINTERQ] start");
    assert_eq!(pointer::read(0), Err(libdunit::EACCES));
    assert_eq!(pointer::read_with_modifiers(0), Err(libdunit::EACCES));
    let handle = libdunit::handle_input_acquire();
    assert!(handle > 0);
    let mut record = [0u8; pointer::RECORD_WITH_MODIFIERS_LEN];
    for invalid in [31, 33, 39, 41] {
        assert_eq!(
            syscall3(
                SYSCALL_READ_POINTER_EVENT,
                handle as usize,
                record.as_mut_ptr() as usize,
                invalid
            ),
            libdunit::EINVAL
        );
    }
    assert_eq!(
        syscall3(SYSCALL_READ_POINTER_EVENT, handle as usize, 0, 32),
        libdunit::EFAULT
    );
    assert_eq!(
        syscall3(SYSCALL_READ_POINTER_EVENT, handle as usize, 0, 40),
        libdunit::EFAULT
    );
    let initial = pointer::read(handle as u32).unwrap().unwrap();
    assert_eq!(initial.flags, pointer::LOST_BEFORE);
    assert_eq!(pointer::read(handle as u32), Ok(None));
    // A child has no INPUT_MASTER handle even when the parent's numeric ID is known.
    let child = libdunit::spawn("/app/pointer_queue_denied");
    assert!(child > 0);
    let mut status = libdunit::WaitStatus::empty();
    for _ in 0..1000 {
        let result = libdunit::wait(child as u32, &mut status);
        if result >= 0 && status.exited() {
            break;
        }
        libdunit::sleep_ms(10);
    }
    assert!(status.exited());
    assert_eq!(status.code, 0);
    libdunit::println("[POINTERQ] access and failed-copy preservation OK");
    libdunit::println("[POINTERQ] sleeping; inject complete short click");
    // Both transitions must arrive while this process cannot poll input.
    libdunit::sleep_ms(1500);
    let mut down = None;
    let mut up = None;
    while let Some(event) = pointer::read(handle as u32).unwrap() {
        if event.buttons & 1 != 0 {
            down = Some(event);
        } else if down.is_some() {
            up = Some(event);
        }
    }
    let down = down.unwrap();
    let up = up.unwrap();
    assert!(initial.sequence < down.sequence && down.sequence < up.sequence);
    assert_eq!((down.x, down.y), (up.x, up.y));
    libdunit::println("[POINTERQ] short down/up retained while sleeping OK");
    while libdunit::get_key_event().is_some() {}
    libdunit::println("[POINTERQ] sleeping; inject Shift and Ctrl clicks then release modifiers");
    libdunit::sleep_ms(1500);
    let mut shift_released = false;
    let mut ctrl_released = false;
    while let Some(key) = libdunit::get_key_event() {
        shift_released |=
            key.scancode == 0x2a && !key.pressed && key.mods & libdunit::KEYMOD_SHIFT == 0;
        ctrl_released |=
            key.scancode == 0x1d && !key.pressed && key.mods & libdunit::KEYMOD_CTRL == 0;
    }
    // This assertion proves releases reached the device before any pointer read.
    assert!(shift_released && ctrl_released);
    assert_eq!(
        syscall3(SYSCALL_READ_POINTER_EVENT, handle as usize, 0, 40),
        libdunit::EFAULT
    );
    let mut previous = up.sequence;
    for modifiers in [libdunit::KEYMOD_SHIFT, libdunit::KEYMOD_CTRL, 0] {
        let down = pointer::read_with_modifiers(handle as u32)
            .unwrap()
            .unwrap();
        let up = pointer::read_with_modifiers(handle as u32)
            .unwrap()
            .unwrap();
        assert_eq!((down.event.buttons, up.event.buttons), (1, 0));
        assert_eq!(down.modifiers, modifiers as u32);
        assert_eq!(up.modifiers, modifiers as u32);
        assert_eq!(down.event.flags, 0);
        assert_eq!(up.event.flags, 0);
        assert!(previous < down.event.sequence && down.event.sequence < up.event.sequence);
        previous = up.event.sequence;
    }
    assert_eq!(pointer::read_with_modifiers(handle as u32), Ok(None));
    libdunit::println("[POINTERQ] modifier snapshots survive key releases and failed v2 copy OK");
    assert_eq!(libdunit::handle_close(handle as u32), 0);
    assert_eq!(pointer::read(handle as u32), Err(libdunit::EACCES));
    let fresh = libdunit::handle_input_acquire();
    assert!(fresh > 0);
    let fresh_event = pointer::read_with_modifiers(fresh as u32).unwrap().unwrap();
    assert_eq!(fresh_event.event.flags, pointer::LOST_BEFORE);
    assert_eq!(fresh_event.modifiers, 0);
    assert_eq!(pointer::read_with_modifiers(fresh as u32), Ok(None));
    assert_eq!(libdunit::handle_close(fresh as u32), 0);
    libdunit::println("[POINTERQ] handover and queue OK");
    libdunit::exit(0)
}
