//! Atomics shared by the real IPI, syscall boundary and post-HAL return path.
use core::sync::atomic::{AtomicU64, Ordering};
pub struct Mailbox {
    requested: AtomicU64,
    delivered: AtomicU64,
    captured: AtomicU64,
    acknowledged: AtomicU64,
}
impl Mailbox {
    pub const fn new() -> Self {
        Self { requested: AtomicU64::new(0), delivered: AtomicU64::new(0),
            captured: AtomicU64::new(0), acknowledged: AtomicU64::new(0) }
    }
    pub fn reserve(&self) -> Option<u64> {
        let ack = self.acknowledged.load(Ordering::Acquire);
        let next = ack.checked_add(1).expect("reschedule generation exhausted");
        self.requested.compare_exchange(ack, next, Ordering::AcqRel, Ordering::Acquire)
            .ok().map(|_| next)
    }
    pub fn outstanding(&self, generation: u64) -> bool {
        generation != 0 && self.requested.load(Ordering::Acquire) == generation
            && !self.acknowledged(generation)
    }
    pub fn deliver(&self) {
        let request = self.requested.load(Ordering::Acquire);
        self.delivered.fetch_max(request, Ordering::Release);
    }
    pub fn pending(&self) -> Option<u64> {
        let delivered = self.delivered.load(Ordering::Acquire);
        (delivered > self.acknowledged.load(Ordering::Acquire)).then_some(delivered)
    }
    /// CPU-local boundary, called only after committing a safe HAL handback.
    pub fn capture(&self, generation: u64) {
        assert_eq!(self.pending(), Some(generation));
        self.captured.store(generation, Ordering::Release);
    }
    /// CPU-local, strictly after HAL, FS/FPU pointers and execution lease return.
    pub fn returned(&self) {
        let captured = self.captured.swap(0, Ordering::AcqRel);
        if captured != 0 { self.acknowledged.store(captured, Ordering::Release); }
    }
    pub fn acknowledged(&self, generation: u64) -> bool {
        generation != 0 && self.acknowledged.load(Ordering::Acquire) >= generation
    }
}
