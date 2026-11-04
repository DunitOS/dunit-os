#!/usr/bin/env python3
"""Validate the bounded external-shell probe and report observations, not extraction claims."""
import argparse
import json
from pathlib import Path
import re


def verify(serial):
    ready=re.search(r'\[SHELLPROBE\] ready pid=(\d+)',serial)
    if not ready:raise ValueError('missing probe identity')
    scene=serial[ready.start():]
    faults=re.findall(r'\[USER-FAULT\] pid=(\d+)[^\r\n]*reason=([^\s]+)',scene)
    if faults!=[(ready[1],'invalid-opcode')]:
        raise ValueError(f'expected only the intentional panel fault: {faults}')
    if f'[GUI-EXIT] app=sdk_shellprobe_fixture kind=3 code=-4' not in scene:
        raise ValueError('faulted panel was not reaped as invalid opcode')
    for marker in ('[SHELLPROBE] survivor 24 commits after peer loss OK',
                   '[GUI-EXIT] app=sdk_shell_survivor kind=0 code=0'):
        if marker not in scene:raise ValueError(f'missing {marker}')
    reaped='[GUI-EXIT] app=sdk_shellprobe_fixture kind=3 code=-4'
    armed='[SHELLPROBE] survivor armed after reap'
    completed='[SHELLPROBE] survivor 24 commits after peer loss OK'
    if armed not in scene or not scene.index(reaped)<scene.index(armed)<scene.index(completed):
        raise ValueError('survivor publications must start after observed peer reap')
    metrics=re.search(r'\[SHELLPROBE\] metrics ([^\r\n]+)',scene)
    if not metrics:raise ValueError('missing panel publication metrics')
    def counter(name):
        m=re.search(r'\b'+name+r': (\d+)',metrics[1])
        if not m:raise ValueError('missing '+name)
        return int(m[1])
    commits=counter('commits');frames=counter('completed_frames')
    if commits!=24 or frames!=24:raise ValueError(f'expected 24 actual publications: {commits}/{frames}')
    if re.search(r'clock_failures: [1-9]',metrics[1]):raise ValueError('probe clock failed')
    stages={}
    for name in ('render','redraw_total','import_attach','commit_result','frame_done','retire'):
        m=re.search(r'\b'+name+r': MetricTime \{ samples: (\d+), total_ns: (\d+), max_ns: (\d+), clock_failures: 0 \}',metrics[1])
        if not m:raise ValueError('missing stage '+name)
        stages[name+'_ms']=int(m[2])/1_000_000
    return {'commits':commits,'completed_frames':frames,'input_qmax':counter('input_queue_high_water'),
            'stages':stages,'isolated_fault':True,'survivor_commits_after_fault':24}


def summarize(timing,serial):
    result=verify(Path(serial).read_text(errors='replace'))
    log=Path(timing).read_text()
    if 'Result    : OK (completed)' not in log:raise ValueError('QEMU probe failed')
    phases={}
    for line in log.splitlines():
        if line.startswith('[INPUT-TIMING] '):
            item=json.loads(line.split(' ',1)[1]);phases[item['phase']]=item['wall_ns']/1_000_000
    if set(phases)!={'external-shell-boot','external-shell-input','shell-fault-recovery'}:
        raise ValueError('missing host timings')
    result['host_wall_ms']=phases
    return result

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('timing',type=Path);parser.add_argument('serial',type=Path)
    parser.add_argument('--output',type=Path);args=parser.parse_args()
    result=summarize(args.timing,args.serial);encoded=json.dumps(result,indent=2)+'\n'
    if args.output:args.output.write_text(encoded)
    print(encoded,end='')
