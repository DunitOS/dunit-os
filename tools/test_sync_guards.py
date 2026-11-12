#!/usr/bin/env python3
"""Check actual kernel guard auto-traits and host-safe lock contention paths."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class Guards(unittest.TestCase):
    def compile(self, body, run=False):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / 'check.rs'
            binary = Path(folder) / 'check'
            source.write_text('#[allow(dead_code)]\n#[path=' + json.dumps(str(ROOT / 'kernel/src/sync.rs')) + '] mod sync;\n' + body)
            result = subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', str(source), '-o', str(binary)], capture_output=True, text=True)
            if run and result.returncode == 0:
                subprocess.run([str(binary)], check=True, timeout=30)
            return result

    def test_cpu_bound_guard_traits(self):
        cases = [(kind, 'Send') for kind in (
            'InterruptGuard', "SpinLockGuard<'static, u64>",
            "IrqSafeSpinLockGuard<'static, u64>")]
        cases += [(kind, 'Sync') for kind in (
            'InterruptGuard', "IrqSafeSpinLockGuard<'static, u64>")]
        for kind, trait in cases:
            with self.subTest(kind=kind, trait=trait):
                r = self.compile(f'fn require<T: {trait}>() {{}} fn main() {{ require::<sync::{kind}>(); }}')
                self.assertNotEqual(r.returncode, 0)
                message = 'cannot be sent' if trait == 'Send' else 'cannot be shared'
                self.assertIn(message, r.stderr)

    def test_shared_guard_requires_sync_value(self):
        r = self.compile("fn require<T: Sync>() {} fn main() { require::<sync::SpinLockGuard<'static, core::cell::Cell<u64>>>(); }")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn('cannot be shared between threads safely', r.stderr)
        r = self.compile("fn require<T: Sync>() {} fn main() { require::<sync::SpinLockGuard<'static, u64>>(); }")
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_actual_lock_contention_and_publication(self):
        r = self.compile('''
static LOCK: sync::SpinLock<(u64, u64)> = sync::SpinLock::new((0, 0));
static ONCE: sync::OnceCell<(u64, u64)> = sync::OnceCell::new();
fn main() {
    let threads: Vec<_> = (0..8).map(|id| std::thread::spawn(move || {
        let _ = ONCE.set((id, !id));
        for _ in 0..10000 {
            let mut guard = LOCK.lock();
            assert_eq!(guard.1, guard.0 * 3);
            assert!(LOCK.try_lock().is_none());
            guard.0 += 1; guard.1 = guard.0 * 3;
            if let Some(pair) = ONCE.get() { assert_eq!(pair.1, !pair.0); }
        }
    })).collect();
    for thread in threads { thread.join().unwrap(); }
    assert_eq!(*LOCK.lock(), (80000, 240000));
    assert!(LOCK.try_lock().is_some());
    assert!(ONCE.set((9, 9)).is_err());
}
''', run=True)
        self.assertEqual(r.returncode, 0, r.stderr)


if __name__ == '__main__':
    unittest.main()
