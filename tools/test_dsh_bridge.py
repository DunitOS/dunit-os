#!/usr/bin/env python3
"""Exercise the actual dsh bridge with bounded, nonreading native PTY shims."""
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
    source = (ROOT / 'tools/test_sdk_stdin_host.rs.in').read_text()
    for marker, path in {
        'SDK_IO_PATH': 'runtime/dunit/src/io.rs',
        'SDK_ERROR_PATH': 'runtime/dunit/src/error.rs',
        'ERRNO_PATH': 'userspace/libdunit/src/errno_numbers.rs',
    }.items():
        source = source.replace(marker, json.dumps(str(ROOT / path)))
    # Extract complete production items verbatim, without copying their logic
    # into a test implementation. HAL/process scheduling alone is scripted.
    app = (ROOT / 'userspace/system_apps/dsh/src/main.rs').read_text()
    bridge = app[app.index('fn bridge('):app.index('/// ANSI escape-parser')]
    sdk = (ROOT / 'runtime/dunit/src/pty.rs').read_text()
    queue = sdk[sdk.index('pub struct PendingInput'):sdk.index('#[cfg(test)]')]
    source += '\n' + (ROOT / 'tools/test_dsh_bridge_host.rs.in').read_text()
    source = source.replace('PRODUCTION_QUEUE', queue).replace('PRODUCTION_BRIDGE', bridge)
    env = dict(os.environ)
    env.setdefault('RUSTUP_TOOLCHAIN', 'stable')
    with tempfile.TemporaryDirectory(prefix='dunit-dsh-bridge-') as folder:
        folder = Path(folder)
        fixture = folder / 'bridge.rs'
        fixture.write_text(source)
        binary = folder / 'bridge-tests'
        shell = folder / 'libdunit_shell.rlib'
        subprocess.run([rustc, '--edition=2021', '--crate-name', 'dunit_shell', '--crate-type=rlib', str(ROOT / 'runtime/dunit-shell/src/lib.rs'), '-o', str(shell)], env=env, check=True)
        subprocess.run([rustc, '--edition=2021', '--test', str(fixture), '--extern', 'dunit_shell=' + str(shell), '-o', str(binary)], env=env, check=True)
        subprocess.run([str(binary), '--nocapture'], check=True)
    print('[DSH-BRIDGE] actual bridge handles full PTY, bounded queue rejection, Ctrl-C/CSI, EOF and output drain')


if __name__ == '__main__':
    main()
