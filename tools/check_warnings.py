#!/usr/bin/env python3
"""Check Rust warnings in the kernel and every full-profile application.

Makefile owns the app list. Touch crate roots to re-emit cached warnings;
--update may establish or lower tools/warnings_budget.json, never raise it.
C HAL and upstream C warnings are outside this Rust gate.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
BUDGET_FILE = REPO_ROOT / "tools" / "warnings_budget.json"

KERNEL_BUILD = [
    "cargo", "build", "--release", "--features", "boot-smoke-tests",
    "-Z", "build-std=core,alloc,compiler_builtins",
    "-Z", "build-std-features=compiler-builtins-mem",
    "-Z", "json-target-spec", "--message-format=json",
]
USERSPACE_BUILD = [
    "cargo", "build", "--release",
    "--target", "../../../userspace/x86_64-unknown-none.json",
    "-Z", "build-std=core,alloc",
    "-Z", "build-std-features=compiler-builtins-mem",
    "-Z", "json-target-spec", "--message-format=json",
]


def touch_crate(crate_dir: Path) -> None:
    """Bump mtime of the crate root so Cargo recompiles and re-emits warnings."""
    for candidate in ("src/lib.rs", "src/main.rs"):
        p = crate_dir / candidate
        if p.is_file():
            p.touch()


def count_rust_warnings(crate_dir: Path, build_cmd: list[str]) -> int:
    """Build a crate with JSON output; count warnings whose spans are in-repo."""
    touch_crate(crate_dir)
    proc = subprocess.run(build_cmd, cwd=crate_dir, capture_output=True, text=True)
    warns = 0
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-message":
            continue
        m = msg.get("message", {})
        if m.get("level") != "warning":
            continue
        spans = m.get("spans", [])
        # Only warnings anchored in our own source, not sysroot/deps.
        if any(("/src/" in s.get("file_name", "") or s.get("file_name", "").startswith("src/"))
               and "/.cargo/" not in s.get("file_name", "")
               for s in spans):
            warns += 1
    if proc.returncode != 0:
        sys.stderr.write(f"[check_warnings] build failed in {crate_dir}:\n")
        sys.stderr.write(proc.stderr[-2000:] + "\n")
        raise SystemExit(3)
    return warns


def measure() -> dict:
    apps = subprocess.check_output(
        ["make", "--no-print-directory", "-s", "list-userspace-apps", "IMAGE_PROFILE=full"],
        cwd=REPO_ROOT, text=True,
    ).split()
    if not apps or len(apps) != len(set(apps)):
        raise ValueError("Makefile must list distinct userspace applications")
    for app in apps:
        if not app.isascii() or not all(c.isalnum() or c in "_-" for c in app):
            raise ValueError(f"invalid userspace app: {app}")
        if not (REPO_ROOT / "userspace/system_apps" / app / "Cargo.toml").is_file():
            raise ValueError(f"missing userspace crate: {app}")
    cargo = subprocess.check_output(
        ["make", "--no-print-directory", "-s", "list-cargo"],
        cwd=REPO_ROOT, text=True,
    ).split()
    if not cargo:
        raise ValueError("Makefile must select Cargo")
    counts: dict[str, int] = {}
    counts["kernel"] = count_rust_warnings(REPO_ROOT / "kernel", cargo + KERNEL_BUILD[1:])
    userspace_total = 0
    for app in apps:
        app_dir = REPO_ROOT / "userspace" / "system_apps" / app
        userspace_total += count_rust_warnings(app_dir, cargo + USERSPACE_BUILD[1:])
    counts["userspace"] = userspace_total
    counts["total"] = counts["kernel"] + counts["userspace"]
    return counts


def load_budget() -> dict | None:
    if BUDGET_FILE.is_file():
        return json.loads(BUDGET_FILE.read_text(encoding="utf-8"))
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--update", action="store_true",
                    help="ratchet the budget down to the measured count and write it back")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    args = ap.parse_args()

    counts = measure()
    budget = load_budget()
    budget_total = budget.get("total") if budget else None

    over = budget_total is not None and counts["total"] > budget_total

    if args.update and not over:
        payload = {
            "description": "Compiler-warnings budget (M0). tools/check_warnings.py "
                           "fails if the measured count exceeds these. Ratchet down only.",
            "kernel": counts["kernel"],
            "userspace": counts["userspace"],
            "total": counts["total"],
        }
        BUDGET_FILE.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    if args.json:
        print(json.dumps({"measured": counts, "budget": budget_total, "over": over}, indent=2))
    else:
        print(f"kernel    warnings: {counts['kernel']}")
        print(f"userspace warnings: {counts['userspace']}")
        print(f"total     warnings: {counts['total']}")
        if budget_total is not None:
            print(f"budget    total   : {budget_total}")
        if args.update and not over:
            print(f"[check_warnings] budget written to {BUDGET_FILE}")
        elif over:
            print(f"[check_warnings] FAIL: {counts['total']} > budget {budget_total}. "
                  "Fix the warnings; --update cannot raise the budget.")
        elif budget_total is not None:
            print("[check_warnings] OK: within budget.")

    return 2 if over else 0


if __name__ == "__main__":
    raise SystemExit(main())
