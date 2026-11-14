#!/usr/bin/env python3
"""HPET clock, IRQ-off progress and cross-CPU clock brackets with core budgets."""
from dunit_cli import ROOT
from test_execution_native import manifest as execution_manifest
from test_smp_boot import main as run_native


def manifest(cpus):
    result=execution_manifest(cpus)
    result['required'] += ['[CLOCK] source=hpet',f'[CLOCK-SMP] bracketed_cpus={cpus}',
        '[CLOCK-PROBE] IRQ-off counter progress >=2000000 ns, PIT delivery unchanged OK']
    return result


if __name__=='__main__':
    run_native(make_manifest=manifest,output_default=ROOT/'build/performance/hardware-clock')
