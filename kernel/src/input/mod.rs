//! Device facts, pointer queue and exclusive input leases share one IRQ-safe
//! transaction. Task/VM/display scopes may enter it; callbacks must not acquire
//! those locks, allocate, block or retain a device reference.
mod pointer_queue;
use pointer_queue::PointerQueue;

struct InputState {
    owner: u64,
    leases: usize,
    generation: u64,
    x: i32,
    y: i32,
    max_x: i32,
    max_y: i32,
    buttons: u8,
    wheel: i32,
    relative: bool,
    queue: PointerQueue,
}
impl InputState {
    fn clamp(&self, x: i32, y: i32) -> (i32, i32) {
        (x.clamp(0, self.max_x), y.clamp(0, self.max_y))
    }
}
static STATE: crate::sync::IrqSafeSpinLock<InputState> =
    crate::sync::IrqSafeSpinLock::new(InputState {
        owner: 0,
        leases: 0,
        generation: 0,
        x: 0,
        y: 0,
        max_x: 0,
        max_y: 0,
        buttons: 0,
        wheel: 0,
        relative: false,
        queue: PointerQueue::new(),
    });

pub(crate) fn try_acquire(pid: u64) -> bool {
    if pid == 0 {
        return false;
    }
    // Snapshot keyboard facts before acquiring input, never input -> keyboard.
    let _irq = crate::sync::InterruptGuard::new();
    let modifiers = crate::drivers::keyboard::modifier_mask() as u32;
    let mut state = STATE.lock();
    if state.owner != 0 && state.owner != pid {
        return false;
    }
    let Some(leases) = state.leases.checked_add(1) else {
        return false;
    };
    if state.owner == 0 {
        let Some(generation) = state.generation.checked_add(1) else {
            return false;
        };
        let (x, y, buttons, relative) = (state.x, state.y, state.buttons, state.relative);
        state
            .queue
            .reset_motion(x as u32, y as u32, buttons as u32, modifiers, relative);
        state.generation = generation;
        state.owner = pid;
    }
    state.leases = leases;
    true
}
pub(crate) fn release(pid: u64) {
    let mut state = STATE.lock();
    if pid != 0 && state.owner == pid {
        state.leases -= 1;
        if state.leases == 0 {
            state.queue.clear();
            state.owner = 0;
        }
    }
}
#[cfg(test)]
pub(crate) fn owner() -> u64 {
    STATE.lock().owner
}

/// Copied authorization, not a guard. Revalidate after pinning the destination
/// so a delayed syscall cannot drain a later lease of the same PID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReadToken {
    owner: u64,
    generation: u64,
}
pub(crate) fn read_token(pid: u64) -> Option<ReadToken> {
    let state = STATE.lock();
    (pid != 0 && state.owner == pid).then_some(ReadToken {
        owner: pid,
        generation: state.generation,
    })
}

/// Keep the front record until the already-pinned user copy succeeds. No
/// task/VM lookup is allowed inside the callback while input is locked.
pub(crate) fn read_pointer_event(
    token: ReadToken,
    copy: impl FnOnce(&[u8; dunit_pointer_abi::RECORD_LEN]) -> Result<(), i64>,
) -> Result<bool, i64> {
    consume_pointer_event(token, |event| copy(&event.event.encode()))
}
pub(crate) fn read_pointer_event_with_modifiers(
    token: ReadToken,
    copy: impl FnOnce(&[u8; dunit_pointer_abi::RECORD_WITH_MODIFIERS_LEN]) -> Result<(), i64>,
) -> Result<bool, i64> {
    consume_pointer_event(token, |event| copy(&event.legacy().encode()))
}
pub(crate) fn read_pointer_event_with_motion(
    token: ReadToken,
    copy: impl FnOnce(&[u8; dunit_pointer_abi::RECORD_WITH_MOTION_LEN]) -> Result<(), i64>,
) -> Result<bool, i64> {
    consume_pointer_event(token, |event| copy(&event.encode()))
}
fn consume_pointer_event(
    token: ReadToken,
    copy: impl FnOnce(dunit_pointer_abi::PointerEventWithMotion) -> Result<(), i64>,
) -> Result<bool, i64> {
    let mut state = STATE.lock();
    if state.owner != token.owner || state.generation != token.generation {
        return Err(crate::syscall::EACCES);
    }
    let Some(event) = state.queue.peek() else {
        return Ok(false);
    };
    copy(event)?;
    state.queue.pop();
    Ok(true)
}

pub fn set_mouse_relative_source(relative: bool) {
    STATE.lock().relative = relative;
}
pub fn set_mouse_bounds(width: usize, height: usize) {
    let mut state = STATE.lock();
    state.max_x = width.saturating_sub(1).min(i32::MAX as usize) as i32;
    state.max_y = height.saturating_sub(1).min(i32::MAX as usize) as i32;
    (state.x, state.y) = state.clamp(state.x, state.y);
}
pub fn set_mouse_position(x: i32, y: i32) {
    let mut state = STATE.lock();
    (state.x, state.y) = state.clamp(x, y);
}
pub fn mouse_position() -> (i32, i32) {
    let state = STATE.lock();
    (state.x, state.y)
}
pub fn mouse_buttons() -> u8 {
    STATE.lock().buttons
}
pub fn take_mouse_scroll_delta() -> i32 {
    core::mem::take(&mut STATE.lock().wheel)
}

/// Compatibility snapshot: preserve its existing access policy, but publish
/// one coherent packet and drain wheel only after the pinned copy succeeds.
pub(crate) fn read_mouse_state(copy: impl FnOnce(&[u8; 16]) -> Result<(), i64>) -> Result<(), i64> {
    let mut state = STATE.lock();
    let mut record = [0; 16];
    record[..4].copy_from_slice(&(state.x as u32).to_le_bytes());
    record[4..8].copy_from_slice(&(state.y as u32).to_le_bytes());
    record[8..12].copy_from_slice(&(state.buttons as u32).to_le_bytes());
    record[12..16].copy_from_slice(&state.wheel.to_le_bytes());
    copy(&record)?;
    state.wheel = 0;
    Ok(())
}

pub fn push_mouse_relative(dx: i32, dy: i32, buttons: u8, wheel: i32) {
    let _irq = crate::sync::InterruptGuard::new();
    let modifiers = crate::drivers::keyboard::modifier_mask() as u32;
    let mut state = STATE.lock();
    let (x, y) = state.clamp(state.x.saturating_add(dx), state.y.saturating_add(dy));
    state.relative = true;
    state.x = x;
    state.y = y;
    state.buttons = buttons;
    state.wheel = state.wheel.wrapping_add(wheel);
    state.queue.push_motion(
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
    let _irq = crate::sync::InterruptGuard::new();
    let modifiers = crate::drivers::keyboard::modifier_mask() as u32;
    let mut state = STATE.lock();
    let (x, y) = state.clamp(x, y);
    state.relative = false;
    state.x = x;
    state.y = y;
    state.buttons = buttons;
    state.wheel = state.wheel.wrapping_add(wheel);
    state
        .queue
        .push(x as u32, y as u32, buttons as u32, wheel, modifiers);
}
