use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
static MOUSE_RELATIVE: AtomicBool = AtomicBool::new(false);
pub fn set_mouse_relative_source(relative: bool) {
    MOUSE_RELATIVE.store(relative, Ordering::Relaxed);
}

static MOUSE_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_Y: AtomicI32 = AtomicI32::new(0);
// Until a display publishes its geometry there is no addressable pointer area.
static MOUSE_MAX_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_MAX_Y: AtomicI32 = AtomicI32::new(0);
static MOUSE_BUTTONS: AtomicU8 = AtomicU8::new(0);
static MOUSE_WHEEL_DELTA: AtomicI32 = AtomicI32::new(0);
mod pointer_queue;
use pointer_queue::PointerQueue;
static POINTER_QUEUE: crate::sync::IrqSafeSpinLock<PointerQueue> =
    crate::sync::IrqSafeSpinLock::new(PointerQueue::new());

pub fn reset_pointer_queue() {
    let _irq = crate::sync::InterruptGuard::new();
    let (x, y) = mouse_position();
    let modifiers = crate::drivers::keyboard::modifier_mask() as u32;
    POINTER_QUEUE.lock().reset_motion(
        x as u32,
        y as u32,
        mouse_buttons() as u32,
        modifiers,
        MOUSE_RELATIVE.load(Ordering::Relaxed),
    );
}
pub fn clear_pointer_queue() {
    POINTER_QUEUE.lock().clear();
}

/// Keep the front record until the user copy succeeds, including EFAULT paths.
pub fn read_pointer_event(
    copy: impl FnOnce(&[u8; dunit_pointer_abi::RECORD_LEN]) -> Result<(), i64>,
) -> Result<bool, i64> {
    consume_pointer_event(|event| copy(&event.event.encode()))
}
pub fn read_pointer_event_with_modifiers(
    copy: impl FnOnce(&[u8; dunit_pointer_abi::RECORD_WITH_MODIFIERS_LEN]) -> Result<(), i64>,
) -> Result<bool, i64> {
    consume_pointer_event(|event| copy(&event.legacy().encode()))
}
pub fn read_pointer_event_with_motion(
    copy: impl FnOnce(&[u8; dunit_pointer_abi::RECORD_WITH_MOTION_LEN]) -> Result<(), i64>,
) -> Result<bool, i64> {
    consume_pointer_event(|event| copy(&event.encode()))
}
fn consume_pointer_event(
    copy: impl FnOnce(dunit_pointer_abi::PointerEventWithMotion) -> Result<(), i64>,
) -> Result<bool, i64> {
    let mut queue = POINTER_QUEUE.lock();
    let Some(event) = queue.peek() else {
        return Ok(false);
    };
    copy(event)?;
    queue.pop();
    Ok(true)
}

pub fn set_mouse_bounds(width: usize, height: usize) {
    MOUSE_MAX_X.store(
        width.saturating_sub(1).min(i32::MAX as usize) as i32,
        Ordering::Relaxed,
    );
    MOUSE_MAX_Y.store(
        height.saturating_sub(1).min(i32::MAX as usize) as i32,
        Ordering::Relaxed,
    );
    clamp_mouse_position();
}

pub fn set_mouse_position(x: i32, y: i32) {
    let (x, y) = clamp_mouse_xy(x, y);
    MOUSE_X.store(x, Ordering::Relaxed);
    MOUSE_Y.store(y, Ordering::Relaxed);
}

pub fn mouse_position() -> (i32, i32) {
    (
        MOUSE_X.load(Ordering::Relaxed),
        MOUSE_Y.load(Ordering::Relaxed),
    )
}

pub fn mouse_buttons() -> u8 {
    MOUSE_BUTTONS.load(Ordering::Relaxed)
}

pub fn take_mouse_scroll_delta() -> i32 {
    MOUSE_WHEEL_DELTA.swap(0, Ordering::Relaxed)
}

pub fn push_mouse_relative(dx: i32, dy: i32, buttons: u8, wheel: i32) {
    set_mouse_relative_source(true);
    let _irq = crate::sync::InterruptGuard::new();
    let modifiers = crate::drivers::keyboard::modifier_mask() as u32;
    let current_x = MOUSE_X.load(Ordering::Relaxed);
    let current_y = MOUSE_Y.load(Ordering::Relaxed);
    let (x, y) = clamp_mouse_xy(current_x.saturating_add(dx), current_y.saturating_add(dy));

    MOUSE_X.store(x, Ordering::Relaxed);
    MOUSE_Y.store(y, Ordering::Relaxed);
    MOUSE_BUTTONS.store(buttons, Ordering::Relaxed);
    if wheel != 0 {
        MOUSE_WHEEL_DELTA.fetch_add(wheel, Ordering::Relaxed);
    }

    POINTER_QUEUE.lock().push_motion(
        x as u32,
        y as u32,
        buttons as u32,
        wheel,
        modifiers,
        dx,
        dy,
        true,
    );
}

pub fn push_mouse_absolute(x: i32, y: i32, buttons: u8, wheel: i32) {
    set_mouse_relative_source(false);
    let _irq = crate::sync::InterruptGuard::new();
    let modifiers = crate::drivers::keyboard::modifier_mask() as u32;
    let (x, y) = clamp_mouse_xy(x, y);

    MOUSE_X.store(x, Ordering::Relaxed);
    MOUSE_Y.store(y, Ordering::Relaxed);
    MOUSE_BUTTONS.store(buttons, Ordering::Relaxed);
    if wheel != 0 {
        MOUSE_WHEEL_DELTA.fetch_add(wheel, Ordering::Relaxed);
    }

    POINTER_QUEUE
        .lock()
        .push(x as u32, y as u32, buttons as u32, wheel, modifiers);
}

fn clamp_mouse_position() {
    let x = MOUSE_X.load(Ordering::Relaxed);
    let y = MOUSE_Y.load(Ordering::Relaxed);
    let (x, y) = clamp_mouse_xy(x, y);
    MOUSE_X.store(x, Ordering::Relaxed);
    MOUSE_Y.store(y, Ordering::Relaxed);
}

fn clamp_mouse_xy(x: i32, y: i32) -> (i32, i32) {
    let max_x = MOUSE_MAX_X.load(Ordering::Relaxed);
    let max_y = MOUSE_MAX_Y.load(Ordering::Relaxed);
    (x.max(0).min(max_x), y.max(0).min(max_y))
}
