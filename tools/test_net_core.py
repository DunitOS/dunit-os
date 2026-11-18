#!/usr/bin/env python3
"""Compile/test the actual no_std userspace packet crate without a Cargo cache."""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='dunit-net-core-') as folder:
        binary = Path(folder) / 'net-core-tests'
        compiler = os.environ.get('RUSTC', 'rustc')
        source = str(ROOT / 'userspace/net-core/src/lib.rs')
        subprocess.run([
            compiler, '--edition=2021', '--crate-type=rlib', '--emit=metadata',
            '-Dwarnings', source, '-o', str(Path(folder) / 'net-core.rmeta'),
        ], check=True)
        subprocess.run([
            compiler, '--edition=2021', '--test', '-O',
            '-Dwarnings', '-Coverflow-checks=yes', '-Cdebug-assertions=yes',
            str(ROOT / 'userspace/net-core/src/lib.rs'), '-o', str(binary),
        ], check=True)
        subprocess.run([str(binary), '--test-threads=1'], check=True, timeout=60)
