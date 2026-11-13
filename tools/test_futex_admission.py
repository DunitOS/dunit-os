#!/usr/bin/env python3
"""Native futex admission, musl mutex/join, barrier and condvar regressions."""
import json
from dunit_cli import ROOT
from test_vfs_ownership import main as run_native


def contract():
    result = json.loads((ROOT / 'tools/futex_markers.json').read_text())
    for name in ('m6_musl_thread', 'm6_profile', 'm6_cond'):
        scene = json.loads((ROOT / f'tools/{name}_markers.json').read_text())
        result['run']['cmd'] += scene['run']['cmd']
        for key in ('required', 'forbidden'):
            result[key] += scene[key]
    return result


if __name__ == '__main__':
    run_native(make_contract=contract, output_default=ROOT / 'build/performance/futex-admission-matrix')
