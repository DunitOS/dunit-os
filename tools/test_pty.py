#!/usr/bin/env python3
"""Actual locked PTY streams: concurrent bytes, reentry and endpoint handover."""
import os
from pathlib import Path
import subprocess
import tempfile
from dunit_cli import ROOT

if __name__ == '__main__':
    with tempfile.TemporaryDirectory() as folder:
        binary = Path(folder) / 'pty-test'
        subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', '--test', '-O',
                        str(ROOT / 'tools/test_pty_host.rs'), '-o', str(binary)], check=True)
        subprocess.run([str(binary)], check=True, timeout=60)
