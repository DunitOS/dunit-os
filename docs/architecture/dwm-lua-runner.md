# DWM Lua: one-shot policy runner

`userspace/system_apps/dwm_lua/main.c` is an ordinary static userspace program
embedding the official vendored Lua 5.5.1 core. It computes policy snapshots;
the compositor remains the owner of windows, surfaces, focus, capabilities,
animation state, and protocol sequencing. No kernel ABI or gui-v1 change is
required. The static runner and compositor integration passed the CLI and desktop QEMU gates (`tools/lua_markers.json` and `tools/lua_desktop_markers.json`).

## Invocation and source contract

Explicit CLI form:

```
dwm_lua /assets/dwm/scripts/columns.lua 1200 800 3
```

Without arguments the same request comes from `DWM_LUA_SCRIPT`,
`DWM_LUA_WIDTH`, `DWM_LUA_HEIGHT`, and `DWM_LUA_WINDOWS`. Missing/invalid
parameters fail with status 2. The environment form works with existing
`pty_spawn_env` without adding an argv transport. The runner reads only the
specified source file, up to 64 KiB; script code does not receive file access.

The parent passes dimensions of the usable work area, excluding its own
reserved panel/dock margins. Lua sees `workarea = {width, height, count}`;
rectangles are relative to work-area origin `(0,0)`. The parent maintains a
stable ordered list of its current target windows, mapping index 1 to the first
window in that snapshot. Lua receives neither PID nor protocol handles.
Width/height must be 1..16384 and window count 0..64. The count is a defensive
runner bound, not desktop policy; the parent still enforces configured max_windows.

The chunk must return this exact shape, with no extra record fields:

```lua
return {
    rectangles = {{x = 0, y = 0, w = 1200, h = 800}},
    animation = {duration_ms = 140, easing = 'ease_out'},
}
```

There must be exactly `count` rectangle entries at contiguous integer keys
1..count. All coordinates/sizes must be Lua integers; x/y are nonnegative,
w/h positive, and every rectangle lies wholly inside the work area. Overlap is
allowed for monocle/floating policies. Duration is 0..2000 ms; easing is
`linear` or `ease_out`. A zero-window snapshot returns an empty rectangle table.
Malformed data causes failure before any stdout output. The examples
`assets/dwm/scripts/columns.lua` and `monocle.lua` exercise two real Lua policies.

## Output contract (application data, version 1)

```
DWM_LUA 1 1200 800 3
RECT 1 0 0 400 800
RECT 2 400 0 400 800
RECT 3 800 0 400 800
ANIMATION 140 ease_out
END
```

Output is decimal ASCII and literal tokens. Header is version, work-area width,
height, count; each RECT contains index,x,y,w,h. For zero windows the header is
immediately followed by ANIMATION and END. stdout is reserved for validated
records. Diagnostics go to stderr with `dwm_lua:` prefix. Exit 0 means a complete
flushed result; 1 means evaluation/validation/allocation/output failure; 2 means
invalid invocation or source read failure. The consumer must buffer the whole
result, require END and exit 0, verify header against its original request,
revalidate all fields and indices, reject extra records, then apply once.
PTY transport may convert LF to CRLF and combine stderr/stdout; the consumer
must tolerate CRLF and reject diagnostic/incomplete results.

The runner validates before emitting its first record; a later transport error
can still leave partial output, so parsing RECT lines incrementally into live
desktop state would be incorrect. Stale results must be discarded if work-area
geometry or the ordered target-window snapshot changes during evaluation.

## Limits and privilege boundary

Selected base functions and the pure math/string/table libraries are available; os/io/package/debug/coroutine libraries are not opened. Math random/randomseed are removed; no entropy guarantee is implied. File loaders, runtime `load`, print/warn,
metatable/raw operations, explicit GC controls, pcall and xpcall are removed.
The parent source loader accepts text chunks only. Arithmetic, Lua tables,
functions, loops, assert/error and iteration remain the real upstream language.
This limited surface does not expose a Dunit API or construct a fake language.

The shared `userspace/lua_runtime/limits.h` limits tracked Lua allocations to
1 MiB including aligned allocation headers, and supplies a count hook with
100,000 VM instructions checked every 1,000 instructions. Lua state allocation
and parsing are included in the memory budget. Source storage, native stack,
and libc allocator overhead are outside it. The deterministic hash seed is for
this trusted-policy prototype; hostile-input deployment needs a suitable seed.

