#!/usr/bin/env python3
"""AP hardware/entry probes. Parked APs are explicitly not SMP task dispatch."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
from dunit_cli import ROOT, cargo


def manifest(cpus):
    return {
        "run": {
            "config": "limine_test_terminal.conf",
            "cmd": [
                "exec /system/bin/tcc /assets/tcc/performance.c -o /tmp/performance",
                "exec /tmp/performance",
            ],
        },
        "required": [
            "[TERM-007] Starting main loop",
            "[DF-TEST] syscall+irq flags preserved",
            f"[SMP] hardware_online={cpus} scheduler=up dispatch_cpus=1 APs=parked",
            "[PERF-CORE] all checks passed",
        ],
        "forbidden": [
            "[PANIC]",
            "tcc: error",
            {"marker": "[USER-FAULT]", "after": "[TERM-007] Starting main loop"},
        ],
    }


def verify(serial, cpus):
    text = Path(serial).read_text()
    rows = re.findall(
        r"\[SMP\] cpu=(\d+) apic=(\d+) ready=(\d+) ipi=(\d+) timer=(\d+) work=(\d+)",
        text,
    )
    if len(rows) != cpus:
        raise ValueError(f"expected {cpus} rows, got {rows}")
    values = [tuple(map(int, r)) for r in rows]
    if [r[0] for r in values] != list(range(cpus)) or len(
        {r[1] for r in values}
    ) != cpus:
        raise ValueError("incorrect CPU identity")
    for i, apic, ready, ipi, timer, work in values:
        if ready != 1 or timer < 2 or ipi != (cpus - 1 if i == 0 else 1):
            raise ValueError("missing actual timer/IPI progress")
        if work != (0 if i == 0 else 2147450880 * (i + 1)):
            raise ValueError("AP work checksum mismatch")
    if "[PERF-CORE] all checks passed" not in text:
        raise ValueError("UP runtime regression")
    return {
        "hardware_online": cpus,
        "dispatch_cpus": 1,
        "scheduler": "up",
        "ap_state": "parked",
        "cpus": values,
    }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--cpus", type=int, nargs="+", default=[1, 2, 4, 8])
    p.add_argument("--accel", choices=("tcg", "kvm"), default="tcg")
    p.add_argument("--reuse-image", action="store_true")
    p.add_argument("--output", type=Path, default=ROOT / "build/performance/smp-boot")
    a = p.parse_args()
    if any(n < 1 or n > 64 for n in a.cpus):
        p.error("supported hardware-probe range is 1..64")
    a.output.mkdir(parents=True, exist_ok=True)
    results = []
    for index, cpus in enumerate(a.cpus):
        folder = a.output / str(cpus)
        folder.mkdir(exist_ok=True)
        markers = folder / "markers.json"
        markers.write_text(json.dumps(manifest(cpus), indent=2) + "\n")
        build = index == 0 and not a.reuse_image
        if build:
            subprocess.run(
                [
                    cargo(),
                    "check",
                    "--release",
                    "--features",
                    "boot-smoke-tests",
                    "-Z",
                    "build-std=core,alloc,compiler_builtins",
                    "-Z",
                    "build-std-features=compiler-builtins-mem",
                    "-Z",
                    "json-target-spec",
                ],
                cwd=ROOT / "kernel",
                check=True,
            )
        command = [
            sys.executable,
            str(ROOT / "tools/qemu_test.py"),
            *(["--build"] if build else []),
            "--markers-file",
            str(markers),
            "--cpus",
            str(cpus),
            "--accel",
            a.accel,
            "--workdir",
            str(folder),
            "--performance-record",
            str(folder / "metadata.json"),
            "--boot-timeout",
            "90",
            "--timeout",
            "300",
            "--cmd-timeout",
            "180",
        ]
        with (folder / "run.log").open("w") as f:
            subprocess.run(
                command, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, check=True
            )
        result = verify(folder / "qemu-serial.full.log", cpus)
        results.append(result)
        print("[SMP-PROBE]", json.dumps(result), flush=True)
        (a.output / "results.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
