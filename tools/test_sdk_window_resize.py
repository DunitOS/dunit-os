#!/usr/bin/env python3
"""Force a real compositor resize while an SDK drawing callback is running."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from dunit_cli import ROOT, TEST_ATTRIBUTES, build, cargo, create, project
from dunit_manifest import installed_manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel', choices=('auto', 'kvm', 'tcg'), default='tcg')
    parser.add_argument('--reuse-image', action='store_true')
    args = parser.parse_args()
    os.environ['RUSTUP_TOOLCHAIN'] = 'nightly'
    os.environ['PATH'] = str(Path(cargo()).parent) + os.pathsep + os.environ['PATH']
    command = [sys.executable, str(ROOT / 'tools/qemu_test.py'), '--markers-file',
        str(ROOT / 'tools/sdk_window_resize_markers.json'), '--accel', args.accel,
        '--post-boot-delay', '3', '--settle', '1.5', '--timeout', '360',
        '--boot-timeout', '180', '--cmd-timeout', '180']
    (ROOT / 'build').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='sdk-resize-', dir=ROOT / 'build') as folder:
        if not args.reuse_image:
            folder = Path(folder)
            app_folder = folder / 'app'
            create(argparse.Namespace(path=app_folder, id='sdk_resize_fixture', name='Resize fixture', kind='gui'))
            (app_folder / 'src/main.rs').write_text(TEST_ATTRIBUTES + (ROOT / 'tools/fixtures/sdk_window_resize.rs').read_text())
            manifest, package, app = project(app_folder)
            artifact = build(manifest, package, app)
            overlay = folder / 'overlay'
            executable = overlay / 'app' / app.binary
            executable.parent.mkdir(parents=True)
            shutil.copyfile(artifact, executable)
            metadata = overlay / 'system/share/dunit/apps' / app.id / 'dunit.toml'
            metadata.parent.mkdir(parents=True)
            metadata.write_text(installed_manifest(app))
            (overlay / 'sdk-resize.toml').write_text('[meta]\nschema_version = 2\n'
                '[startup]\nentries = ["sdk_resize_fixture"]\nself_test = false\n'
                '[shortcuts]\ncmd_mod = "super"\nwin_max = true\nwin_close = true\n')
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
