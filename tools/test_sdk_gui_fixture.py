#!/usr/bin/env python3
"""Build native SDK GUI fixtures and run them through qemu_test.py."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from dunit_cli import ROOT, TEST_ATTRIBUTES, build, cargo, create, project
from dunit_manifest import installed_manifest


def main(fixture=None):
    parser = argparse.ArgumentParser(description=__doc__)
    if fixture is None:
        parser.add_argument('fixture', choices=('resize', 'component'))
    parser.add_argument('--accel', choices=('auto', 'kvm', 'tcg'), default='tcg')
    parser.add_argument('--reuse-image', action='store_true')
    args = parser.parse_args()
    fixture = fixture or args.fixture
    source_name = 'sdk_window_resize' if fixture == 'resize' else 'sdk_component'
    app_id = 'sdk_resize_fixture' if fixture == 'resize' else 'sdk_component_fixture'
    config_name = f'sdk-{fixture}.toml'

    os.environ['RUSTUP_TOOLCHAIN'] = 'nightly'
    os.environ['PATH'] = str(Path(cargo()).parent) + os.pathsep + os.environ['PATH']
    command = [sys.executable, str(ROOT / 'tools/qemu_test.py'), '--markers-file',
        str(ROOT / f'tools/{source_name}_markers.json'), '--accel', args.accel,
        '--post-boot-delay', '3', '--settle', '1.5', '--timeout', '360',
        '--boot-timeout', '180', '--cmd-timeout', '180']
    (ROOT / 'build').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=f'sdk-{fixture}-', dir=ROOT / 'build') as folder:
        if not args.reuse_image:
            folder = Path(folder)
            app_folder = folder / 'app'
            create(argparse.Namespace(path=app_folder, id=app_id, name=f'SDK {fixture} fixture', kind='gui'))
            (app_folder / 'src/main.rs').write_text(TEST_ATTRIBUTES + (ROOT / f'tools/fixtures/{source_name}.rs').read_text())
            manifest, package, app = project(app_folder)
            artifact = build(manifest, package, app)
            overlay = folder / 'overlay'
            executable = overlay / 'app' / app.binary
            executable.parent.mkdir(parents=True)
            shutil.copyfile(artifact, executable)
            metadata = overlay / 'system/share/dunit/apps' / app.id / 'dunit.toml'
            metadata.parent.mkdir(parents=True)
            metadata.write_text(installed_manifest(app))
            shutil.copyfile(ROOT / f'tools/fixtures/{source_name}.toml', overlay / config_name)
            subprocess.run([cargo(), 'build', '--release', '--features', 'boot-smoke-tests',
                '-Z', 'build-std=core,alloc,compiler_builtins', '-Z', 'build-std-features=compiler-builtins-mem',
                '-Z', 'json-target-spec'], cwd=ROOT / 'kernel', check=True)
            command += ['--build', '--initrd-overlay', str(overlay)]
        status = subprocess.run(command, cwd=ROOT).returncode
        if status:
            print((ROOT / 'build/qemu-serial.log').read_text(errors='replace')[-16000:])
        return status


if __name__ == '__main__':
    raise SystemExit(main())
