#!/usr/bin/env python3
"""Run actual SDK stream code on a host with scripted native I/O boundaries."""
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
    source = (ROOT / 'tools/test_sdk_stdin_host.rs.in').read_text()
    for marker, path in {
        'SDK_IO_PATH': 'runtime/dunit/src/io.rs',
        'SDK_ERROR_PATH': 'runtime/dunit/src/error.rs',
        'ERRNO_PATH': 'userspace/libdunit/src/errno_numbers.rs',
    }.items():
        source = source.replace(marker, json.dumps(str(ROOT / path)))
    with tempfile.TemporaryDirectory(prefix='dunit-sdk-stdin-') as folder:
        folder = Path(folder)
        fixture = folder / 'stdin.rs'
        fixture.write_text(source)
        binary = folder / 'stdin-tests'
        subprocess.run([rustc, '--edition=2021', '--test', str(fixture), '-o', str(binary)], env=env, check=True)
        subprocess.run([str(binary), '--nocapture'], check=True)
    print('[SDK-STDIN] actual SDK nonblocking input, readiness, partial output and error regressions passed')


if __name__ == '__main__':
    main()
