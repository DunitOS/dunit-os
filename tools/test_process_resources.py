#!/usr/bin/env python3
"""Process teardown, fd inheritance/rollback and native lifecycle regressions."""
from test_vfs_ownership import contract as vfs_contract, main as run_vfs
from dunit_cli import ROOT
import json
import subprocess
import sys


def contract():
    result = vfs_contract()
    result['required'].append('[PROCESS-RESOURCES] detach, dup, rollback, kill EOF/cap close before wait OK')
    result['forbidden'].append('[PROCESS-RESOURCES] FAIL')
    result['required'].append('[DISPATCH-PIN] stack/FPU/address-space survive Process drop and reclaim after frame OK')
    for name in ('m6_reap', 'waitset', 'pty_args',
                 'm6_musl_spawn', 'm6_pipe', 'm6_cond'):
        scene = json.loads((ROOT / f'tools/{name}_markers.json').read_text())
        result['run']['cmd'] += scene['run']['cmd']
        for key in ('required', 'forbidden'):
            result[key] += scene[key]
    return result


if __name__ == '__main__':
    args = run_vfs(make_contract=contract, output_default=ROOT / 'build/performance/process-resources')
    # This scene intentionally faults one user thread. Keep its original budget
    # in its own VM instead of relaxing the no-fault filesystem regression run.
    folder = args.output / 'thread-lifecycle'
    folder.mkdir(exist_ok=True)
    scene = json.loads((ROOT / 'tools/m6_thread_lifecycle_markers.json').read_text())
    scene['required'] += ['[TERM-007] Starting main loop', '[DF-TEST] syscall+irq flags preserved',
                          '[DISPATCH-PIN] stack/FPU/address-space survive Process drop and reclaim after frame OK']
    markers = folder / 'markers.json'
    markers.write_text(json.dumps(scene, indent=2) + '\n')
    subprocess.run([sys.executable, str(ROOT / 'tools/qemu_test.py'), '--markers-file', str(markers),
                    '--accel', args.accel, '--cpus', str(args.cpus), '--timeout', '360',
                    '--cmd-timeout', '240', '--boot-timeout', '120', '--workdir', str(folder),
                    '--performance-record', str(folder / 'metadata.json')], cwd=ROOT, check=True)
