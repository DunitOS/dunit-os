# Lua runtime: compatibility and system integration

Status: static compilation and Dunit QEMU compatibility, runner regression and compositor integration passed. See `tools/lua_markers.json` and `tools/lua_desktop_markers.json`.

See [system Lua](lua-system.md) for the full interpreter, bytecode compiler,
TCC build and on-device source rebuild.

## Reference implementation

Use the official, unmodified Lua **5.5.1** source release (2026-07-24), downloaded
from <https://www.lua.org/ftp/lua-5.5.1.tar.gz>. SHA-256:

```
1c4b4068d67061f2a2231ad2b5422e77acea1487ea9890f6320af614f4373dce
```

The official unmodified source is vendored in `third_party/lua/src`, with
provenance and pinned archive hash in `third_party/lua/UPSTREAM.md`, its upstream
README alongside, and the MIT copyright/license block retained in `src/lua.h`.
See the official [download page](https://www.lua.org/download.html) and
[Lua 5.5 manual](https://www.lua.org/manual/5.5/manual.html).
Updating the version requires rerunning the compatibility
gate. The harness uses the Lua 5.5 `lua_newstate(allocator, userdata, seed)` API;
it is intentionally not source-compatible with the two-argument Lua 5.4 API.

## Scope and execution model

`userspace/ctests/lua_spike.c` embeds the official VM and parser using the C API.
It tests arithmetic (integer division and floating exponentiation), table
construction/update, protected errors with `pcall`, instruction-hook exhaustion,
allocator exhaustion, and complete allocator release after `lua_close`.
Each probe has a fresh state and a 256 KiB budget for Lua-managed allocations,
including allocator headers. The surrounding C process, native stack and musl
allocator bookkeeping are outside that budget. An explicit seed makes this probe
reproducible; production must supply an unpredictable seed for hash hardening.

Base, math, string and table libraries are registered. Math random/randomseed are removed. `os`, `io`, `package`, `debug` and coroutine
libraries are never registered; `load`, `loadfile`, `dofile`, and `collectgarbage`
are removed. Source enters through `luaL_loadbufferx(..., "t")`, refusing binary
chunks. The spike's scripts are fixed test data, not a shipped script loader.
No Dunit syscall, filesystem, process, network, GUI, or capability API is exposed
to Lua. No language imitation is used.

The count hook raises an ordinary Lua error after approximately 20,000 VM
instructions, checked every 1,000 instructions. **This is a compatibility probe,
not a hostile-script isolation mechanism.** A script can catch hook errors with
`pcall`; hooks also do not preempt long-running C functions or source compilation.
Before accepting untrusted scripts, run the VM in a separate Dunit process with
external time/memory enforcement and explicit host-call capabilities. Do not
claim that this hook supplies an uncatchable CPU limit. Keep production host
functions bounded and avoid permitting coroutine creation without installing
equivalent hooks on every thread.

## Static compilation

The normal `make userspace` recipe builds the compatibility probe and DWM runner
with Dunit cross-TCC (`-std=c11`) and the pinned native musl sysroot. The explicit
`LUA_UNITS` list in Makefile is the source of truth. The full standalone runtime
uses the additional standard-library units and upstream lua.c/luac.c.

For the compatibility probe and restricted DWM runner, do not enable
`LUA_USE_LINUX`, `LUA_USE_POSIX`, dlopen, readline, or the standalone `lua.c`
frontend. Their explicit units exclude `liolib.c`, `loslib.c`, `loadlib.c`,
`ldblib.c`, `lcorolib.c`, and `linit.c`. No host `-lm`/`-ldl` is needed: the Dunit
musl archive contains its math functions. Auxiliary/base-library object code
still references standard stdio helpers, even though the file-loading globals
are removed; linking them does not expose them as Lua APIs.

`Makefile` compiles `lua_spike` and `dwm_lua` through the pinned musl
sysroot with its explicit `LUA_SOURCES` list and packages the resulting ELFs
through the standard userspace/initrd path. `userspace/lua_runtime/limits.h`
is shared by the compatibility probe and runner so allocator/hook behavior has
one implementation. See [the runner contract](dwm-lua-runner.md) for the
one-shot process, validated rectangles, animation policy, and CLI/environment
invocation. The runner exposes fewer base functions than the acceptance probe;
protected error handling remains tested in the latter.

## Pinned musl compatibility assessment

The pinned `toolchains/dunit-musl` supplies `malloc/realloc/free`, memcpy/strings,
strtod/numeric formatting, locale helpers, ISO C math, and x86-64
`setjmp/longjmp`. These are the relevant Lua dependencies. Lua errors exercise
the latter directly. This single-threaded generic-C profile does not require
signals, fork/exec, networking, dynamic loading, wall-clock time, pthreads, poll,
or directory iteration. `lua_newstate` takes an explicit seed, avoiding
`luaL_newstate`'s default time-based seed generation on Dunit's uptime-only
clock. Actual static linking and execution remain the authoritative proof.

Possible blockers to check are unresolved libc symbols, compiler assumptions
in floating math/locale, musl allocator behavior under the configured VM budget,
and correct native exception unwinding through setjmp/longjmp. Do not replace
failures with fake successful libc stubs.

## QEMU gate (passed)

The build coordinator must package the resulting ELF through the normal initrd
path and run it via `tools/qemu_test.py`, never a separate QEMU launcher. The executable manifest is `tools/lua_markers.json`; it requires all of:

```
[TERM-007] Starting main loop
[LUA] language OK
[LUA] instruction-budget OK
[LUA] allocation-budget OK
[LUA] compatibility OK
```

Forbid `[PANIC]` and `[LUA] FAIL`; require normal process exit status zero.
The compatibility and runner gates passed in Dunit QEMU.
This gate supports choosing the real upstream Lua runtime for integration; it
does not establish a production sandbox, application API, editor, file format,
or security policy.

The same manifest runs an explicit `dwm_lua` CLI request and
`userspace/ctests/dwm_lua_test.c`. The C regression parent launches the actual runner through native PTY
create/spawn/read/close calls from the generated ABI, compares the complete
successful policy output, and checks WIFEXITED/WEXITSTATUS for each request.
Pinned musl has no usable pipe/file-action adaptation, so the test does not
assume that newer fork features exist. Child polling is bounded; failure or
timeout kills and boundedly reaps the child before closing the PTY. Success coverage is
columns, monocle, zero windows, and environment invocation. Negative coverage
is invalid geometry, missing rectangles, instruction exhaustion, allocator
exhaustion, syntax failure, and invalid arguments. Every failure must have the expected nonzero exit status, an actual diagnostic,
and no policy header/rectangle in the captured merged stdout/stderr. The test
does not claim that a merged PTY can distinguish the two output streams. This avoids global serial markers
mistaking one child's failure status for all failed cases.

Lua fixtures live under `assets/dwm/scripts/tests/`; the existing recursive
asset packer ships them as `/assets/dwm/scripts/tests/`. They are inert scripts
and are never automatically evaluated by the compositor. Their explicit use is
limited to the regression parent. All QEMU execution must still go through
`tools/qemu_test.py --build --markers-file tools/lua_markers.json` with adequate
boot/command timeouts. A passing manifest is required before reporting the Lua
runtime integration as tested.

The runner reads source in at most 4096-byte requests, with a final extra-byte
check after 64 KiB; no read exceeds the kernel MAX_USER_COPY limit.
