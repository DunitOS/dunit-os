#!/usr/bin/env python3
"""Registry/task ownership with original hardware, PMM and fixed-work budgets."""
from dunit_cli import ROOT
from test_pmm_smp import manifest as pmm_manifest
from test_smp_boot import main as run_native


def manifest(cpus):
    result=pmm_manifest(cpus)
    result['required'].append('[TASK-REGISTRY] callback pin, detached cleanup, dispatch pin and reentry OK')
    result['forbidden'].append('[TASK-REGISTRY] FAIL')
    return result


if __name__=='__main__':
    run_native(make_manifest=manifest, output_default=ROOT/'build/performance/process-registry')
