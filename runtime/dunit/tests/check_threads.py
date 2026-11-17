#!/usr/bin/env python3
"""Host checks of actual SDK ownership/sync code, without native syscalls."""
import os
from pathlib import Path
import re
import subprocess
import tempfile

SOURCE = Path(__file__).with_name("thread_sync.rs")


def main():
    compiler = os.environ.get("RUSTC", "rustc")
    with tempfile.TemporaryDirectory(prefix="dunit-sdk-threads-") as directory:
        directory = Path(directory)
        binary = directory / "threads"
        subprocess.run(
            [compiler, "--edition=2021", "-Dwarnings", "--test", str(SOURCE), "-o", str(binary)],
            check=True,
        )
        subprocess.run([str(binary), "--test-threads=1"], check=True, timeout=60)
        source = re.sub(
            r'#\[path = "([^"]+)"\]',
            lambda match: '#[path = "' + str((SOURCE.parent / match[1]).resolve()) + '"]',
            SOURCE.read_text(),
        )
        negative = [
            ("guard-send", "fn require<T: Send>() {} require::<sync::MutexGuard<'static, usize>>();", "E0277"),
            ("guard-cell-sync", "fn require<T: Sync>() {} require::<sync::MutexGuard<'static, std::cell::Cell<usize>>>();", "E0277"),
            ("non-send-closure", "let captured=std::rc::Rc::new(1); let _=thread::spawn(move||captured);", "E0277"),
            ("borrowed-closure", "let value=4; let _=thread::spawn(||value);", "E0373"),
        ]
        for name, body, diagnostic in negative:
            path = directory / (name + ".rs")
            path.write_text(source + "\n#[test] fn rejected_contract() {" + body + "}\n")
            result = subprocess.run(
                [compiler, "--edition=2021", "--test", str(path), "-o", str(binary)],
                capture_output=True, text=True,
            )
            if result.returncode == 0 or diagnostic not in result.stderr:
                raise AssertionError(f"{name}: expected {diagnostic} rejection\n{result.stderr}")
            print(f"[SDK-THREADS-HOST] {name}: rejected as required", flush=True)


if __name__ == "__main__":
    main()
