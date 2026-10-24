# Pinned upstream libc-test subset

The independent upstream suite is [libc-test](https://wiki.musl-libc.org/libc-test),
maintained as part of musl. The official mirror was fetched with
`git clone --depth 1 git://repo.or.cz/libc-test.git`; its HEAD is pinned to
`7b95dfa5f5d5ca4d949221e0228ccc290bacc14e` (2026-08-25).
The author's original repository is `git://nsz.repo.hu:49100/repo/libc-test`.
Vendored files retain their exact upstream bytes, AUTHORS and COPYRIGHT.
The vendored subset is MIT; additional BSD/GPL licenses apply to portions of
upstream math vector data, which this subset does not copy.

## Scope and evidence

`tests/libc/manifest.json` inventories **all 435** `src/*/*.c` paths at the pin,
using exactly the upstream root Makefile `SRCS` scope. Every record identifies
its SHA-256, status, reason and responsible milestone/domain. Common harness
sources are support, not successful test cases. Nested math vector headers are
inputs to cases rather than separate executables. Nothing omitted from the
subset is implicitly considered passing, failed or unsupported.

Twelve original functional cases are included:

| Area | Original cases |
| --- | --- |
| Strings | `string` |
| Integer conversion | `strtol`, `wcstol` |
| Floating conversion | `strtod_simple`, `strtof` |
| Formatted I/O | `snprintf`, `sscanf`, `swprintf` |
| File stream pushback | `ungetc` (real `tmpfile`, seeking, reading) |
| Sorting | `qsort` |
| Generic mathematics | `tgmath` |
| Unsigned arithmetic | `udiv` |

This is a bounded baseline, **not the full ISO C/POSIX or libm suite**. Full
math vectors, API compile-only checks, process/thread/loader regressions and
additional functional/regression cases remain explicitly deferred for review
and native integration. Networking coverage belongs to M8. Deferral is not an
assertion that a case requires missing kernel support.

Original `common/test.h`, `common/print.c` and `common/rand.c` are reused.
`tests/libc/runner.c` is a separate adapter: each case's `main` is renamed only
with `-Dmain=libc_test_case_main` while compiling its own translation unit. No
upstream source, assertion, expected value or diagnostics are edited. Each
executable gets its own original `t_status` state. The adapter reports PASS only
if both the original case return and upstream `t_status` are zero. Crashes,
missing output and timeouts cannot produce the required PASS marker.

## Integration

The build tool verifies vendored hashes before invoking the compiler and fails
on any compiler/linker failure; it never runs host binaries as native evidence.
The supplied compiler is the native Dunit musl wrapper, not a Linux ABI adapter:

```sh
python3 tools/libc_test.py verify
python3 tools/libc_test.py build \
  --cc "$PWD/build/dunit-sysroot/bin/dunit-cc" \
  --out-dir build/userspace
```

Call build after sysroot preparation and before packing userspace binaries into
the initrd. Objects are temporary and do not enter the binary directory. The
output names are listed by `python3 tools/libc_test.py list`.
Run the OS only through the project's existing harness:

```sh
python3 tools/qemu_test.py --build \
  --markers-file tools/libc_test_markers.json \
  --timeout 420 --boot-timeout 180 --cmd-timeout 90
```

All twelve PASS markers are required, along with terminal boot. A nonzero
original return or nonzero `t_status` is a failure, never a skip. Existing
negative boot-isolation tests may deliberately generate USER-FAULT messages;
the individual libc cases still must return their own success marker. The
markers do not weaken the suite's original diagnostics.

For provenance checks against a locally fetched upstream checkout:

```sh
python3 tools/libc_test.py verify --upstream /tmp/dunit-libc-test-upstream
```

This verifies the commit, complete source inventory and every original SHA.
Do not update hashes to disguise local changes: an intentional suite update
requires reviewing the new upstream revision, inventory and original sources.

## Validation status

Source provenance and orchestration host tests are verified with:

```sh
python3 tools/libc_test.py self-test
```

The host tests validate source pins, local inventory, exact marker coverage,
and rejection of modified source pins/missing case rationale. They do not
establish native correctness. Native compiler and QEMU results must be recorded
by the integrating system run before marking this milestone complete.
