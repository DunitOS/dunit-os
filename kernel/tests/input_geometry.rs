//! Exercises the actual hardware-independent input module on the host.
// Host threads use the actual SpinLock; CLI/STI need QEMU.
#[allow(dead_code)]
#[path = "../src/sync.rs"]
mod actual_sync;
mod sync {
    pub struct InterruptGuard;
    impl InterruptGuard {
        pub fn new() -> Self {
            Self
        }
    }
    pub use crate::actual_sync::SpinLock as IrqSafeSpinLock;
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

mod syscall {
    pub const EACCES: i64 = -13;
}
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// One test owns the module's global pointer state, avoiding parallel-test races.
#[test]
fn pointer_tracks_display_geometry() {
    let _test = TEST_LOCK.lock().unwrap();
    input::set_mouse_bounds(0, 0);
    input::push_mouse_absolute(0, 0, 0, 0);
    input::take_mouse_scroll_delta();
    assert!(input::try_acquire(7));
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

    input::release(7);
    assert!(input::try_acquire(7));
    let token = input::read_token(7).unwrap();
    assert_eq!(input::read_pointer_event(token, |_| Ok(())), Ok(true));
    input::push_mouse_absolute(2000, 1000, 2, -1);
    assert_eq!(input::read_pointer_event(token, |_| Err(-14)), Err(-14));
    let mut event = None;
    assert_eq!(
        input::read_pointer_event(token, |record| {
            event = dunit_pointer_abi::PointerEvent::decode(record);
            Ok(())
        }),
        Ok(true)
    );
    let event = event.unwrap();
    assert_eq!((event.x, event.y), (2000, 1000));
    assert_eq!((event.buttons, event.wheel), (2, -1));
    assert_eq!(
        input::read_pointer_event(token, |_| panic!("empty queue copied")),
        Ok(false)
    );

    // Snapshot at enqueue survives later key changes and failed v2 user copies.
    drivers::keyboard::MODIFIERS.store(3, core::sync::atomic::Ordering::Relaxed);
    input::push_mouse_absolute(50, 60, 1, 0);
    drivers::keyboard::MODIFIERS.store(0, core::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        input::read_pointer_event_with_modifiers(token, |_| Err(-14)),
        Err(-14)
    );
    let mut captured = None;
    assert_eq!(
        input::read_pointer_event_with_modifiers(token, |bytes| {
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
        input::read_pointer_event_with_modifiers(token, |_| panic!("empty v2 queue copied")),
        Ok(false)
    );
    drivers::keyboard::MODIFIERS.store(2, core::sync::atomic::Ordering::Relaxed);
    input::release(7);
    assert!(input::try_acquire(7));
    let token = input::read_token(7).unwrap();
    drivers::keyboard::MODIFIERS.store(0, core::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        input::read_pointer_event_with_modifiers(token, |bytes| {
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
    input::release(7);
}

fn acquire(pid: u64) -> input::ReadToken {
    assert_eq!(input::owner(), 0);
    drivers::keyboard::MODIFIERS.store(0, core::sync::atomic::Ordering::Relaxed);
    input::set_mouse_bounds(1024, 1024);
    input::push_mouse_absolute(0, 0, 0, 0);
    input::take_mouse_scroll_delta();
    assert!(input::try_acquire(pid));
    input::read_token(pid).unwrap()
}

#[test]
fn leases_preserve_pending_data_and_reject_stale_same_pid_reads() {
    let _test = TEST_LOCK.lock().unwrap();
    assert!(!input::try_acquire(0));
    let token = acquire(7);
    assert_eq!(input::read_pointer_event(token, |_| Ok(())), Ok(true));
    input::push_mouse_absolute(5, 6, 1, 2);
    assert!(input::try_acquire(7));
    input::release(8);
    input::release(7);
    assert_eq!(input::owner(), 7);
    assert!(!input::try_acquire(8));
    assert_eq!(input::read_pointer_event(token, |_| Err(-14)), Err(-14));
    assert_eq!(
        input::read_pointer_event(token, |bytes| {
            let event = dunit_pointer_abi::PointerEvent::decode(bytes).unwrap();
            assert_eq!(
                (event.x, event.y, event.buttons, event.wheel, event.flags),
                (5, 6, 1, 2, 0)
            );
            Ok(())
        }),
        Ok(true)
    );
    input::release(7);
    assert_eq!(input::owner(), 0);
    assert_eq!(input::read_token(7), None);
    input::release(7); // Closing an old owner must not affect a new lease.
    for pid in [7, 8] {
        assert!(input::try_acquire(pid));
        assert_eq!(
            input::read_pointer_event(token, |_| panic!("stale reader copied")),
            Err(-13)
        );
        let fresh = input::read_token(pid).unwrap();
        assert_eq!(
            input::read_pointer_event_with_motion(fresh, |bytes| {
                let event = dunit_pointer_abi::PointerEventWithMotion::decode(bytes).unwrap();
                assert_eq!(event.event.flags, dunit_pointer_abi::LOST_BEFORE);
                assert_eq!(
                    (
                        event.event.x,
                        event.event.y,
                        event.event.buttons,
                        event.event.wheel
                    ),
                    (5, 6, 1, 0)
                );
                assert_eq!((event.dx, event.dy), (0, 0));
                Ok(())
            }),
            Ok(true)
        );
        input::release(pid);
    }
}

#[test]
fn final_release_and_handover_wait_for_active_copy() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let _test = TEST_LOCK.lock().unwrap();
    let token = acquire(7);
    let (entered_tx, entered_rx) = channel();
    let (leave_tx, leave_rx) = channel();
    let (attempt_tx, attempt_rx) = channel();
    let (done_tx, done_rx) = channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            assert_eq!(
                input::read_pointer_event(token, |_| {
                    entered_tx.send(()).unwrap();
                    // Host-only delay models an in-flight copy, never kernel policy.
                    leave_rx.recv().unwrap();
                    Ok(())
                }),
                Ok(true)
            );
        });
        entered_rx.recv().unwrap();
        scope.spawn(move || {
            attempt_tx.send(()).unwrap();
            input::release(7);
            assert!(input::try_acquire(8));
            done_tx.send(()).unwrap();
        });
        attempt_rx.recv().unwrap();
        assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
        leave_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    });
    assert_eq!(
        input::read_pointer_event(token, |_| panic!("retired reader copied")),
        Err(-13)
    );
    let fresh = input::read_token(8).unwrap();
    assert_eq!(
        input::read_pointer_event(fresh, |bytes| {
            assert_eq!(
                dunit_pointer_abi::PointerEvent::decode(bytes)
                    .unwrap()
                    .flags,
                dunit_pointer_abi::LOST_BEFORE
            );
            Ok(())
        }),
        Ok(true)
    );
    input::release(8);
}

#[test]
fn concurrent_acquire_release_and_device_updates_keep_one_owner_and_snapshot() {
    let _test = TEST_LOCK.lock().unwrap();
    let token = acquire(7);
    input::release(7);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for n in 0..5000 {
                input::set_mouse_bounds(n % 1024 + 1, n % 1024 + 1);
                input::push_mouse_relative(1, 1, 0, 1);
                let (x, y) = input::mouse_position();
                assert_eq!(x, y);
            }
        });
        for pid in 10..14 {
            scope.spawn(move || {
                for _ in 0..500 {
                    while !input::try_acquire(pid) {
                        std::thread::yield_now();
                    }
                    assert_eq!(input::owner(), pid);
                    let fresh = input::read_token(pid).unwrap();
                    assert_eq!(
                        input::read_pointer_event(token, |_| panic!("old generation copied")),
                        Err(-13)
                    );
                    assert_eq!(
                        input::read_pointer_event(fresh, |bytes| {
                            let event = dunit_pointer_abi::PointerEvent::decode(bytes).unwrap();
                            assert_eq!(event.x, event.y);
                            assert_eq!(event.flags, dunit_pointer_abi::LOST_BEFORE);
                            Ok(())
                        }),
                        Ok(true)
                    );
                    input::release(pid);
                }
            });
        }
    });
    assert_eq!(input::owner(), 0);
    assert_eq!(input::take_mouse_scroll_delta(), 5000);
}

#[test]
fn compatibility_snapshot_keeps_wheel_on_copy_error_and_copies_one_packet() {
    let _test = TEST_LOCK.lock().unwrap();
    let token = acquire(7);
    input::push_mouse_absolute(10, 20, 2, -3);
    assert_eq!(input::read_mouse_state(|_| Err(-14)), Err(-14));
    input::push_mouse_absolute(30, 40, 1, 5);
    assert_eq!(
        input::read_mouse_state(|bytes| {
            let words: [u32; 4] = core::array::from_fn(|i| {
                u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
            });
            assert_eq!(words, [30, 40, 1, 2]);
            Ok(())
        }),
        Ok(())
    );
    assert_eq!(input::take_mouse_scroll_delta(), 0);
    // The compatibility accumulator does not consume the native event stream.
    assert_eq!(input::read_pointer_event(token, |_| Ok(())), Ok(true));
    input::release(7);
}
