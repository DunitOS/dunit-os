#!/usr/bin/env python3
"""Compile/test the no_std userspace cert-store crate on host."""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='dunit-cert-store-') as folder:
        work = Path(folder)
        compiler = os.environ.get('RUSTC', 'rustc')
        tls_lib = work / 'libdunit_tls_core.rlib'
        binary = work / 'cert-store-tests'
        subprocess.run([
            compiler, '--edition=2021', '--crate-type=rlib', '--crate-name=dunit_tls_core',
            '-Dwarnings', str(ROOT / 'userspace/tls-core/src/lib.rs'), '-o', str(tls_lib),
        ], check=True)
        subprocess.run([
            compiler, '--edition=2021', '--test', '-O',
            '-Dwarnings', '-Coverflow-checks=yes', '-Cdebug-assertions=yes',
            '--extern', f'dunit_tls_core={tls_lib}',
            str(ROOT / 'userspace/cert-store/src/lib.rs'), '-o', str(binary),
        ], check=True)
        subprocess.run([str(binary), '--test-threads=1'], check=True, timeout=60)

