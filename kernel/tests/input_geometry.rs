//! Exercises the actual hardware-independent input module on the host.
// Host threads use a mutex; CLI/STI and interrupt safety are checked in QEMU.
mod sync {
    pub struct InterruptGuard;
    impl InterruptGuard {
        pub fn new() -> Self {
            Self
        }
    }
    pub struct IrqSafeSpinLock<T>(std::sync::Mutex<T>);
    impl<T> IrqSafeSpinLock<T> {
        pub const fn new(value: T) -> Self {
            Self(std::sync::Mutex::new(value))
        }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
    }
}
mod drivers {
    pub mod keyboard {
        pub static MODIFIERS: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
        pub fn modifier_mask() -> u8 {
            MODIFIERS.load(core::sync::atomic::Ordering::Relaxed)
        }
    }
}
#[path = "../src/input/mod.rs"]
#[allow(dead_code)]
mod input;

// One test owns the module's global pointer state, avoiding parallel-test races.
#[test]
fn pointer_tracks_display_geometry() {
    assert_eq!(input::mouse_position(), (0, 0));
    input::push_mouse_relative(100, 100, 0, 0);
    assert_eq!(input::mouse_position(), (0, 0));

    input::set_mouse_bounds(1920, 1080);
    input::push_mouse_relative(i32::MAX, i32::MAX, 1, 2);
    assert_eq!(input::mouse_position(), (1919, 1079));
    assert_eq!(input::mouse_buttons(), 1);
    assert_eq!(input::take_mouse_scroll_delta(), 2);
    input::push_mouse_relative(i32::MIN, i32::MIN, 0, 0);
    assert_eq!(input::mouse_position(), (0, 0));

    input::push_mouse_absolute(1800, 1000, 0, 0);
    input::set_mouse_bounds(800, 600);
    assert_eq!(input::mouse_position(), (799, 599));
    input::set_mouse_bounds(2560, 1440);
    assert_eq!(input::mouse_position(), (799, 599));
    input::push_mouse_relative(i32::MAX, i32::MAX, 0, 0);
    assert_eq!(input::mouse_position(), (2559, 1439));
    input::push_mouse_absolute(-1, 2000, 0, 0);
    assert_eq!(input::mouse_position(), (0, 1439));

    input::clear_pointer_queue();
    input::push_mouse_absolute(2000, 1000, 2, -1);
    assert_eq!(input::read_pointer_event(|_| Err(-14)), Err(-14));
    let mut event = None;
    assert_eq!(
        input::read_pointer_event(|record| {
            event = dunit_pointer_abi::PointerEvent::decode(record);
            Ok(())
        }),
        Ok(true)
    );
    let event = event.unwrap();
    assert_eq!((event.x, event.y), (2000, 1000));
    assert_eq!((event.buttons, event.wheel), (2, -1));
    assert_eq!(
        input::read_pointer_event(|_| panic!("empty queue copied")),
        Ok(false)
    );

    // Snapshot at enqueue survives later key changes and failed v2 user copies.
    drivers::keyboard::MODIFIERS.store(3, core::sync::atomic::Ordering::Relaxed);
    input::push_mouse_absolute(50, 60, 1, 0);
    drivers::keyboard::MODIFIERS.store(0, core::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        input::read_pointer_event_with_modifiers(|_| Err(-14)),
        Err(-14)
    );
    let mut captured = None;
    assert_eq!(
        input::read_pointer_event_with_modifiers(|bytes| {
            captured = dunit_pointer_abi::PointerEventWithModifiers::decode(bytes);
            Ok(())
        }),
        Ok(true)
    );
    let captured = captured.unwrap();
    assert_eq!(captured.modifiers, 3);
    assert_eq!(
        (captured.event.x, captured.event.y, captured.event.buttons),
        (50, 60, 1)
    );
    assert_eq!(
        input::read_pointer_event_with_modifiers(|_| panic!("empty v2 queue copied")),
        Ok(false)
    );
    drivers::keyboard::MODIFIERS.store(2, core::sync::atomic::Ordering::Relaxed);
    input::reset_pointer_queue();
    drivers::keyboard::MODIFIERS.store(0, core::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        input::read_pointer_event_with_modifiers(|bytes| {
            let reset = dunit_pointer_abi::PointerEventWithModifiers::decode(bytes).unwrap();
            assert_eq!(reset.modifiers, 2);
            assert_eq!(reset.event.flags, dunit_pointer_abi::LOST_BEFORE);
            Ok(())
        }),
        Ok(true)
    );

    input::set_mouse_bounds(0, 0);
    assert_eq!(input::mouse_position(), (0, 0));
    input::set_mouse_bounds(1, 1);
    input::push_mouse_absolute(10, 10, 0, 0);
    assert_eq!(input::mouse_position(), (0, 0));
    input::set_mouse_bounds(usize::MAX, usize::MAX);
    input::push_mouse_relative(i32::MAX, i32::MAX, 0, 0);
    assert_eq!(input::mouse_position(), (i32::MAX, i32::MAX));
}
