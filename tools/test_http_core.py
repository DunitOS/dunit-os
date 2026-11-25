#!/usr/bin/env python3
"""Compile/test the no_std userspace HTTP/1.1 crate on host."""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='dunit-http-core-') as folder:
        binary = Path(folder) / 'http-core-tests'
        compiler = os.environ.get('RUSTC', 'rustc')
        source = str(ROOT / 'userspace/http-core/src/lib.rs')
        subprocess.run([
            compiler, '--edition=2021', '--test', '-O',
            '-Dwarnings', '-Coverflow-checks=yes', '-Cdebug-assertions=yes',
            source, '-o', str(binary),
        ], check=True)
        subprocess.run([str(binary), '--test-threads=1'], check=True, timeout=60)

