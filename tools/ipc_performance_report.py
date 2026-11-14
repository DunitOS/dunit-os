#!/usr/bin/env python3
"""Validate native fixed-work IPC samples; output measured round-trip medians."""
import argparse
import json
from pathlib import Path
import re
import statistics


def verify(serial):
    records=[json.loads(value) for value in re.findall(r'\[PERF-IPC\] (\{[^\n]+\})',serial)]
    if len(records)!=6 or [r['trial'] for r in records]!=list(range(6)):
        raise ValueError('expected warmup plus five ordered IPC records')
    for record in records:
        trial=record['trial']
        expected=sum((i*16777619)^(trial*0x9e3779b9)^((0x123456789abcdef0+i)^0xfedcba9876543210) for i in range(1000))&((1<<64)-1)
        if record['round_trips']!=1000 or record['bytes']!=64000 or record['checksum']!=expected or record['elapsed_ns']<=0:
            raise ValueError('IPC work/checksum/time mismatch')
    if '[PERF-IPC] all checks passed' not in serial or '[PERF-IPC] FAIL' in serial:
        raise ValueError('IPC execution did not pass')
    return {'records':records,'median_round_trip_ns':statistics.median(r['elapsed_ns']/1000 for r in records[1:]),
            'scope':'UP round-trip work; not scheduler SMP scaling, per-input tails or physical scanout'}


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('serial',type=Path)
    args=parser.parse_args();print(json.dumps(verify(args.serial.read_text()),indent=2))
