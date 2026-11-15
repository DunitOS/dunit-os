//! Actual display/console locks and Bochs programming with emulated port I/O.
//! Host memory stands in for VRAM. CR3/TLB, device scanout and IRQs need QEMU.
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Mutex, Once,
};
use std::time::Duration;

#[allow(dead_code)]
#[path = "../src/sync.rs"]
mod actual_sync;
mod sync {
    pub use crate::actual_sync::SpinLock;
    pub struct InterruptGuard;
    impl InterruptGuard {
        pub fn new() -> Self {
            Self
        }
    }
}
static TEST_LOCK: Mutex<()> = Mutex::new(());
const VRAM_BYTES: usize = 32 * 1024 * 1024;
const CANARY: u32 = 0x12345678;
static VRAM: AtomicUsize = AtomicUsize::new(0);
mod hal {
    use super::*;
    pub struct Device {
        pub index: u16,
        pub registers: [u16; 16],
        pub fail_programs: usize,
        pub fault: bool,
    }
    pub static DEVICE: Mutex<Device> = Mutex::new(Device {
        index: 0,
        registers: [0xB0C5, 96, 64, 32, 65, 0, 96, 0, 0, 0, 512, 0, 0, 0, 0, 0],
        fail_programs: 0,
        fault: false,
    });
    pub unsafe fn hal_outw(port: u16, value: u16) {
        let mut device = DEVICE.lock().unwrap();
        if port == 0x1ce {
            device.index = value;
            return;
        }
        assert_eq!(port, 0x1cf);
        let index = device.index as usize;
        device.registers[index] = value;
        if index == 4 && value & 1 != 0 {
            device.fault = device.fail_programs > 0;
            device.fail_programs = device.fail_programs.saturating_sub(1);
        }
    }
    pub unsafe fn hal_inw(port: u16) -> u16 {
        assert_eq!(port, 0x1cf);
        let device = DEVICE.lock().unwrap();
        let index = device.index as usize;
        if device.registers[4] & 2 != 0 && (index == 1 || index == 2) {
            return 4096;
        }
        if device.fault && index == 1 {
            return device.registers[1] ^ 1;
        }
        device.registers[index]
    }
}
#[path = "../src/drivers/display/mod.rs"]
pub mod display;
mod drivers {
    pub use crate::display;
    #[allow(dead_code)]
    pub mod pci {
        pub struct Device {
            pub vendor_id: u16,
            pub device_id: u16,
            pub class_code: u8,
        }
        pub enum PciBar {
            Memory32(u32),
            Memory64(u64),
            Io(u16),
        }
        pub struct Resource {
            pub bar: PciBar,
            pub size: u64,
        }
        pub fn for_each_device(op: impl FnOnce(&Device)) {
            op(&Device {
                vendor_id: 0x1234,
                device_id: 0x1111,
                class_code: 3,
            });
        }
        pub fn read_bar_info(_: &Device, _: u8) -> Resource {
            Resource {
                bar: PciBar::Memory32(0xe0000000),
                size: crate::VRAM_BYTES as u64,
            }
        }
    }
}
mod memory {
    pub mod vmm {
        use super::super::*;
        pub static MAPS: AtomicUsize = AtomicUsize::new(0);
        pub static UNMAPS: AtomicUsize = AtomicUsize::new(0);
        pub fn map_framebuffer_region(_: usize, length: usize, reference: usize) -> Option<usize> {
            assert_eq!(reference, VRAM.load(Ordering::Acquire));
            assert!(length <= VRAM_BYTES);
            MAPS.fetch_add(1, Ordering::SeqCst);
            Some(reference)
        }
        pub fn unmap_mmio_region(_: usize, _: usize) -> bool {
            UNMAPS.fetch_add(1, Ordering::SeqCst);
            true
        }
    }
}
mod input {
    use super::*;
    pub static BOUNDS: AtomicU64 = AtomicU64::new(0);
    pub fn set_mouse_bounds(width: usize, height: usize) {
        BOUNDS.store((width as u64) << 32 | height as u64, Ordering::Release);
    }
}
mod serial {
    use super::*;
    pub static BYTES: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    pub fn serial_write_byte(byte: u8) {
        // Logs must run after both console and display guards are released.
        let _ = crate::terminal::get_cursor_info();
        BYTES.lock().unwrap().push(byte);
    }
    pub fn serial_write(text: &str) {
        for byte in text.bytes() {
            serial_write_byte(byte);
        }
    }
    pub fn write_dec(value: u64) {
        serial_write(&value.to_string());
    }
}
use serial::serial_write;
#[path = "../src/console_font.rs"]
mod console_font;
#[path = "../src/terminal.rs"]
mod terminal;

