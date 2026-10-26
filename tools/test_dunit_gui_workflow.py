#!/usr/bin/env python3
"""Native GUI SDK acceptance; every guest uses cargo dunit and qemu_test.py."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

from dunit_cli import ROOT, cargo


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel', choices=('auto', 'kvm', 'tcg'), default='tcg')
    args = parser.parse_args()
    env = os.environ.copy()
    env['PATH'] = str(ROOT / 'tools') + os.pathsep + str(Path(cargo()).parent) + os.pathsep + env['PATH']
    env['RUSTUP_TOOLCHAIN'] = 'nightly'
    logs = ROOT / 'build/sdk-gui-workflow'
    logs.mkdir(parents=True, exist_ok=True)

    def invoke(label, arguments, expected=0, evidence=()):
        log = logs / (label + '.log')
        print(f'[sdk-gui] {label} → {log}', flush=True)
        with log.open('w') as output:
            result = subprocess.run([cargo(), 'dunit', *map(str, arguments)], cwd=ROOT,
                env=env, stdout=output, stderr=subprocess.STDOUT, timeout=1200)
        content = log.read_text()
        if result.returncode != expected or any(marker not in content for marker in evidence):
            print(content[-10000:])
            raise RuntimeError(f'{label}: unexpected status/evidence ({result.returncode})')
        if 'dwm_settings: warning' in content:
            raise RuntimeError(f'{label}: generated GUI configuration rejected')
        print(f'[sdk-gui] {label}: OK', flush=True)

    with tempfile.TemporaryDirectory(prefix='dunit-sdk-gui-') as directory:
        project = Path(directory) / 'demo'
        invoke('new', ['new', project, '--id', 'sdk_gui_workflow', '--name', 'Окно Dunit', '--kind', 'gui'])
        options = ['--manifest-path', project / 'Cargo.toml']
        guest = [*options, '--accel', args.accel, '--timeout', '240']
        invoke('build', ['build', *options])
        invoke('run', ['run', *guest], evidence=('[GUI-PRESENT] app=sdk_gui_workflow',
            '[GUI-EXIT] app=sdk_gui_workflow kind=0 code=0'))
        invoke('frames', ['test', *guest], evidence=('[SDKTEST] PASS sdk_gui_workflow::native_window_frames',
            '[SDKTEST] OK count=1', '[GUI-EXIT] app=sdk_gui_workflow kind=0 code=0'))
        source = project / 'src/main.rs'
        original = source.read_text()
        prefix = original.split('#[dunit::test]')[0]
        for label, suffix, evidence in (
            ('empty', '', ('[SDKTEST] FAIL no tests discovered', '[GUI-EXIT] app=sdk_gui_workflow kind=0 code=1')),
            ('error', '#[dunit::test]\nfn fails() -> dunit::Result<()> { Err(dunit::Error(dunit::errno::EINVAL)) }\n',
             ('[SDKTEST] FAIL', '[GUI-EXIT] app=sdk_gui_workflow kind=0 code=1')),
            ('panic', '#[dunit::test]\nfn panics() { panic!("GUI workflow failure"); }\n',
             ('[SDK] PANIC', '[GUI-EXIT] app=sdk_gui_workflow kind=0 code=101')),
        ):
            source.write_text(prefix + suffix)
            invoke(label, ['test', *guest], expected=1, evidence=evidence)
    print('[sdk-gui] all native GUI cases passed', flush=True)


if __name__ == '__main__':
    main()
