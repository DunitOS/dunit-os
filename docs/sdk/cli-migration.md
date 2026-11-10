# M7.11: first CLI migration block

`dufetch` now uses `#[dunit::main]`, checked SDK output and the existing
`system`, `process` and `fs` adapters. Its logo, logical ANSI colors, labels,
spacing, number formatting and conditional facts remain application presentation.
The pure `report.rs` accepts a fact snapshot and an SDK `Write` implementation;
it performs no syscalls. There is one formatter in production.

The application no longer owns a loader entry/panic handler, raw stdout writes
or a second cwd reader. Partial writes are completed and errors propagate to the
SDK entry diagnostic/exit status. A failed stats/cwd query no longer silently
pretends zero stats or `/`. Display absence still omits the display line.

`system::display_info()` wraps the existing framebuffer query and returns only
width, height and pitch, without exposing the mapping address. It introduces no
syscall, kernel field, wire format or privilege change. This is not a new uname
contract: unavailable OS/kernel/platform identity is still not fabricated.

`wc_demo` already uses native musl stdio, which is the C SDK's file/stream API.
Its migration consists of consuming the generated `<dunit.h>` umbrella; the
portable counting/file logic and native libc implementation are unchanged.
There is no parallel C stream wrapper and no rewrite merely to increase scope.

## Acceptance

Before removing the old `dufetch` entry/I/O code, both original applications
passed `sdk_cli_migration_markers.json` in QEMU TCG. The C test parent, itself
compiled by TCC inside Dunit, redirects real stdout/stderr using native spawn
file actions, waits for and validates each child status, and reads back files:

- `dufetch` exits 0 with nonempty stdout and empty stderr. Its actual UTF-8 bytes
  are exported as bounded hex lines, avoiding serial tagging/chunking ambiguity.
- `wc_demo` with its default path produces exactly
  `wc demo.txt -> 3 lines, 13 words, 61 bytes\n` and exits 0.
- An explicit `/tmp/m711-wc.txt` produces the corresponding exact line and exits 0.
- A missing parent produces empty stdout, exact
  `fopen(w): No such file or directory\n` on stderr and exits 1.

The identical native contract passes after migration. The existing, unchanged
`tools/minimal_markers.json` also passes on actual rebuilt Minimal release ISOs
before and after the application change; image composition is checked separately.
Minimal retains six files and no GUI runtime dependency.

`tools/fixtures/dufetch_stdout.json` records the old source commit/hash and five
complete stdout snapshots produced by that old renderer before migration. It
covers unavailable facts, seconds/minutes/hours, missing/invalid display geometry,
unknown tick frequency, large counters, Unicode cwd, optional IPC/NIC lines and
ANSI swatches. The fixture contains expected bytes, not a retained legacy
implementation. The current production formatter must match every byte for the
same input. A writer limited to three bytes verifies complete writes; injected EIO
at byte 17 verifies propagation and the exact written prefix for every snapshot.
The unchanged C program also compiles/runs against the umbrella header on Linux
with identical output and file bytes.

Live kernel counters, uptime and PID cannot be byte-identical across processes
and reboots. Their native format is validated and only those numeric values are
masked before comparing with the pre-migration report. All units, punctuation,
labels, ANSI, Unicode logo, spacing, display geometry and cwd remain exact. The
fixed-snapshot comparison additionally checks every numeric value without masking.
Do not replace the frozen baseline merely to hide a regression.

```sh
python3 tools/test_cli_sdk_migration.py --host-only
python3 tools/test_cli_sdk_migration.py --accel tcg
python3 tools/qemu_test.py --build --variant minimal \
  --markers-file tools/minimal_markers.json --accel tcg \
  --boot-timeout 180 --cmd-timeout 90 --timeout 360
python3 tools/check_image_profile.py build/dunit_minimal.iso minimal
```

OS runs/builds use only `qemu_test.py`; the migration runner performs the required
matching kernel quick build first. `--reuse-image` is for an already current Full
image. Receipts/raw captures stay under ignored `build/`. SDK stream/loader tests
also pass (nine tests across lib, `streams` and `entry_data`).

This completes the first pair of M7.11 applications. Remaining CLI/TUI, GUI and
service migrations and the separate platform-identity decision stay open.
