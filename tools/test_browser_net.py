#!/usr/bin/env python3
"""Host unit test runner for dunit_browser_net."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CRATE = ROOT / "userspace" / "browser-net"


def main() -> int:
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo" / "bin" / "cargo")
    env = dict(os.environ)
    env["PATH"] = f"{Path.home() / '.cargo' / 'bin'}:{env.get('PATH', '')}"
    rustc_v = subprocess.check_output(["rustc", "--version", "--verbose"], text=True)
    host_target = next(
        line.split(": ", 1)[1].strip()
        for line in rustc_v.splitlines()
        if line.startswith("host:")
    )
    subprocess.run(
        [cargo, "test", "--target", host_target, "--manifest-path", str(CRATE / "Cargo.toml")],
        env=env,
        check=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

