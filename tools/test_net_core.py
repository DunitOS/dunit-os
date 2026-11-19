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
        library = Path(folder) / 'libdunit_net_core.rlib'
        subprocess.run([
            compiler, '--edition=2021', '--crate-type=rlib', '--crate-name=dunit_net_core',
            '-Dwarnings', source, '-o', str(library),
        ], check=True)
        subprocess.run([
            compiler, '--edition=2021', '--test', '-O',
            '-Dwarnings', '-Coverflow-checks=yes', '-Cdebug-assertions=yes',
            str(ROOT / 'userspace/net-core/src/lib.rs'), '-o', str(binary),
        ], check=True)
        subprocess.run([str(binary), '--test-threads=1'], check=True, timeout=60)
        interface_binary = Path(folder) / 'netd-interface-tests'
        subprocess.run([
            compiler, '--edition=2021', '--test', '-Dwarnings', '-Coverflow-checks=yes',
            '--extern', f'dunit_net_core={library}',
            str(ROOT / 'userspace/system_apps/netd/src/interface.rs'), '-o', str(interface_binary),
        ], check=True)
        subprocess.run([str(interface_binary)], check=True, timeout=15)
