#!/usr/bin/env python3
"""Execution admission/handoff with original hardware, PMM and registry checks."""
from dunit_cli import ROOT
from test_process_registry import manifest as registry_manifest
from test_smp_boot import main as run_native


def manifest(cpus):
    result = registry_manifest(cpus)
    result['required'].append('[TASK-EXECUTION] exclusive admission, return handoff and retirement gates OK')
    result['forbidden'].append('[TASK-EXECUTION] FAIL')
    return result


if __name__ == '__main__':
    run_native(make_manifest=manifest, output_default=ROOT / 'build/performance/task-execution')
