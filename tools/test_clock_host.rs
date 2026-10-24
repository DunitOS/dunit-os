use std::sync::atomic::{AtomicI64,Ordering};
static EPOCH: AtomicI64=AtomicI64::new(-1);
mod hal {
    pub unsafe fn hal_boot_epoch_seconds()->i64 {super::EPOCH.load(super::Ordering::Relaxed)}
    pub unsafe fn hal_outb(_:u16,_:u8) {}
}
#[path="../kernel/src/clock.rs"] mod clock;
#[test]
fn utc_validity_and_monotonic_progress_are_distinct() {
    clock::init_pit();assert!(clock::realtime().is_none());
    EPOCH.store(1720000000,Ordering::Relaxed);clock::init_pit();
    assert_eq!(clock::realtime(),Some((1720000000,0)));
    let deadline=clock::Deadline::after_ms(10);assert!(!deadline.expired());
    for _ in 0..101 {clock::on_tick();}
    assert!(deadline.expired());assert_eq!(deadline.tick(),1);
    assert_eq!(clock::monotonic_ns(),1010000000);
    assert_eq!(clock::realtime(),Some((1720000001,10000000)));
}
