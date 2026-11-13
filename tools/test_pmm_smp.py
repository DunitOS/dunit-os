#!/usr/bin/env python3
"""Actual BSP/AP physical allocations plus original hardware/runtime budget."""
from dunit_cli import ROOT
from test_smp_boot import main as run_native, manifest as boot_manifest


def manifest(cpus):
    result=boot_manifest(cpus)
    result['required'] += [f'[PMM-SMP] cpu={cpu} rounds=128' for cpu in range(cpus)]
    result['required'].append(f'[PMM-SMP] cpus={cpus} rounds=128 frames={cpus*128*4} unique/reclaimed OK')
    return result


if __name__=='__main__':
    run_native(make_manifest=manifest, output_default=ROOT/'build/performance/pmm-smp')
