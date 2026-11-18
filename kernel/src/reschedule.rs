//! One outstanding generation per target CPU. Delivery is a sticky hint, never
//! an execution-return acknowledgement. No receiver lock is taken by the IPI.

#[path = "reschedule_state.rs"]
mod state;
pub use state::Mailbox;
static MAILBOXES: [Mailbox; super::CPU_CAPACITY] =
    [const { Mailbox::new() }; super::CPU_CAPACITY];
fn local() -> &'static Mailbox { &MAILBOXES[super::index()] }

/// Concurrent senders serialize by admission rather than holding a lock while
/// waiting. Failed transport leaves an outstanding ticket: retry that ticket,
/// never invent a return acknowledgement for an undelivered request.
pub fn request(cpu: usize) -> Option<u64> {
    extern "C" { fn hal_mp_count() -> u64; }
    if cpu >= unsafe { hal_mp_count() } as usize { return None; }
    let generation = MAILBOXES.get(cpu)?.reserve()?;
    retry(cpu, generation);
    Some(generation)
}
pub fn retry(cpu: usize, generation: u64) -> bool {
    extern "C" { fn hal_mp_send_reschedule(cpu: u64) -> i32; }
    MAILBOXES.get(cpu).map(|m| m.outstanding(generation)).unwrap_or(false)
        && unsafe { hal_mp_send_reschedule(cpu as u64) != 0 }
}
pub fn acknowledged(cpu: usize, generation: u64) -> bool {
    MAILBOXES.get(cpu).map(|m| m.acknowledged(generation)).unwrap_or(false)
}
#[no_mangle]
pub extern "C" fn kernel_reschedule_ipi() { local().deliver(); }
pub(crate) fn pending() -> Option<u64> { local().pending() }
pub(crate) fn capture(generation: u64) { local().capture(generation); }
pub(crate) fn returned() { local().returned(); }

#[cfg(feature = "boot-smoke-tests")]
#[path = "irq_progress_smoke.rs"]
mod smoke;
#[cfg(feature = "boot-smoke-tests")]
pub(crate) use smoke::debug_log;
