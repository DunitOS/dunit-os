#!/usr/bin/env python3
"""Exercise the real scheduler with deterministic reentrant state checks."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT


class Queue(unittest.TestCase):
    def test_queue_lock_order_generation_and_contention(self):
        source = r'''
extern crate alloc;
#[allow(dead_code)] #[path=@SYNC@] mod actual_sync;
mod sync {
    // Host-only IRQ shim: real SpinLock storage/ordering, no privileged CLI/STI.
    pub use crate::actual_sync::SpinLock as IrqSafeSpinLock;
    pub struct InterruptGuard;
    impl InterruptGuard { pub fn new() -> Self { Self } }
}
mod memory { pub fn serial_write(_: &str) {} }
mod process {
    use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};
    #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct ProcessId(pub u64);
    #[derive(Debug, PartialEq, Eq)] pub enum ProcessError { NotRunnable, SchedulerUnavailable }
    static READY: AtomicU64 = AtomicU64::new(0);
    static HOOK: AtomicU8 = AtomicU8::new(0);
    #[path=@QUEUE@] pub mod scheduler;
    pub fn is_pid_runnable(pid: ProcessId) -> bool {
        let mode = HOOK.swap(0, Ordering::SeqCst);
        if mode != 0 {
            // Models remove+reenqueue between the observation and queue commit.
            // This would deadlock if validation held the scheduler lock.
            scheduler::remove(pid);
            scheduler::enqueue_ready(pid).unwrap();
            return mode == 2;
        }
        READY.load(Ordering::SeqCst) & (1 << pid.0) != 0
    }
    pub fn run() {
        use scheduler::*;
        READY.store(2, Ordering::SeqCst);
        assert_eq!(enqueue_ready(ProcessId(1)), Err(ProcessError::SchedulerUnavailable));
        init();
        READY.store(14, Ordering::SeqCst);
        for n in 1..=3 { enqueue_ready(ProcessId(n)).unwrap(); }
        enqueue_ready(ProcessId(2)).unwrap(); assert_eq!(ready_len(), 3);
        for n in [1,2,3,1] { assert_eq!(pick_next_candidate(), Some(ProcessId(n))); }
        assert_eq!(pick_next_candidate_excluding(ProcessId(2)), Some(ProcessId(3)));
        assert_eq!(ready_len(), 3);
        init(); READY.store(2, Ordering::SeqCst); enqueue_ready(ProcessId(1)).unwrap();
        assert_eq!(pick_next_candidate_excluding(ProcessId(1)), None);
        assert_eq!(pick_next_candidate(), Some(ProcessId(1)));
        assert_eq!(enqueue_ready(ProcessId(2)), Err(ProcessError::NotRunnable));
        for mode in [1,2] {
            HOOK.store(mode, Ordering::SeqCst);
            assert_eq!(pick_next_candidate(), None);
            assert_eq!(ready_len(), 1);
            assert_eq!(pick_next_candidate(), Some(ProcessId(1)));
        }
        HOOK.store(2, Ordering::SeqCst); assert!(!has_ready_tasks());
        assert!(has_ready_tasks());
        init(); READY.store(6, Ordering::SeqCst);
        enqueue_ready(ProcessId(1)).unwrap(); enqueue_ready(ProcessId(2)).unwrap();
        READY.store(4, Ordering::SeqCst);
        assert_eq!(pick_next_candidate(), Some(ProcessId(2))); // no skipped successor
        assert_eq!(ready_len(), 1);
        READY.store(0, Ordering::SeqCst); assert!(!has_ready_tasks());
        assert_eq!(pick_next_candidate(), None); assert_eq!(ready_len(), 0);
        init(); READY.store(510, Ordering::SeqCst);
        let threads: Vec<_> = (1..=8).map(|n| std::thread::spawn(move || {
            for _ in 0..1000 { remove(ProcessId(n)); enqueue_ready(ProcessId(n)).unwrap(); }
        })).collect();
        for thread in threads { thread.join().unwrap(); }
        assert_eq!(ready_len(), 8);
        let mut seen = 0;
        for _ in 0..8 { seen |= 1 << pick_next_candidate().unwrap().0; }
        assert_eq!(seen, 510);
        println!("scheduler: round-robin/exclusion/stale removal/reentrant ABA/8000 concurrent requeues OK");
    }
}
fn main() { process::run(); }
'''.replace('@SYNC@', json.dumps(str(ROOT / 'kernel/src/sync.rs'))).replace('@QUEUE@', json.dumps(str(ROOT / 'kernel/src/process/scheduler.rs')))
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'queue.rs'
            binary = Path(folder) / 'queue'
            path.write_text(source)
            subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', '-O', str(path), '-o', str(binary)], check=True, capture_output=True)
            subprocess.run([str(binary)], check=True, timeout=30)


if __name__ == '__main__':
    unittest.main()
