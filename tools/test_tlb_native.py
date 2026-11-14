#!/usr/bin/env python3
"""Actual AP mappings, NMI acknowledgement and reclaim with existing core budgets."""
from dunit_cli import ROOT
from test_clock_native import manifest as clock_manifest
from test_smp_boot import main as run_native


def manifest(cpus):
    result=clock_manifest(cpus)
    result['required'].append(f'[TLB-SMP] cpus={cpus} rounds=32 AP retarget, IRQ-off NMI acks and final reclaim OK')
    result['forbidden'].append('[TLB-SMP] FAIL')
    result['required'].append('[USER-MAPPING] source pin and task-scope copy survive object drop OK')
    return result


if __name__=='__main__':
    run_native(make_manifest=manifest,output_default=ROOT/'build/performance/tlb-shootdown')
