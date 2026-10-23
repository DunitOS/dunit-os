# System Lua and the DWM policy runner

Dunit ships official unmodified Lua 5.5.1 in three roles:

- `/system/bin/lua`: the ordinary standalone interpreter with all upstream
  libraries (base, coroutine, debug, io, math, os, package, string, table, utf8).
- `/system/bin/luac`: the upstream bytecode compiler.
- `/app/dwm_lua`: the separate resource-limited desktop policy process.

All three, plus the Lua compatibility probe, are compiled from C with the
Dunit **cross-TCC**, linked against native Dunit musl. No host Lua executable or
Linux compatibility ABI is substituted. TCC compiles the interpreter into a
static native ELF; Lua compiles scripts to Lua bytecode and executes the VM.

## Commands on Dunit

```
exec lua -v
exec lua -i
exec lua /path/to/program.lua
exec luac -o /tmp/program.luac /path/to/program.lua
exec lua /tmp/program.luac
exec tcc @/system/src/lua/build.rsp
exec /tmp/lua -v
```

Use `-i` for the REPL: Dunit's current libc does not implement terminal `ioctl`
recognition. The interpreter's automatic tty detection cannot be assumed.
`os.exit(0)` leaves the REPL. Native blocking stdio waits for input readiness,
including when launched through a terminal's PTY.

## Build and module configuration

`Makefile` owns `LUA_UNITS`/`LUA_FULL_UNITS`; both cross compilation and the
on-device response file derive from these same lists. `config/lua.h` owns
installation defaults. Upstream Lua's `LUA_PATH_5_5`/`LUA_PATH`,
`LUA_CPATH_5_5`/`LUA_CPATH`, and initialization environment variables retain their
standard behavior. `package.path` is also mutable from Lua.

The default Lua source-module locations are `/system/share/lua/5.5/` and the
current directory. They are installation defaults, configurable through the
header or standard runtime environment. No DWM script or theme path is compiled
into the system interpreter. GUI policy runner paths remain in `[scripts]` TOML.

`tools/pack_initrd.py` installs the executables, upstream C sources/headers and
Dunit defaults under `/system/src/lua/`. The generated `build.rsp` instructs the
actual on-device `/system/bin/tcc` to compile/link them to `/tmp/lua`.
It avoids command-line length limits without a custom compiler or shell shim.

## Native libc fixes required by full Lua

The musl working tree adds shared native I/O adaptation in
`src/internal/dunit_io.h`: bounded transfers, correct short-I/O handling by stdio
and writev, and cooperative waiting on asynchronous read readiness. `remove`
uses the existing native path-length adapters. Without these, `fgets` ended on
EAGAIN, `os.remove` used Linux-shaped arguments, and TCC's single large fwrite
silently left a corrupt output executable. No kernel ABI change was required.

The transfer granularity is a technical memory-safety invariant, not a user
policy setting. The Dunit syscall itself remains unchanged. This work does not
add nonblocking-FD mode support to the libc port.

## Validation

`tools/lua_system_markers.json` checks standard libraries, coroutine resume/yield,
UTF-8, string packing, dumping/loading Lua bytecode, source-module require,
file create/write/read/close/remove, and the interactive REPL. It also compiles
all Lua sources with TCC **inside Dunit** and runs the same standard-library
regression using the resulting `/tmp/lua`. Marker waits are scoped per command,
including successful exit, so one invocation cannot satisfy another's proof.
`musl_io` verifies >200 KiB write/writev/fwrite/fread content and error handling.
The separate Lua manifest tests the TCC-built restricted DWM runner.

All OS execution uses tools/qemu_test.py. Do not run builds/QEMU concurrently.

## Platform limits

Full upstream language and library tables do not create OS facilities missing
from Dunit: dynamic C module loading, POSIX signals, shell process execution and
other unsupported libc calls retain their documented errors. Lua file modules,
ordinary file I/O and bytecode work. The full interpreter is a normal userspace
program, not the restricted DWM execution environment. The DWM runner intentionally
exposes a smaller API and enforces independent budgets/deadlines.
