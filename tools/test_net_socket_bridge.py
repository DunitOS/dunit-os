#!/usr/bin/env python3
"""Exercise the actual fixed-size kernel socket bridge using the shared codec."""
import json
import os
from pathlib import Path
import re
import subprocess
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
KERNEL = ROOT / "kernel/src/syscall"
RUSTC = os.environ.get("RUSTC", "rustc")

with TemporaryDirectory() as directory:
    work = Path(directory)
    abi = work / "libdunit_socket_abi.rlib"
    subprocess.run(
        [RUSTC, "--edition=2021", "--crate-name=dunit_socket_abi", "--crate-type=rlib",
         "-Dwarnings", str(ROOT / "abi/socket/src/lib.rs"), "-o", str(abi)],
        check=True,
    )
    errors = re.findall(r"^pub const E[A-Z]+: i64 = -[0-9]+;",
                        (KERNEL / "mod.rs").read_text(), re.MULTILINE)
    source = work / "bridge.rs"
    source.write_text("\n".join(errors) + f"""
use dunit_socket_abi::*;

fn wire_error(_: WireError) -> i64 {{ EINVAL }}

#[derive(Clone, Copy)]
enum FdTarget {{
    NetSocket {{ token: u64 }},
    NetService {{ token: u64 }},
    Other,
}}

#[path = {json.dumps(str(KERNEL / "net_socket/state.rs"))}]
mod bridge;

#[test]
fn unknown_descriptor_has_no_socket_authority() {{
    assert_eq!(bridge::Bridge::new().ready(1, FdTarget::Other), 12);
}}
""")
    binary = work / "bridge-tests"
    subprocess.run(
        [RUSTC, "--edition=2021", "--test", "-Dwarnings", "--extern", f"dunit_socket_abi={abi}",
         str(source), "-o", str(binary)],
        check=True,
    )
    subprocess.run([str(binary)], check=True, timeout=60)
