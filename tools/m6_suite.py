#!/usr/bin/env python3
"""Run existing M6 marker contracts together through qemu_test.py.

This composes complete contracts, preserving every required/forbidden marker
and command action. Disk/power tests have different VM lifecycles and remain
separate; they are not silently treated as passed by this runner.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CORE = ['m6_profile_markers.json', 'pty_args_markers.json',
        'libc_test_markers.json', 'm6_musl_time_markers.json', 'lua_system_markers.json']
SEPARATE = {'m6_poweroff_markers.json', 'm6_reboot_markers.json',
            'm6_installer_markers.json', 'm6_persist_write_markers.json',
            'm6_persist_verify_markers.json', 'm6_profile_markers.json'}


def compose(names):
    result = {'sources': names,
              'run': {'config': 'limine_test_terminal.conf', 'cmd': [], 'command_actions': {}},
              'required': [], 'forbidden': []}
    for name in names:
        source = json.loads((ROOT / 'tools' / name).read_text())
        run = source.get('run', {})
        result['run']['cmd'].extend(run.get('cmd', []))
        for command, actions in run.get('command_actions', {}).items():
            old = result['run']['command_actions'].get(command)
            if old is not None and old != actions:
                raise ValueError('conflicting input actions for ' + command)
            result['run']['command_actions'][command] = actions
        if run.get('actions'):
            raise ValueError('global actions need a separate lifecycle: ' + name)
        result['required'].extend(source['required'])
        result['forbidden'].extend(source['forbidden'])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--group', choices=['core', 'regression'], required=True)
    parser.add_argument('--build', action='store_true')
    parser.add_argument('--workdir', type=Path, required=True)
    parser.add_argument('--mem', default='512M')
    args = parser.parse_args()
    names = CORE if args.group == 'core' else sorted(
        path.name for path in (ROOT/'tools').glob('m6_*markers.json') if path.name not in SEPARATE
    ) + ['svg_markers.json', 'icon_packs_markers.json', 'setup_markers.json',
         'themes_terminal_markers.json', 'lua_desktop_markers.json']
    manifest = compose(names)
    workdir = args.workdir.resolve()
    workdir.mkdir(parents=True, exist_ok=True)
    path = workdir / 'contracts.json'
    path.write_text(json.dumps(manifest, indent=2) + '\n')
    print(f"[M6-SUITE] {len(names)} complete contracts, {len(manifest['run']['cmd'])} commands", flush=True)
    command = [sys.executable, str(ROOT/'tools/qemu_test.py'), '--markers-file', str(path),
               '--workdir', str(workdir), '--mem', args.mem, '--timeout', '1500',
               '--boot-timeout', '180', '--cmd-timeout', '120', '--settle', '3']
    if args.build: command.append('--build')
    return subprocess.run(command, cwd=ROOT).returncode


if __name__ == '__main__': raise SystemExit(main())