**This is not a sandbox or hard CPU/memory enforcement.** Count hooks do not
interrupt compilation or arbitrary C library calls, and process resources and
inherited capabilities remain outside Lua's allocator. The parent enforces an external deadline using the monotonic clock and kill/wait; locally trusted policy is still required because inherited process authority is unchanged. The VM never runs synchronously on a compositor thread.
Removing pcall reduces one quota-evasion surface; it does not prove isolation.

## Existing compositor integration points

`[scripts]` is part of shared Settings and its TOML serializer:

```toml
[scripts]
enabled = false
runner = "/app/dwm_lua"
layout = "/assets/dwm/scripts/columns.lua"
timeout_ms = 500
```

Terminal setup and the graphical Window behavior page edit this same model.
`gui_server/src/lua_policy.rs` starts a child through PTY, bounds response storage
to 16 KiB, drains at most four 1 KiB chunks per frame, checks its exit status,
revalidates the full result, and kills/reaps on timeout or failure. The monotonic
clock's reported frequency converts the configured 50..5000 ms deadline.

The compositor recomputes when active workspace, participating windows, usable
area or configuration changes. Maximized/minimized windows are excluded. A
changed snapshot cancels stale work. All cells must fit window decorations
before any cell is applied. Existing CONFIGURE/ACK/import/commit operations
resize the clients; `Lua geometry committed` is emitted only when actual client
buffer dimensions match the requested cells. Failures retain current geometry.

The compositor applies bounded duration/easing to its own fade/menu animation
state and interpolates window positions. Progress uses elapsed monotonic
milliseconds, including dock hover; dropped frames cannot extend a requested
transition. If the clock is unavailable, transitions finish immediately instead
of freezing. Buffer resizing remains a protocol transaction, not a pixel-interpolated resize. Dragging detaches a window from Lua
placement until settings reload, so script output cannot fight active dragging.
Floating restore geometry and detachment survive display-session re-entry.
Disabling Lua restores tiled windows to their remembered floating geometry.

The runner supplies neither per-frame scripting nor privileged configuration
rewrites. Scripts should run when the layout snapshot changes, not every frame.
This avoids making interpreter latency part of compositor rendering.

## Build and validation gates

The sole application C unit is `userspace/system_apps/dwm_lua/main.c`; link it
against the same explicit Lua core + lauxlib + lbaselib + lmathlib + lstrlib + ltablib units as `lua_spike`, with
`-std=c11 -O2 -Wall -Wextra -Ithird_party/lua/src` through Dunit cross-TCC. No Linux/POSIX
Lua feature macros, dlopen, readline, external linker process or host libm are
needed. `limits.h` is an ordinary local header, not another C unit. Build/QEMU
must be serialized by the coordinator.

Required tests are columns 1200x800/3 (exact output above), monocle with multiple
windows, zero-window output, environment invocation, and repeatability. Reject
negative/oversized dimensions, invalid integers/count, missing source, source
over 64 KiB, binary chunks, malformed Lua, missing/extra/fractional/out-of-bounds
rectangles, unknown animation values, allocation exhaustion, and an infinite
Lua loop. Every failed case must exit nonzero and emit no policy stdout.
The input scripts in `assets/dwm/scripts/tests/` provide the
out-of-bounds, missing-rectangle, quota and allocation cases; run outside/missing
with count 1. Use the normal qemu_test.py harness for Dunit execution and check
process exit/result markers; host compilation alone is not an acceptance gate.

`tools/lua_markers.json` runs the compatibility probe, explicit CLI output,
and the `dwm_lua_test` C parent. The parent observes each child's real wait
status through native PTY transport, checks exact successful policy output, and
requires diagnostic output without policy records for rejected requests. This supplies per-case evidence even though the outer marker harness
matches global serial substrings. The test fixtures ship through the existing
recursive assets packer and are read from `/assets/dwm/scripts/tests/`.

The regression transport uses generated native PTY syscall constants because
pinned musl does not provide usable pipe/file-action redirection. All spawned
requests use the environment invocation; the separate manifest CLI command
exercises argv invocation. PTY output merges stderr/stdout, so failures are
validated as diagnostic plus absence of policy, not as provably empty stdout.
The parent bounds polling and kills/reaps children on timeout or transport
failure. Source loading uses 4096-byte chunks and one overflow byte to respect
kernel MAX_USER_COPY while preserving the 64 KiB script-size limit.
