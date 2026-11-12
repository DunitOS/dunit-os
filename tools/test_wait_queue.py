#!/usr/bin/env python3
"""Exercise actual wait registration storage with reentrant wake callbacks."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT


class WaitQueueTest(unittest.TestCase):
    def test_fifo_limits_cancellation_and_reentry(self):
        source = r'''
extern crate alloc;
#[allow(dead_code)] #[path=@SYNC@] mod actual_sync;
mod sync { pub use crate::actual_sync::SpinLock as IrqSafeSpinLock; }
mod process {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct ProcessId(pub u64);
    #[path=@WAIT@] mod wait_queue;
    use wait_queue::*;
    use crate::sync::IrqSafeSpinLock;
    fn entry(n: u64, key: WaitKey, deadline: Option<u64>) -> WaitEntry {
        WaitEntry { entity: ProcessId(n), key, deadline }
    }
    pub fn run() {
        let q = IrqSafeSpinLock::new(WaitQueue::new());
        for n in 1..=4 { q.lock().push(entry(n, WaitKey::Pipe(7), None)); }
        q.lock().push(entry(5, WaitKey::Ipc(ProcessId(9)), Some(10)));
        let pipe = |e: &WaitEntry| e.key == WaitKey::Pipe(7);
        let mut seen = Vec::new();
        assert_eq!(wake_matching(&q, pipe, 2, |e| {
            // Reacquiring the real lock detects callbacks under the queue lock.
            let mut lock = q.try_lock().expect("wake callback holds wait queue lock");
            seen.push(e.entity.0);
            lock.remove_entity(ProcessId(3));
            lock.push(entry(e.entity.0, e.key, None)); // same-PID ABA
        }), 2);
        assert_eq!(seen, [1,2]);
        seen.clear();
        assert_eq!(wake_matching(&q, pipe, 0, |e| {
            seen.push(e.entity.0);
            q.lock().push(entry(e.entity.0, e.key, None));
        }), 3);
        assert_eq!(seen, [4,1,2]); // freshly registered entries excluded
        seen.clear();
        assert_eq!(wake_matching(&q, |e| e.deadline.map(|d| d <= 9).unwrap_or(false), 0,
            |_| panic!("deadline fired early")), 0);
        assert_eq!(wake_matching(&q, |e| e.deadline == Some(10), 0,
            |e| seen.push(e.entity.0)), 1);
        assert_eq!(seen, [5]);
        seen.clear();
        // Nested producer can consume remaining old waiters; outer pass must
        // neither duplicate them nor consume registrations created by callback.
        assert_eq!(wake_matching(&q, pipe, 0, |e| {
            seen.push(e.entity.0);
            assert_eq!(wake_matching(&q, pipe, 0, |_| {}), 2);
            q.lock().push(entry(99, WaitKey::Poll, None));
        }), 1);
        assert_eq!(seen, [4]);
        assert_eq!(wake_matching(&q, |e| e.key == WaitKey::Poll, 0,
            |e| assert_eq!(e.entity, ProcessId(99))), 1);
        assert_eq!(wake_matching(&q, |_| true, 0, |_| panic!("queue not empty")), 0);
        let q = IrqSafeSpinLock::new(WaitQueue::new());
        for n in 1..=2 { q.lock().push(entry(n, WaitKey::Timer, None)); }
        assert_eq!(wake_matching(&q, |_| true, 0, |e| {
            assert_eq!(e.entity, ProcessId(1));
            let mut lock = q.lock();
            lock.remove_entity(ProcessId(2));
            lock.push(entry(2, WaitKey::Timer, None));
        }), 1); // canceled old registration must not make replacement eligible
        assert_eq!(wake_matching(&q, |_| true, 0,
            |e| assert_eq!(e.entity, ProcessId(2))), 1);
        let word = WaitKey::Word { owner: ProcessId(10), addr: 0x1000 };
        q.lock().push(entry(1, word, None));
        q.lock().push(entry(2, WaitKey::Word { owner: ProcessId(11), addr: 0x1000 }, None));
        assert_eq!(wake_matching(&q, |e| e.key == word, 1,
            |e| assert_eq!(e.entity, ProcessId(1))), 1);
        assert_eq!(wake_matching(&q, |_| true, 0,
            |e| assert_eq!(e.entity, ProcessId(2))), 1);
        // Concurrent registration/selection uses real kernel SpinLock storage.
        let q = std::sync::Arc::new(IrqSafeSpinLock::new(WaitQueue::new()));
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let threads: Vec<_> = (0..8).map(|n| {
            let q = q.clone(); let count = count.clone();
            std::thread::spawn(move || {
                for _ in 0..1000 {
                    q.lock().push(entry(n, WaitKey::WaitSet, None));
                    wake_matching(&q, |_| true, 1, |_| {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    });
                }
            })
        }).collect();
        for t in threads { t.join().unwrap(); }
        wake_matching(&q, |_| true, 0, |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 8000);
        println!("wait queue: FIFO/limits/deadlines/cancel/reentry/ABA/8000 concurrent registrations OK");
    }
}
fn main() { process::run(); }
'''.replace('@SYNC@', json.dumps(str(ROOT / 'kernel/src/sync.rs'))).replace('@WAIT@', json.dumps(str(ROOT / 'kernel/src/process/wait_queue.rs')))
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'wait.rs'
            binary = Path(folder) / 'wait'
            path.write_text(source)
            result = subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', '-O', str(path), '-o', str(binary)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            subprocess.run([str(binary)], check=True, timeout=30)


if __name__ == '__main__':
    unittest.main()
