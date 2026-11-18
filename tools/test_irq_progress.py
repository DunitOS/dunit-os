#!/usr/bin/env python3
"""Compile actual mailbox/boundary modules with only task/HAL host shims."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class IrqProgress(unittest.TestCase):
    def test_actual_mailbox_and_boundaries(self):
        source = (ROOT / 'tools/test_irq_progress_host.rs.in').read_text()
        for token, path in [('MAILBOX', 'kernel/src/reschedule_state.rs'),
                            ('BOUNDARY', 'kernel/src/process/syscall_boundary.rs')]:
            source = source.replace('@' + token + '@', json.dumps(str(ROOT / path)))
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'irq.rs'
            path.write_text(source)
            subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', '-Cdebuginfo=0',
                            str(path), '-o', str(path.with_suffix(''))], check=True)
            subprocess.run([str(path.with_suffix(''))], check=True, timeout=60)

if __name__ == '__main__':
    unittest.main()
