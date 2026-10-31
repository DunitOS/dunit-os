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
    let handle = libdunit::handle_input_acquire();
    assert!(handle > 0);
    let mut record = [0u8; pointer::RECORD_LEN];
    assert_eq!(
        syscall3(
            SYSCALL_READ_POINTER_EVENT,
            handle as usize,
            record.as_mut_ptr() as usize,
            31
        ),
        libdunit::EINVAL
    );
    assert_eq!(
        syscall3(SYSCALL_READ_POINTER_EVENT, handle as usize, 0, 32),
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
    assert_eq!(libdunit::handle_close(handle as u32), 0);
    assert_eq!(pointer::read(handle as u32), Err(libdunit::EACCES));
    let fresh = libdunit::handle_input_acquire();
    assert!(fresh > 0);
    assert_eq!(
        pointer::read(fresh as u32).unwrap().unwrap().flags,
        pointer::LOST_BEFORE
    );
    assert_eq!(pointer::read(fresh as u32), Ok(None));
    assert_eq!(libdunit::handle_close(fresh as u32), 0);
    libdunit::println("[POINTERQ] handover and queue OK");
    libdunit::exit(0)
}
