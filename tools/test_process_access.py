#!/usr/bin/env python3
"""Compile the actual current-access module to enforce non-escaping borrows."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT


class Access(unittest.TestCase):
    def compile(self, body):
        source = r'''
#![allow(dead_code)]
mod sync {
    pub struct InterruptGuard;
    impl InterruptGuard { pub fn new() -> Self { Self } }
}
mod process {
    pub struct Process { pub pid: u64, pub cwd: String, pub cap: std::sync::Arc<u64> }
    impl Process { fn take_fd_wakeup(&mut self) -> bool { false } }
    fn wake_wait_set_waiters() {}
    fn current_process() -> Option<&'static Process> { None }
    fn current_process_mut() -> Option<&'static mut Process> { None }
    fn current_thread_mut() -> Option<&'static mut Process> { None }
    #[path=@ACCESS@] pub mod current_access;
    pub use current_access::*;
}
fn main() {
@BODY@
}
'''.replace('@ACCESS@', json.dumps(str(ROOT / 'kernel/src/process/current_access.rs'))).replace('@BODY@', body)
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'access.rs'
            path.write_text(source)
            return subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', str(path),
                                   '-o', str(Path(folder) / 'access')], capture_output=True, text=True)

    def test_owned_results_compile(self):
        result = self.compile('''
let _ = process::with_current_process(|p| p.pid);
let _ = process::with_current_process(|p| p.cwd.clone());
let _ = process::with_current_process(|p| p.cap.clone());
let _ = process::with_current_process_mut(|p| { p.pid += 1; p.pid });
let _ = process::with_current_thread_mut(|p| std::mem::take(&mut p.cwd));
''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_process_and_field_borrows_cannot_escape(self):
        for helper, expression in [('with_current_process', 'p'), ('with_current_process', 'p.cwd.as_str()'),
                                   ('with_current_process_mut', '&mut p.pid'),
                                   ('with_current_thread_mut', '&mut p.cwd')]:
            with self.subTest(helper=helper, expression=expression):
                result = self.compile(f'let _ = process::{helper}(|p| {expression});')
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('lifetime', result.stderr)

    def test_static_argument_callback_is_rejected(self):
        result = self.compile('''
fn needs_static(_: &'static process::Process) -> u64 { 0 }
let _ = process::with_current_process(needs_static);
''')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('not general enough', result.stderr)


if __name__ == '__main__':
    unittest.main()
