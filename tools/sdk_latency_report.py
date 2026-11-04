#!/usr/bin/env python3
"""Summarize identical QEMU input scenarios; wall times are not scanout latency."""
import argparse
import json
from pathlib import Path
import re
import statistics


def summarize(timing, serial):
    phases = {}
    for raw in Path(timing).read_text().splitlines():
        if raw.startswith('[INPUT-TIMING] '):
            item = json.loads(raw.split(' ', 1)[1])
            phases[item['phase']] = {'wall_ms': item['wall_ns'] / 1_000_000}
    expected = {f'{case}-{n}' for case in ('wheel', 'short', 'long', 'drag') for n in range(1, 4)}
    if set(phases) not in (expected, expected | {'wheel-burst'}):
        raise ValueError(f'{timing}: expected all twelve input phases and only the optional wheel burst')
    if 'Result    : OK (completed)' not in Path(timing).read_text():
        raise ValueError(f'{timing}: QEMU scenario did not complete successfully')
    def field(text, name):
        match = re.search(r'\b' + re.escape(name) + r': (\d+)', text)
        if not match:
            raise ValueError(f'missing counter {name}')
        return int(match[1])
    def duration(text, name):
        match = re.search(r'\b' + re.escape(name) + r': MetricTime \{ samples: (\d+), total_ns: (\d+), max_ns: (\d+), clock_failures: (\d+) \}', text)
        if not match or int(match[4]):
            raise ValueError(f'missing timing or failed clock: {name}')
        return int(match[2]) / 1_000_000
    for line in Path(serial).read_text().splitlines():
        match = re.search(r'\[PERF-METRICS\] phase=([a-z]+-\d+) accepted=(\d+) (.*)', line)
        if not match:
            continue
        label, accepted, text = match.groups()
        if label == 'wheel-4' and 'wheel-burst' in phases:
            label = 'wheel-burst'
        if label not in phases:
            raise ValueError(f'unmeasured phase {label}')
        if not label.startswith('drag-') and int(accepted) != 24:
            raise ValueError(f'{label}: input was lost')
        phases[label].update(accepted=int(accepted), render_ms=duration(text, 'render'),
            redraw_ms=duration(text, 'redraw_total'), import_attach_ms=duration(text, 'import_attach'),
            commit_result_ms=duration(text, 'commit_result'), frame_done_ms=duration(text, 'frame_done'),
            retire_ms=duration(text, 'retire'), redraws=field(text, 'redraw_calls'),
            commits=field(text, 'commits'), input_qmax=field(text, 'input_queue_high_water'))
    result = {}
    cases = [(case, [f'{case}-{n}' for n in range(1, 4)])
             for case in ('wheel', 'short', 'long', 'drag')]
    if 'wheel-burst' in phases:
        cases.append(('wheel_burst', ['wheel-burst']))
    for case, labels in cases:
        trials = [phases[label] for label in labels]
        if any('render_ms' not in t for t in trials):
            raise ValueError(f'{case}: missing client metrics')
        result[case] = {'trials': trials, 'median': {
            key: statistics.median(t[key] for t in trials)
            for key in trials[0] if key != 'accepted'},
            'max_input_q': max(t['input_qmax'] for t in trials)}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', nargs=3, action='append', required=True, metavar=('LABEL', 'TIMING_LOG', 'SERIAL_LOG'))
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    result = {label: summarize(timing, serial) for label, timing, serial in args.run}
    if args.output:
        args.output.write_text(json.dumps(result, indent=2) + '\n')
    print('| Run | Scene | Median ms | Range ms | Render ms | FrameDone ms | Input qmax |')
    print('| --- | --- | ---: | --- | ---: | ---: | ---: |')
    for label, cases in result.items():
        for case, data in cases.items():
            m = data['median']; samples = [x['wall_ms'] for x in data['trials']]
            print(f"| {label} | {case} | {m['wall_ms']:.1f} | {min(samples):.1f}–{max(samples):.1f} | {m['render_ms']:.1f} | {m['frame_done_ms']:.1f} | {data['max_input_q']} |")


if __name__ == '__main__':
    main()
