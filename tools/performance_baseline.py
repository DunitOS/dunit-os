#!/usr/bin/env python3
"""OP0: checked fixed work, five measured UI trials, explicit platform metadata."""

import argparse
import copy
import json
import math
from pathlib import Path
import re
import statistics
import subprocess
import sys
from dunit_cli import ROOT
from sdk_latency_report import parse_phases


def replace(value, old, new):
    if isinstance(value, str):
        return value.replace(old, new)
    if isinstance(value, list):
        return [replace(x, old, new) for x in value]
    if isinstance(value, dict):
        return {k: replace(v, old, new) for k, v in value.items()}
    return value


def contract():
    d = json.loads((ROOT / "tools/sdk_latency_markers.json").read_text())
    actions = d["run"]["command_actions"]["exec /app/gui_server"]
    blocks = {}
    for i, a in enumerate(actions):
        marker = a.get("wait_marker", "")
        if re.fullmatch(r"\[PERF\] \w+-\d+ setup", marker):
            label = marker.split()[1]
            end = next(
                j
                for j in range(i, len(actions))
                if actions[j].get("measure_end")
                == ("wheel-burst" if label == "wheel-4" else label)
            )
            blocks[label] = copy.deepcopy(actions[i - 2 : end + 1])
    expanded = copy.deepcopy(actions[:2])
    for mode in ("wheel", "short", "long", "drag"):
        for trial in (range(1, 8) if mode == "wheel" else range(1, 7)):
            if f"{mode}-{trial}" in blocks:
                expanded += blocks[f"{mode}-{trial}"]
            else:
                expanded += replace(blocks[f"{mode}-3"], f"{mode}-3", f"{mode}-{trial}")
    burst_end = next(
        i for i, a in enumerate(actions) if a.get("measure_end") == "wheel-burst"
    )
    expanded += copy.deepcopy(actions[burst_end + 1 :])
    d["run"]["image_profile"] = "full"
    d["run"]["fixture_release_profile"] = "size"
    d["run"]["command_actions"]["exec /app/gui_server"] = expanded
    d["run"]["cmd"][:0] = [
        "exec /system/bin/tcc /assets/tcc/performance.c -o /tmp/performance",
        "exec /tmp/performance",
    ]
    d["required"] += ["[PERF-CORE] all checks passed"]
    d["forbidden"] += [
        {"marker": "[USER-FAULT]", "after": "[PERF-CORE] start"},
        "tcc: error",
    ]
    return d


def stats(values):
    ordered = sorted(values)
    return {
        "samples": len(values),
        "p50": statistics.median(values),
        "p95": ordered[math.ceil(len(values) * 0.95) - 1],
        "p99": ordered[math.ceil(len(values) * 0.99) - 1],
        "min": min(values),
        "max": max(values),
        "raw": values,
        "tails": "empirical nearest rank; five trials do not establish an event-latency SLA",
    }


def report(folder):
    folder = Path(folder)
    timing = folder / "timing.log"
    serial = folder / "qemu-serial.full.log"
    phases = parse_phases(timing, serial)
    scenes = {}
    for mode in ("wheel", "short", "long", "drag"):
        labels = [
            f"{mode}-{n}" for n in ([2, 3, 5, 6, 7] if mode == "wheel" else range(2, 7))
        ]
        scenes[mode] = {
            "stream_completion_ms": stats([phases[k]["wall_ms"] for k in labels]),
            "trials": [phases[k] for k in labels],
            "warmup": phases[f"{mode}-1"],
            "physical_input_to_photon": None,
        }
    scenes["wheel-burst"] = phases["wheel-burst"]
    core = {}
    for raw in serial.read_text().splitlines():
        if "[PERF-CORE] {" in raw:
            item = json.loads(raw[raw.index("{") :])
            scene = item["scene"]
            trial = item["trial"]
            group = core.setdefault(scene, {})
            if trial in group:
                raise ValueError("duplicate core trial")
            group[trial] = item
    if set(core) != {"cpu-1", "cpu-2", "cpu-4", "cpu-8", "allocator", "memfs-io"}:
        raise ValueError("missing core scenes")
    metadata = json.loads((folder / "metadata.json").read_text())
    for scene, trials in core.items():
        if set(trials) != set(range(6)):
            raise ValueError("missing core trials")
        expected = {
            "cpu-1": (4194304, 9007160971231232),
            "cpu-2": (4194304, 9007160971231232),
            "cpu-4": (4194304, 9007160971231232),
            "cpu-8": (4194304, 9007160971231232),
            "allocator": (16777216, 528482304),
            "memfs-io": (2097152, 7340032),
        }[scene]
        if any((v["work_units"], v["checksum"]) != expected for v in trials.values()):
            raise ValueError("incorrect fixed work")
        core[scene] = {
            "guest_elapsed_ms": stats(
                [trials[n]["elapsed_ns"] / 1e6 for n in range(1, 6)]
            ),
            "clock_resolution_ms": (metadata["clock"]["guest_resolution_ns"] / 1e6 if metadata["clock"]["guest_resolution_ns"] is not None else None),
            "irq_off_may_undercount": metadata["clock"]["guest_source"] != "hpet",
            "trials": list(trials.values()),
        }
    if not metadata["result"]["ok"]:
        raise ValueError("failed QEMU metadata")
    result = {
        "schema_version": 1,
        "metadata": metadata,
        "ui": scenes,
        "core": core,
        "unmeasured": [
            "guest CPU accounting",
            "physical scanout",
            "per-event p95/p99",
            "KVM scaling",
            "allocation counts",
            "kernel pointer queue occupancy",
            "IPC microbenchmark",
        ],
        "interpretation": "UI times include injection and polling; core times are PIT elapsed, not CPU time. Thread counts are not kernel online CPUs.",
    }
    (folder / "baseline.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--accel", choices=("tcg", "kvm"), default="tcg")
    parser.add_argument("--cpus", type=int, default=1)
    parser.add_argument("--report-only", action="store_true")
    parser.add_argument("--reuse-image", action="store_true")
    args = parser.parse_args()
    folder = args.output.resolve()
    folder.mkdir(parents=True, exist_ok=True)
    if not args.report_only:
        manifest = folder / "markers.json"
        manifest.write_text(json.dumps(contract(), indent=2) + "\n")
        command = [
            sys.executable,
            str(ROOT / "tools/test_sdk_gui_fixture.py"),
            "latency",
            "--markers-file",
            str(manifest),
            "--run-dir",
            str(folder),
            "--accel",
            args.accel,
            "--cpus",
            str(args.cpus),
            "--timeout",
            "900",
            "--cmd-timeout",
            "600",
        ]
        if args.reuse_image:
            command += ["--reuse-image"]
        with (folder / "timing.log").open("w") as f:
            subprocess.run(
                command, cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, check=True
            )
    result = report(folder)
    for scene, data in result["ui"].items():
        if "stream_completion_ms" in data:
            print(scene, data["stream_completion_ms"])
    print("saved", folder / "baseline.json")


if __name__ == "__main__":
    main()
