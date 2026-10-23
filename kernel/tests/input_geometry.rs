//! Exercises the actual hardware-independent input module on the host.
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

    while input::pop_mouse_event().is_some() {}
    input::push_mouse_absolute(2000, 1000, 2, -1);
    let event = input::pop_mouse_event().unwrap();
    assert_eq!((event.x, event.y), (2000, 1000));
    assert!(event.absolute);
    assert_eq!((event.buttons, event.wheel), (2, -1));

    input::set_mouse_bounds(0, 0);
    assert_eq!(input::mouse_position(), (0, 0));
    input::set_mouse_bounds(1, 1);
    input::push_mouse_absolute(10, 10, 0, 0);
    assert_eq!(input::mouse_position(), (0, 0));
    input::set_mouse_bounds(usize::MAX, usize::MAX);
    input::push_mouse_relative(i32::MAX, i32::MAX, 0, 0);
    assert_eq!(input::mouse_position(), (i32::MAX, i32::MAX));
}