fn setup() {
    static INITIALIZE: Once = Once::new();
    INITIALIZE.call_once(|| {
        let pixels = Box::leak(vec![CANARY; VRAM_BYTES / 4 + 128].into_boxed_slice());
        let address = unsafe { pixels.as_mut_ptr().add(64) } as usize;
        VRAM.store(address, Ordering::Release);
        assert!(unsafe { display::init_boot(address, 96, 64, 384) });
        display::init();
        assert!(terminal::init());
    });
    while display::owner() != 0 {
        display::release(display::owner());
    }
    assert!(display::try_acquire(1));
    display::set_mode(1, 96, 64).unwrap();
    display::release(1);
    terminal::console().unwrap().clear_screen();
}
fn pixels() -> Vec<u32> {
    let framebuffer = display::framebuffer_snapshot().unwrap();
    unsafe {
        std::slice::from_raw_parts(
            framebuffer.address as *const u32,
            framebuffer.byte_len().unwrap() / 4,
        )
        .to_vec()
    }
}
fn assert_canaries() {
    let address = VRAM.load(Ordering::Acquire) as *const u32;
    for n in 0..64 {
        unsafe {
            assert_eq!(*address.sub(n + 1), CANARY);
            assert_eq!(*address.add(VRAM_BYTES / 4 + n), CANARY);
        }
    }
}

