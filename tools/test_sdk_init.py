#!/usr/bin/env python3
"""Run init's actual supervision policy against scripted child/log boundaries."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    rustc = os.environ.get('RUSTC') or shutil.which('rustc')
    if not rustc:
        raise SystemExit('rustc is required (set RUSTC or add it to PATH)')
    env = dict(os.environ)
    env.setdefault('RUSTUP_TOOLCHAIN', 'stable')
    init = (ROOT / 'userspace/system_apps/init/src/main.rs').read_text()
    process = (ROOT / 'runtime/dunit/src/process.rs').read_text()
    source = (ROOT / 'tools/test_sdk_init_host.rs.in').read_text()
    # Copy whole actual items; filesystem, entry and panic are native-test scope.
    sections = {
        'INIT_TYPES': init[init.index('#[derive(Clone, Copy, PartialEq, Eq)]'):init.index('// ---- small helpers')],
        'INIT_PARSER': init[init.index('fn push_u32'):init.index('/// Discover and register')],
        'INIT_POLICY': init[init.index('fn start_service'):init.index('// PID 1 retains')],
        'EXIT_STATUS': process[process.index('#[derive(Debug, Clone, Copy, PartialEq, Eq)]'):process.index('#[derive(Debug)]\npub struct Child')],
    }
    native = (ROOT / 'userspace/libdunit/src/lib.rs').read_text()
    sections['WAIT_KINDS'] = '\n'.join(line for line in native.splitlines() if line.startswith('pub const WAIT_KIND_'))
    for marker, text in sections.items():
        source = source.replace(marker, text)
    with tempfile.TemporaryDirectory(prefix='dunit-sdk-init-') as folder:
        fixture = Path(folder) / 'init.rs'
        fixture.write_text(source)
        binary = Path(folder) / 'init-tests'
        subprocess.run([rustc, '--edition=2021', '--test', str(fixture), '-o', str(binary)], env=env, check=True)
        subprocess.run([str(binary), '--nocapture'], check=True)
    print('[SDK-INIT] actual supervision restart, fallback ownership/reap and policy regressions passed')


if __name__ == '__main__':
    main()
