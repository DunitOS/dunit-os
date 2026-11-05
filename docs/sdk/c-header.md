# Native C entry header — M7.10 first increment

`#include <dunit.h>` imports the existing native musl file/process interfaces and
Dunit's generated syscall, errno, rights and transfer constants. This increment
provides no GUI entry points and does not define a new C UI runtime ABI.

`tools/gen_abi.py` generates the umbrella header; its included Dunit headers
continue to come from the existing ABI manifests. No syscall numbers or error,
rights or transfer values are copied into the umbrella. `--check` checks it too.
No kernel ABI, permissions or GUI wire format changes are required.

The libc APIs keep their existing semantics: for example, `open`/`read` report
`-1` and set `errno`, while `posix_spawn` returns an error number. `SYS_*` values
are native Dunit syscall numbers. Their visibility is not a promise of Linux
compatibility or of unsupported POSIX operations; the existing native profile
and documented refusals still apply.

`tools/ensure_musl_sysroot.py` installs the common headers under the host sysroot's
`include/` after both a libc rebuild and a cache hit. Changed headers are copied
without rebuilding unchanged libc or rewriting unchanged headers. The existing
image packer publishes them under `/system/include/`; on-device TCC already
searches that sysroot. This increment retains that established layout.

## Validation

Five host regressions cover generated includes without copied constants,
fresh installation, changed headers, unchanged timestamps, libc cache hits and
rebuilds. The existing nine image-profile checks also pass.

The native contract compiles `/assets/tcc/sdk_core.c` with TCC **inside Dunit**,
links it against the installed libc and runs the result. It checks actual
file write/seek/read/stat/unlink behavior, `EBADF`/`ENOENT`, and spawning/waiting
for a child that returns 7. Duplicate inclusion of `<dunit.h>` also compiles.

```sh
python3 tools/gen_abi.py --check
python3 tools/test_c_sdk_headers.py
python3 tools/qemu_test.py --build --markers-file tools/c_sdk_core_markers.json \
  --post-boot-delay 3 --settle 3 --cmd-timeout 90 --timeout 300 --boot-timeout 180
```

QEMU TCG passed all five required/four forbidden markers on 5 November 2026:
`[CSDK-CORE] file and errno OK`, `[CSDK-CORE] spawn and wait OK`,
`[CSDK-CORE] ok`, and parent exit code 0. The child exit status is asserted by
the parent, not inferred from a printed success line.

M7.10 remains open for the callback C interface over the existing Rust UI
runtime, generated GUI types, static-library packaging and native C GUI proof.
No second widget implementation or placeholder GUI API is introduced here.
