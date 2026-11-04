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
        parser.add_argument('fixture', choices=('resize', 'component', 'theme', 'effects', 'backdrop', 'motion', 'buttons', 'checks', 'input', 'multiline', 'scroll', 'scroll_xy', 'autoscroll', 'slider', 'spinbox', 'vertical_slider', 'progress', 'menus', 'workspace', 'list', 'tree', 'collections', 'lifecycle', 'latency', 'console', 'modifiers', 'transfer', 'matrix', 'clipboard'))
    parser.add_argument('--accel', choices=('auto', 'kvm', 'tcg'), default='tcg')
    parser.add_argument('--reuse-image', action='store_true')
    args = parser.parse_args()
    fixture = fixture or args.fixture
    source_name = "sdk_window_resize" if fixture == "resize" else f"sdk_{fixture}"
    app_id = f'sdk_{fixture}_fixture'
    config_name = f'sdk-{fixture.replace("_", "-")}.toml'
    contract = json.loads((ROOT / f'tools/{source_name}_markers.json').read_text()) if fixture!='matrix' else {}
    image_profile = contract.get('run', {}).get('image_profile', 'dwm')
    if image_profile not in ('dwm', 'full'):
        raise ValueError('SDK fixtures require dwm or full image composition')
    boot_config = ('limine_test_terminal.conf' if image_profile=='full' else 'limine_sdk_terminal.conf')

    os.environ['RUSTUP_TOOLCHAIN'] = 'nightly'
    os.environ['PATH'] = str(Path(cargo()).parent) + os.pathsep + os.environ['PATH']
    command = [sys.executable, str(ROOT / 'tools/qemu_test.py'), '--markers-file',
        str(ROOT / f'tools/{source_name}_markers.json'), '--accel', args.accel,
        '--post-boot-delay', '3', '--settle', '1.5', '--timeout', '360',
        '--boot-timeout', '180', '--cmd-timeout', '180']
    if fixture=='transfer':
        command[command.index('--timeout')+1]='600'
        command[command.index('--cmd-timeout')+1]='480'
    (ROOT / 'build').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=f'sdk-{fixture}-', dir=ROOT / 'build') as folder:
        if not args.reuse_image:
            folder = Path(folder)
            overlay = folder / 'overlay'
            def install_fixture(source, identity, name, kind='gui'):
                app_folder = folder / identity
                create(argparse.Namespace(path=app_folder, id=identity, name=name, kind=kind))
                fixture_cache = ROOT / 'build/sdk-fixture-target'
                fixture_cache.mkdir(exist_ok=True)
                (app_folder / 'target').symlink_to(fixture_cache, target_is_directory=True)
                if fixture in ('component', 'resize', 'transfer'):
                    with (app_folder / 'Cargo.toml').open('a') as manifest_file:
                        manifest_file.write('dwm_settings = { path = '
                            + json.dumps(str(ROOT / 'userspace/dwm_settings')) + ' }\n')
                (app_folder / 'src/main.rs').write_text(TEST_ATTRIBUTES + (ROOT / f'tools/fixtures/{source}.rs').read_text())
                manifest, package, app = project(app_folder)
                artifact = build(manifest, package, app)
                executable = overlay / 'app' / app.binary
                executable.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(artifact, executable)
                metadata = overlay / 'system/share/dunit/apps' / app.id / 'dunit.toml'
                metadata.parent.mkdir(parents=True)
                metadata.write_text(installed_manifest(app))
                return executable
            if fixture == 'console':
                executable = install_fixture(source_name, 'sdk_console_cli', 'Console CLI', 'cli')
                shutil.copyfile(executable, overlay / 'app/console command')
                install_fixture(source_name, 'sdk_console_tui', 'Console TUI', 'tui')
            elif fixture in ('transfer','clipboard'):
                executable=install_fixture(source_name,app_id,'Transfer source')
                target_id=f'sdk_{fixture}_target'
                metadata=overlay/'system/share/dunit/apps'/target_id/'dunit.toml'
                metadata.parent.mkdir(parents=True)
                document=(overlay/'system/share/dunit/apps'/app_id/'dunit.toml').read_text()
                metadata.write_text(document.replace(f'id = "{app_id}"',f'id = "{target_id}"'))
            else:
                install_fixture(source_name, app_id, f'SDK {fixture} fixture')
            if fixture == 'lifecycle':
                install_fixture('sdk_lifecycle_peer', 'sdk_lifecycle_peer', 'Legacy input peer')
            if fixture=='matrix':
                from dnd_matrix import prepare
                markers=prepare(overlay)
                command[command.index('--markers-file')+1]=str(markers)
                command[command.index('--timeout')+1]='1800'
                command[command.index('--cmd-timeout')+1]='1500'
            else:
                shutil.copyfile(ROOT / f'tools/fixtures/{source_name}.toml', overlay / config_name)
            if fixture in ('theme', 'effects', 'backdrop', 'motion'):
                profile = (ROOT / 'assets/dwm/sdk/dark.dss').read_text()
                if fixture == 'motion':
                    profile += (ROOT / 'tools/fixtures/sdk_motion_profile.dss').read_text()
                (overlay / 'sdk-theme.dss').write_text(profile)
            # DF/IRQ contracts also need the kernel's boot-smoke checks and
            # their full application composition. Other fixtures use DWM.
            kernel_features=['--features','boot-smoke-tests'] if image_profile=='full' else []
            subprocess.run([cargo(), 'build', '--release', *kernel_features,
                '-Z', 'build-std=core,alloc,compiler_builtins', '-Z', 'build-std-features=compiler-builtins-mem',
                '-Z', 'json-target-spec'], cwd=ROOT / 'kernel', check=True)
            command += ['--build', '--initrd-overlay', str(overlay)]
            command+=['--image-profile',image_profile,'--config',boot_config]
        if args.reuse_image and fixture=='matrix':
            from dnd_matrix import prepare
            overlay=Path(folder)/'overlay';overlay.mkdir()
            markers=prepare(overlay)
            command[command.index('--markers-file')+1]=str(markers)
            command[command.index('--timeout')+1]='1800'
            command[command.index('--cmd-timeout')+1]='1500'
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
