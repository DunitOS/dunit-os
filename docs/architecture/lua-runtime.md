# Lua runtime

Dunit ships official unmodified Lua 5.5.1 (24 July 2026) from
[the upstream release](https://www.lua.org/ftp/lua-5.5.1.tar.gz), SHA-256
`1c4b4068d67061f2a2231ad2b5422e77acea1487ea9890f6320af614f4373dce`.
Sources and the original MIT license remain in `third_party/lua/src`; provenance
is in [UPSTREAM.md](../../third_party/lua/UPSTREAM.md). Version changes require
fresh native compatibility tests. Lua 5.5's seeded three-argument lua_newstate
API differs from Lua 5.4.

## Programs and commands

`/system/bin/lua` is the ordinary interpreter with all upstream libraries;
`/system/bin/luac` compiles bytecode. `/app/dwm_lua` is a separate restricted
[desktop worker](desktop.md#lua-layout). All use cross-TCC and native Dunit musl,
producing static native ELF, not host Lua or a Linux ABI replacement.

```text
exec lua -v
exec lua -i
exec lua /path/program.lua
exec luac -o /tmp/program.luac /path/program.lua
exec lua /tmp/program.luac
exec tcc @/system/src/lua/build.rsp
exec /tmp/lua -v
```

Use `-i` to select the REPL explicitly; `os.exit(0)` leaves it. Native libc
recognizes attached PTY descriptors through its ioctl subset and blocking stdio
waits for readiness. No full termios line discipline is implied.

## Build and configuration

Makefile LUA_UNITS/LUA_FULL_UNITS own the source lists for host cross compilation
and on-device response files. `config/lua.h` owns installation defaults.
`tools/pack_initrd.py` installs upstream sources/headers and build.rsp under
`/system/src/lua`; the response file builds `/tmp/lua` with real on-device TCC.
Default source modules live in `/system/share/lua/5.5/` and the current directory.
Upstream LUA_PATH_5_5/LUA_PATH, LUA_CPATH_5_5/LUA_CPATH and init variables keep
standard behavior; package.path remains mutable. Desktop script paths are TOML
policy and are not compiled into the interpreter.

Full Lua includes io/os/package/debug/coroutine/utf8. Dynamic C modules,
unsupported libc calls and unavailable OS facilities return documented errors.
Ordinary file modules, file I/O and bytecode are supported. Do not interpret an
upstream library table as proof of popen/dlopen/signals/networking.

Native musl `dunit_io.h` bounds transfers and handles short I/O and EAGAIN waits;
remove uses native path adapters. These prevent truncated TCC output and broken
fgets. Current fd flags, pipes and spawn actions are documented in the
[native ABI](../../abi/README.md#descriptors-and-spawn). No host math/dl library
is substituted; musl supplies math, locale, allocator and setjmp/longjmp.

## Compatibility probe and isolation

`userspace/ctests/lua_spike.c` creates a fresh state per probe with a 256 KiB
Lua-managed allocation quota, including its allocation headers. C process,
stack and musl bookkeeping lie outside the quota. It tests arithmetic/tables,
protected errors, instruction exhaustion, allocator exhaustion and full release
at lua_close. Its deterministic seed is test data; secure production entropy
is still absent.

The probe registers base/math/string/table; random/randomseed and file-loading,
load and collectgarbage globals are removed. It loads source-only buffers and
exposes no Dunit filesystem/process/GUI/capability host API. The restricted worker
uses separate production limits and exposes fewer functions than this probe.

A count hook checks every 1000 instructions and raises after about 20000 in the
probe. Lua pcall can catch ordinary hook errors; compilation and long C calls
are not preempted. Hooks/heap quotas alone are not hostile-script isolation.
Desktop workers run in separate processes with external monotonic deadlines,
bounded host functions and validated output. Never move synchronous Lua execution
into the compositor or assume a Lua allocator quota limits all process memory.

## Verification

`tools/lua_markers.json` exercises the language probe and actual PTY-launched
worker: columns, monocle, no windows, environment invocation, bad geometry,
missing rectangles, instruction/memory exhaustion, syntax and argument errors.
The parent compares complete output and each child's exit; failures emit a
nonzero status/diagnostic and no partial policy. A timeout kills and reaps the
worker before PTY close. A merged PTY cannot distinguish stdout from stderr.
Inert fixtures live in `/assets/dwm/scripts/tests/` and are explicitly selected.
The runner reads chunks no larger than 4096 bytes with a final overflow check
at the 64 KiB source bound.

`tools/lua_system_markers.json` checks standard libraries, coroutine/UTF-8,
string packing, bytecode, require, file roundtrips, REPL and a complete TCC rebuild
inside Dunit followed by the same library regression. `musl_io` checks more than
200 KiB across write/writev/fwrite/fread and failures. Desktop policy/fallback
uses separate Lua desktop manifests. All native verification uses qemu_test.py;
source inspection and historical passed runs are not a fresh execution.
