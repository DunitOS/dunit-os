"""Reproducibility metadata; no credentials, environment dumps or guessed CPU time."""

from datetime import datetime, timezone
import hashlib
import os
from pathlib import Path
import platform
import subprocess


def command(args, cwd=None):
    try:
        return subprocess.check_output(
            args, cwd=cwd, text=True, stderr=subprocess.DEVNULL, timeout=15
        ).strip()
    except (OSError, subprocess.SubprocessError):
        return None


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def read(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return None


def cpu_count(command_line):
    value = "1"
    for i, arg in enumerate(command_line):
        if arg == "-smp":
            value = command_line[i + 1]
        elif arg.startswith("-smp="):
            value = arg[5:]
    first = value.split(",")[0]
    try:
        return int(first.removeprefix("cpus="))
    except ValueError:
        return None


def collect(root, image, qemu_command, profile):
    root = Path(root)
    model = next(
        (
            line.split(":", 1)[1].strip()
            for line in (read("/proc/cpuinfo") or "").splitlines()
            if line.startswith("model name")
        ),
        None,
    )
    quota = {
        name: read("/sys/fs/cgroup/" + name)
        for name in ("cpu.max", "cpuset.cpus.effective", "memory.max")
    }
    diff = command(["git", "diff", "HEAD", "--"], root)
    dirty = command(["git", "status", "--porcelain", "--untracked-files=normal"], root)
    untracked = (
        command(["git", "ls-files", "--others", "--exclude-standard"], root) or ""
    )
    untracked_hashes = {
        p: digest(root / p)
        for p in untracked.splitlines()
        if (root / p).is_file() and not (root / p).is_symlink()
    }
    machine = qemu_command[qemu_command.index("-machine") + 1]
    return {
        "schema_version": 1,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "git": {
            "commit": command(["git", "rev-parse", "HEAD"], root),
            "status": dirty,
            "tracked_diff_sha256": (
                hashlib.sha256(diff.encode()).hexdigest() if diff is not None else None
            ),
            "untracked_sha256": untracked_hashes,
        },
        "image": {
            "path": str(image),
            "sha256": digest(image),
            "bytes": Path(image).stat().st_size,
            "profile": profile,
        },
        "tools": {
            "qemu": command([qemu_command[0], "--version"]),
            "rustc": command(["rustc", "-Vv"]),
            "cc": command(["cc", "--version"]),
        },
        "guest": {
            "qemu_command": qemu_command,
            "machine": machine,
            "requested_vcpus": cpu_count(qemu_command),
            "emulator_vcpus": None,
            "kernel_online_cpus": None,
        },
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "cpu_model": model,
            "logical_cpus": os.cpu_count(),
            "affinity": (
                sorted(os.sched_getaffinity(0))
                if hasattr(os, "sched_getaffinity")
                else None
            ),
            "cgroup": quota,
            "kvm_accessible": os.access("/dev/kvm", os.R_OK | os.W_OK),
        },
        "clock": {
            "guest_source": None,
            "guest_resolution_ns": None,
            "guest_timer_event": None,
            "host": "time.monotonic",
            "physical_scanout_measured": False,
        },
        "measurements": {
            "launch_to_ready_wall_ms": None,
            "run_wall_ms": None,
            "cpu_time_ms": None,
            "rss_bytes": None,
        },
    }
