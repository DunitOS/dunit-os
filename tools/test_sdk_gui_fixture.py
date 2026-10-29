#!/usr/bin/env python3
"""Build native SDK GUI fixtures and run them through qemu_test.py."""
import argparse
import json
import os
import re
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
        parser.add_argument('fixture', choices=('resize', 'component', 'theme', 'effects', 'backdrop', 'motion', 'buttons', 'checks', 'input', 'multiline', 'scroll', 'scroll_xy'))
    parser.add_argument('--accel', choices=('auto', 'kvm', 'tcg'), default='tcg')
    parser.add_argument('--reuse-image', action='store_true')
    args = parser.parse_args()
    fixture = fixture or args.fixture
    source_name = "sdk_window_resize" if fixture == "resize" else f"sdk_{fixture}"
    app_id = f'sdk_{fixture}_fixture'
    config_name = f'sdk-{fixture.replace("_", "-")}.toml'

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
            if fixture in ('component', 'resize'):
                with (app_folder / 'Cargo.toml').open('a') as manifest_file:
                    manifest_file.write('dwm_settings = { path = '
                        + json.dumps(str(ROOT / 'userspace/dwm_settings')) + ' }\n')
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
            if fixture in ('theme', 'effects', 'backdrop', 'motion'):
                profile = (ROOT / 'assets/dwm/sdk/dark.dss').read_text()
                if fixture == 'motion':
                    profile += (ROOT / 'tools/fixtures/sdk_motion_profile.dss').read_text()
                (overlay / 'sdk-theme.dss').write_text(profile)
            subprocess.run([cargo(), 'build', '--release', '--features', 'boot-smoke-tests',
                '-Z', 'build-std=core,alloc,compiler_builtins', '-Z', 'build-std-features=compiler-builtins-mem',
                '-Z', 'json-target-spec'], cwd=ROOT / 'kernel', check=True)
            command += ['--build', '--initrd-overlay', str(overlay)]
        status = subprocess.run(command, cwd=ROOT).returncode
        if not status and fixture == 'component':
            serial = (ROOT / 'build/qemu-serial.full.log').read_text(errors='replace')
            frames = re.findall(
                r'\[DAMAGE\] partial composed=(\d+) presented=(\d+) texture=(\d+) screen=(\d+)',
                serial)
            dirty_pixels = 480 * 32
            if len(frames) != 2 or any(
                tuple(map(int, frame[:3])) != (dirty_pixels,) * 3
                or int(frame[3]) <= dirty_pixels for frame in frames
            ):
                print(f'Unexpected component presentation counts: {frames}')
                status=1
        if status:
            print((ROOT / 'build/qemu-serial.log').read_text(errors='replace')[-16000:])
        if not status and fixture in ('theme', 'effects', 'backdrop', 'motion'):
            from sdk_theme_pixels import verify
            verify(ROOT, fixture)
        return status


if __name__ == '__main__':
    raise SystemExit(main())
