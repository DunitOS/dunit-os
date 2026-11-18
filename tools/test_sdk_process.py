#!/usr/bin/env python3
"""Run actual SDK process code on a host with scripted native process boundaries."""
import json
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
    # This host-only fixture uses stable Rust and must not trigger the OS's
    # rust-toolchain.toml nightly update/download during a regression run.
    env.setdefault('RUSTUP_TOOLCHAIN', 'stable')
    source = (ROOT / 'tools/test_sdk_process_host.rs.in').read_text()
    for marker, path in {
        'SDK_PROCESS_PATH': 'runtime/dunit/src/process.rs',
        'SDK_TIME_PATH': 'runtime/dunit/src/time.rs',
        'SDK_ERROR_PATH': 'runtime/dunit/src/error.rs',
        'ERRNO_PATH': 'userspace/libdunit/src/errno_numbers.rs',
    }.items():
        source = source.replace(marker, json.dumps(str(ROOT / path)))
    # Keep native status layout/constants/methods authoritative in libdunit.
    native = (ROOT / 'userspace/libdunit/src/lib.rs').read_text()
    start = native.rindex('#[repr(C)]', 0, native.index('pub struct WaitStatus'))
    end = native.index('pub fn print_usize', start)
    source = source.replace('WAIT_STATUS_SOURCE', native[start:end])
    with tempfile.TemporaryDirectory(prefix='dunit-sdk-process-') as folder:
        folder = Path(folder)
        fixture = folder / 'process.rs'
        fixture.write_text(source)
        binary = folder / 'process-tests'
        subprocess.run([rustc, '--edition=2021', '--test', str(fixture), '-o', str(binary)], env=env, check=True)
        subprocess.run([str(binary), '--nocapture'], check=True)
    print('[SDK-PROCESS] actual SDK prepared/cache, nonblocking wait, completion wake and validation regressions passed')


if __name__ == '__main__':
    main()