#[test]
fn display_lease_preserves_scanout_and_console_resumes_after_release() {
    let _test = TEST_LOCK.lock().unwrap();
    setup();
    let mut console = terminal::console().unwrap();
    console.write_str("before\n");
    let before = pixels();
    assert!(display::try_acquire(7));
    console.set_fg_color(0xFF00FF);
    console.draw_char('X');
    console.reset_fg_color();
    console.write_str("during\n");
    console.draw_cursor(true);
    console.draw_cursor(false);
    console.scroll_view(-1);
    console.clear_top_area(usize::MAX);
    console.clear_screen();
    console.set_theme(&term_theme::DEFAULT);
    console.write_display_str("leased\n");
    assert_eq!(
        pixels(),
        before,
        "a published lease excludes every console writer"
    );
    assert!(serial::BYTES
        .lock()
        .unwrap()
        .windows(6)
        .any(|s| s == b"during"));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(move || {
                let mut output = terminal::console().unwrap();
                for _ in 0..200 {
                    output.write_display_str("text\n");
                    output.scroll_view(-1);
                    output.scroll_view(1);
                    assert!(terminal::get_cursor_info().unwrap().y < 64);
                }
            });
        }
    });
    assert_eq!(pixels(), before);
    display::set_mode(7, 128, 80).unwrap();
    display::release(7);
    console.clear_screen();
    console.write_display_str("resumed\n");
    assert_eq!(terminal::get_cursor_info().unwrap().y, 16);
    assert_canaries();
}
#[test]
fn independent_consoles_do_not_share_scrollback() {
    let _test = TEST_LOCK.lock().unwrap();
    setup();
    let mut first = terminal::FbConsole::new(96, 64);
    let mut second = terminal::FbConsole::new(96, 64);
    first.clear_screen();
    first.draw_char('A');
    let before = pixels();
    second.clear_screen();
    second.draw_char('B');
    first.scroll_view(0);
    first.draw_cursor(false);
    assert_eq!(pixels(), before);
}
#[test]
fn repeated_acquisition_keeps_owner_until_last_handle_release() {
    let _test = TEST_LOCK.lock().unwrap();
    setup();
    assert!(!display::try_acquire(0));
    assert!(display::try_acquire(7));
    assert!(display::try_acquire(7));
    display::release(8);
    display::release(0);
    display::release(7);
    assert_eq!(display::owner(), 7);
    assert!(!display::try_acquire(8));
    display::release(7);
    assert!(display::try_acquire(8));
    assert_eq!(
        display::get_framebuffer(7),
        Err(display::DisplayError::AccessDenied)
    );
    display::release(8);
}
#[test]
fn ownership_modeset_and_release_wait_for_active_pixel_scopes() {
    let _test = TEST_LOCK.lock().unwrap();
    setup();
    use std::sync::mpsc::channel;
    // Start with a console writer holding the actual display guard.
    let (entered_tx, entered_rx) = channel();
    let (leave_tx, leave_rx) = channel();
    let (attempt_tx, attempt_rx) = channel();
    let (done_tx, done_rx) = channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            display::with_console_framebuffer(|framebuffer| {
                entered_tx.send(()).unwrap();
                leave_rx.recv().unwrap();
                unsafe {
                    (framebuffer.address as *mut u32).write_volatile(0xff00ff);
                }
            })
        });
        entered_rx.recv().unwrap();
        scope.spawn(move || {
            attempt_tx.send(()).unwrap();
            done_tx.send(display::try_acquire(7)).unwrap();
        });
        attempt_rx.recv().unwrap();
        assert!(
            done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "lease published before writer stopped"
        );
        leave_tx.send(()).unwrap();
        assert!(done_rx.recv_timeout(Duration::from_secs(2)).unwrap());
    });
    for change_mode in [true, false] {
        let (entered_tx, entered_rx) = channel();
        let (leave_tx, leave_rx) = channel();
        let (attempt_tx, attempt_rx) = channel();
        let (done_tx, done_rx) = channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                display::present(7, None, |framebuffer| {
                    entered_tx.send(()).unwrap();
                    leave_rx.recv().unwrap();
                    assert_eq!(framebuffer.width, 96);
                })
                .unwrap()
            });
            entered_rx.recv().unwrap();
            scope.spawn(move || {
                attempt_tx.send(()).unwrap();
                if change_mode {
                    display::set_mode(7, 128, 80).unwrap();
                } else {
                    display::release(7);
                }
                done_tx.send(()).unwrap();
            });
            attempt_rx.recv().unwrap();
            assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
            leave_tx.send(()).unwrap();
            done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        });
        if change_mode {
            display::set_mode(7, 96, 64).unwrap();
        }
    }
    assert_eq!(display::owner(), 0);
    assert_eq!(memory::vmm::UNMAPS.load(Ordering::SeqCst), 0);
}
#[test]
fn rejected_modes_restore_or_quarantine_and_can_recover() {
    let _test = TEST_LOCK.lock().unwrap();
    setup();
    assert!(display::try_acquire(7));
    let before = display::get_framebuffer(7).unwrap();
    assert_eq!(
        display::set_mode(7, 0, 80),
        Err(display::DisplayError::InvalidMode)
    );
    hal::DEVICE.lock().unwrap().fail_programs = 1;
    assert_eq!(
        display::set_mode(7, 128, 80),
        Err(display::DisplayError::Io)
    );
    assert_eq!(display::get_framebuffer(7).unwrap(), before);
    let registers = hal::DEVICE.lock().unwrap().registers;
    assert_eq!(&registers[1..4], &[96, 64, 32]);
    hal::DEVICE.lock().unwrap().fail_programs = 2;
    assert_eq!(
        display::set_mode(7, 128, 80),
        Err(display::DisplayError::DeviceLost)
    );
    assert_eq!(
        display::get_framebuffer(7),
        Err(display::DisplayError::Unavailable)
    );
    assert_eq!(
        display::with_framebuffer(7, |_| panic!("lost scanout touched")),
        Err(display::DisplayError::Unavailable)
    );
    display::release(7);
    terminal::console()
        .unwrap()
        .write_display_str("quarantined\n");
    assert!(display::try_acquire(7));
    display::set_mode(7, 96, 64).unwrap();
    assert!(display::get_framebuffer(7).is_ok());
    display::release(7);
    assert_canaries();
}
#[test]
fn concurrent_modes_and_copies_observe_one_geometry_and_backing() {
    let _test = TEST_LOCK.lock().unwrap();
    setup();
    assert!(display::try_acquire(7));
    let address = display::get_framebuffer(7).unwrap().address;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            for n in 0..2_000 {
                let (width, height) = if n % 2 == 0 { (128, 80) } else { (96, 64) };
                display::set_mode(7, width, height).unwrap();
            }
        });
        for _ in 0..3 {
            scope.spawn(move || {
                for _ in 0..2_000 {
                    display::present(7, None, |framebuffer| {
                        assert_eq!(framebuffer.address, address);
                        assert_eq!(framebuffer.pitch, framebuffer.width * 4);
                        let hardware = hal::DEVICE.lock().unwrap();
                        assert_eq!(framebuffer.width, hardware.registers[1] as u32);
                        assert_eq!(framebuffer.height, hardware.registers[2] as u32);
                        drop(hardware);
                        let last = (framebuffer.byte_len().unwrap() / 4) - 1;
                        unsafe {
                            (framebuffer.address as *mut u32)
                                .add(last)
                                .write_volatile(0xfeedface);
                        }
                    })
                    .unwrap();
                }
            });
        }
    });
    display::release(7);
    assert_canaries();
    assert_eq!(memory::vmm::MAPS.load(Ordering::SeqCst), 1);
    assert_eq!(memory::vmm::UNMAPS.load(Ordering::SeqCst), 0);
}
