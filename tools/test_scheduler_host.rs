extern crate alloc;
use std::sync::atomic::{AtomicU64,Ordering};
#[derive(Clone,Copy,PartialEq,Eq,Debug)] pub struct ProcessId(pub u64);
#[derive(Debug)] pub enum ProcessError {NotRunnable,SchedulerUnavailable}
static READY:AtomicU64=AtomicU64::new(0);
pub fn is_pid_runnable(pid:ProcessId)->bool {READY.load(Ordering::Relaxed)&(1<<pid.0)!=0}
mod memory {pub fn serial_write(_: &str) {}}
mod sync {
    pub struct IrqSafeSpinLock<T>(std::sync::Mutex<T>);
    impl<T> IrqSafeSpinLock<T> {
        pub const fn new(value:T)->Self {Self(std::sync::Mutex::new(value))}
        pub fn lock(&self)->std::sync::MutexGuard<'_,T>{self.0.lock().unwrap()}
    }
}
#[path="../kernel/src/process/scheduler.rs"] mod scheduler;
#[test]
fn excluding_a_ready_thread_does_not_lose_it_when_its_parent_blocks() {
    scheduler::init();
    READY.store(1<<2,Ordering::Relaxed);
    scheduler::enqueue_ready(ProcessId(2)).unwrap();
    assert_eq!(scheduler::pick_next_candidate_excluding(ProcessId(2)),None);
    assert_eq!(scheduler::ready_len(),1);
    assert!(scheduler::has_ready_tasks());
    assert_eq!(scheduler::pick_next_candidate(),Some(ProcessId(2)));
    READY.store((1<<1)|(1<<2),Ordering::Relaxed);
    scheduler::enqueue_ready(ProcessId(1)).unwrap();
    assert_eq!(scheduler::pick_next_candidate_excluding(ProcessId(2)),Some(ProcessId(1)));
    scheduler::remove(ProcessId(1));
    READY.store(0,Ordering::Relaxed);
    assert!(!scheduler::has_ready_tasks());
    assert_eq!(scheduler::pick_next_candidate(),None);
    assert_eq!(scheduler::ready_len(),0);
}
