#!/usr/bin/env python3
"""Run actual SDK display and terminal code on a host with scripted native display/input boundaries."""
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
    source = (ROOT / 'tools/test_sdk_display_host.rs.in').read_text()
    for marker, path in {
        'SDK_DISPLAY_PATH': 'runtime/dunit/src/display.rs',
        'SDK_TERMINAL_PATH': 'runtime/dunit/src/terminal.rs',
        'SDK_ERROR_PATH': 'runtime/dunit/src/error.rs',
        'ERRNO_PATH': 'userspace/libdunit/src/errno_numbers.rs',
    }.items():
        source = source.replace(marker, json.dumps(str(ROOT / path)))
    native = (ROOT / 'userspace/libdunit/src/lib.rs').read_text()
    start = native.rindex('#[repr(C)]', 0, native.index('pub struct FbInfo'))
    end = native.index('#[inline(always)]', start)
    source = source.replace('NATIVE_RECORDS', native[start:end])
    with tempfile.TemporaryDirectory(prefix='dunit-sdk-display-') as folder:
        folder = Path(folder)
        fixture = folder / 'display.rs'
        fixture.write_text(source)
        binary = folder / 'display-tests'
        subprocess.run([rustc, '--edition=2021', '--test', str(fixture), '-o', str(binary)], env=env, check=True)
        subprocess.run([str(binary), '--nocapture'], check=True)
    print('[SDK-DISPLAY] actual owned display release/busy/error and terminal cursor/scancode regressions passed')


if __name__ == '__main__':
    main()
