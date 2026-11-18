#!/usr/bin/env python3
"""Run setup's actual menu/parser against SDK streams with native I/O scripted.

Reuses a host-built dwm_settings dependency cache to avoid a second Cargo target.
Build that crate for the host first if the cache is absent; DWM_HOST_TARGET_DIR
can select an existing target directory.
"""
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
        raise SystemExit('rustc is required')
    env = dict(os.environ)
    env.setdefault('RUSTUP_TOOLCHAIN', 'stable')
    host = subprocess.check_output([rustc, '-vV'], env=env, text=True).split('host: ')[1].splitlines()[0]
    cache = Path(os.environ.get('DWM_HOST_TARGET_DIR', ROOT / 'userspace/dwm_settings/target' / host))
    matches = [p for p in cache.rglob('libdwm_settings*.rlib') if p.name != 'libdwm_settings.rlib']
    if len(matches) != 1:
        raise SystemExit('select a host dwm_settings cache with DWM_HOST_TARGET_DIR (one libdwm_settings rlib required)')
    links = sorted({str(p.parent) for p in cache.rglob('*.rlib')} | {str(p.parent) for p in cache.rglob('*.rmeta')} | {str(cache.parent / 'debug/deps')})
    source = (ROOT / 'tools/test_sdk_stdin_host.rs.in').read_text()
    source += (ROOT / 'tools/test_setup_sdk_host.rs.in').read_text()
    for marker, path in {
        'SDK_IO_PATH': 'runtime/dunit/src/io.rs',
        'SDK_ERROR_PATH': 'runtime/dunit/src/error.rs',
        'ERRNO_PATH': 'userspace/libdunit/src/errno_numbers.rs',
        'SETUP_APP_PATH': 'userspace/system_apps/setup/src/app.rs',
    }.items():
        source = source.replace(marker, json.dumps(str(ROOT / path)))
    with tempfile.TemporaryDirectory(prefix='dunit-setup-sdk-') as folder:
        folder = Path(folder)
        fixture = folder / 'setup.rs'
        fixture.write_text(source)
        binary = folder / 'setup-tests'
        command = [rustc, '--edition=2021', '--test', str(fixture), '--extern', f'dwm_settings={matches[0]}']
        for link in links:
            command.extend(['-L', f'dependency={link}'])
        command.extend(['-o', str(binary)])
        subprocess.run(command, env=env, check=True)
        subprocess.run([str(binary), '--nocapture'], check=True)
    print('[SETUP-SDK] actual menu, shared parser, buffered input and partial output regressions passed')


if __name__ == '__main__':
    main()
