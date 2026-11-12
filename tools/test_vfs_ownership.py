#!/usr/bin/env python3
"""Native owned-backend/lifetime and filesystem regression budget via qemu_test."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from dunit_cli import ROOT, cargo


SCENES = ('m6_musl_fsmgmt', 'm6_fileops', 'm6_lseek', 'm6_fstat', 'm6_dup',
          'm6_direxcl', 'm6_native_fd', 'm6_thread_stress', 'm6_handle')


def contract():
    result = {'run': {'config': 'limine_test_terminal.conf', 'cmd': []},
              'required': ['[TERM-007] Starting main loop',
                           '[DF-TEST] syscall+irq flags preserved',
                           '[VFS-OWNERSHIP] namespace drop, unlink, pinned I/O and final close OK'],
              'forbidden': ['[PANIC]', '[FS-SMOKE] FAIL:',
                            {'marker': '[USER-FAULT]', 'after': '[TERM-007] Starting main loop'}]}
    for name in SCENES:
        scene = json.loads((ROOT / f'tools/{name}_markers.json').read_text())
        result['run']['cmd'] += scene['run']['cmd']
        for key in ('required', 'forbidden'):
            result[key] += scene[key]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reuse-image', action='store_true')
    parser.add_argument('--accel', choices=('tcg', 'kvm'), default='tcg')
    parser.add_argument('--cpus', type=int, default=4)
    parser.add_argument('--output', type=Path, default=ROOT / 'build/performance/vfs-ownership')
    args = parser.parse_args()
    if not 1 <= args.cpus <= 64:
        parser.error("supported CPU range is 1..64")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    markers = args.output / 'markers.json'
    markers.write_text(json.dumps(contract(), indent=2) + '\n')
    if not args.reuse_image:
        subprocess.run([cargo(), 'check', '--release', '--features', 'boot-smoke-tests',
                        '-Z', 'build-std=core,alloc,compiler_builtins', '-Z',
                        'build-std-features=compiler-builtins-mem', '-Z', 'json-target-spec'],
                       cwd=ROOT / 'kernel', check=True)
    subprocess.run([sys.executable, str(ROOT / 'tools/qemu_test.py'),
                    *([] if args.reuse_image else ['--build']), '--markers-file', str(markers),
                    '--accel', args.accel, '--cpus', str(args.cpus), '--timeout', '600',
                    '--cmd-timeout', '240', '--boot-timeout', '120', '--workdir', str(args.output),
                    '--performance-record', str(args.output / 'metadata.json')], cwd=ROOT, check=True)


if __name__ == '__main__':
    main()
