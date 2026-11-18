#!/usr/bin/env python3
"""Run actual poll/wait-set/attached syscall code with IRQ and fault shims."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT


class SyscallWaitScopes(unittest.TestCase):
    def test_transactions_faults_and_remote_probe_gap(self):
        source = (ROOT / 'tools/test_syscall_wait_scopes_host.rs.in').read_text()
        native = (ROOT / 'kernel/src/syscall/native_fd.rs').read_text()
        poll = native[native.index('pub(super) fn poll('):native.index('/// Eager private file snapshot')]
        source = source.replace('@POLL@', poll)
        for key, path in [('WAIT', 'kernel/src/syscall/wait_set.rs'),
                          ('ATTACHED', 'kernel/src/syscall/attached_ipc.rs'),
                          ('ABI', 'abi/wait-set/src/lib.rs')]:
            source = source.replace('@' + key + '@', json.dumps(str(ROOT / path)))
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'scopes.rs'
            path.write_text(source)
            result = subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021',
                                     str(path), '-O', '-o', str(path.with_suffix(''))],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            subprocess.run([str(path.with_suffix(''))], check=True, timeout=30)


if __name__ == '__main__':
    unittest.main()
