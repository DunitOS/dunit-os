#!/usr/bin/env python3
"""End-to-end SDK acceptance, including failure propagation in real guests."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

from dunit_cli import ROOT, cargo
from check_image_profile import read_archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel', choices=('auto', 'kvm', 'tcg'), default='tcg')
    args = parser.parse_args()
    environment = os.environ.copy()
    environment['PATH'] = str(ROOT / 'tools') + os.pathsep + str(Path(cargo()).parent) + os.pathsep + environment['PATH']
    environment['RUSTUP_TOOLCHAIN'] = 'nightly'
    logs = ROOT / 'build/sdk-workflow'
    logs.mkdir(parents=True, exist_ok=True)

    def invoke(label, arguments, expected=0, evidence=()):
        log = logs / (label + '.log')
        print(f'[sdk-workflow] {label} → {log}', flush=True)
        with log.open('w') as output:
            result = subprocess.run([cargo(), 'dunit', *map(str, arguments)], cwd=ROOT,
                                    env=environment, stdout=output, stderr=subprocess.STDOUT, timeout=1200)
        content = log.read_text()
        if result.returncode != expected or any(marker not in content for marker in evidence):
            print(content[-12000:])
            raise RuntimeError(f'{label}: unexpected status/evidence (code {result.returncode})')
        if label == "test":
            with tempfile.TemporaryDirectory(prefix="dunit-payload-") as directory:
                installed = Path(directory) / "initrd.img"
                subprocess.run(["mcopy", "-i", str(ROOT / "build/installer-esp.img"),
                                "::/boot/initrd.img", str(installed)], check=True)
                live = (ROOT / "build/initrd.img").read_bytes()
                if installed.read_bytes() != live:
                    raise RuntimeError("SDK Live and installer payloads differ")
                entries = read_archive(live)
                if "/system/share/dunit/apps/sdk_workflow/dunit.toml" not in entries:
                    raise RuntimeError("SDK manifest missing from packaged payload")
        print(f'[sdk-workflow] {label}: OK', flush=True)

    with tempfile.TemporaryDirectory(prefix='dunit-sdk-') as directory:
        project = Path(directory) / 'hello'
        invoke('new', ['new', project, '--id', 'sdk_workflow'])
        manifest = project / 'Cargo.toml'
        options = ['--manifest-path', manifest]
        guest = [*options, '--accel', args.accel]
        invoke('build', ['build', *options])
        invoke('run', ['run', *guest], evidence=('[SDKAPP] ok', '/app/sdk_workflow returned code=0'))
        invoke('test', ['test', *guest], evidence=('[SDKTEST] OK count=1', '/app/sdk_workflow returned code=0'))
        source = project / 'src/main.rs'
        original = source.read_text()
        prefix = original.split('#[dunit::test]')[0]
        for label, suffix, markers in (
            ('zero-tests', '', ('[SDKTEST] FAIL no tests discovered', 'returned code=1')),
            ('failed-test', '#[dunit::test]\nfn fails() -> dunit::Result<()> { Err(dunit::Error(dunit::errno::EINVAL)) }\n',
             ('[SDKTEST] FAIL', 'returned code=1')),
            ('panic-test', '#[dunit::test]\nfn panics() { panic!("intentional workflow failure"); }\n',
             ('[SDK] PANIC', 'returned code=101')),
        ):
            source.write_text(prefix + suffix)
            invoke(label, ['test', *guest], expected=1, evidence=markers)
        calc = ROOT / 'userspace/system_apps/calc/Cargo.toml'
        invoke('calc-test', ['test', '--manifest-path', calc, '--accel', args.accel],
               evidence=('[SDKTEST] OK count=2', '/app/calc returned code=0'))
        invoke('calc-run', ['run', '--manifest-path', calc, '--accel', args.accel,
                            '--input-ready', 'calc> ', '--input', '7*6', '--input', 'exit'],
               evidence=('calc: bye', '/app/calc returned code=0'))
    print('[sdk-workflow] all native SDK acceptance cases passed', flush=True)


if __name__ == '__main__':
    main()
