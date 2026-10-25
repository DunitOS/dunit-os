# Native Rust application API (M7, early)

The SDK lives in `runtime/dunit`. It uses `no_std + alloc` and the native
`libdunit` ABI. It is not a Rust std port. Application manifests, `cargo dunit`,
thread/sync wrappers and declarative GUI components are still unfinished.

## Entry point

A native application in `userspace/system_apps/<name>` depends on:

```toml
[dependencies]
dunit = { path = "../../../runtime/dunit" }
```

Its source can be an ordinary safe function:

```rust
#![no_std]
#![no_main]

#[dunit::main]
fn main() -> dunit::Result<()> {
    let text = dunit::fs::read_to_string("/assets/exec-path")?;
    dunit::println!("{}", text)?;
    Ok(())
}
```

`#[dunit::main]` accepts one safe synchronous function without parameters or
generics, returning `dunit::Result<()>`. It supplies the C loader entry,
initializes argv/envp, and installs the panic handler. Ok exits 0; Err prints a
native errno diagnostic and exits 1; panic reports the failure and exits 101.
Entry diagnostics use a bounded UTF-8 buffer, no heap allocation and one
best-effort stderr write: a full/disconnected terminal cannot prevent exit.
Use one generated entry per binary. A dependency must be named `dunit`.

## Available modules

| Module | Current interface |
|---|---|
| `io` | `Read`, `Write`, `BufReader`, stdin/stdout/stderr, formatted output |
| `fs` | Owned `File`, `OpenOptions`, read/write helpers, metadata, seek, sync, rename/remove, directories and cwd |
| `args` | Owned UTF-8 argument strings from the initialized loader stack |
| `env` | Owned optional environment value; invalid names return EINVAL |
| `time` | Monotonic `Instant`, `Duration`, checked deadlines and blocking sleep |
| `process` | `Child::spawn`, id, kill, try/wait/timeout, cached exit status |
| `system` | Checked access to the existing kernel's `SystemStats` facts |
| `event` | Owning event loop over the native heterogeneous wait-set |
| `errno` | Re-export of generated native errno definitions; no copied table |

All fallible operations use `dunit::Result<T>`. `Error::raw_os_error()` is the
negative Dunit errno, and Display/name return its existing ABI name.

## Streams and files

`Read::read_exact` completes short reads; premature EOF is EIO.
`Write::write_all` completes short writes; zero progress is EIO. Formatting
propagates the original I/O error. `print!`/`println!` return a Result, so use `?`
or handle failure explicitly.

Keep a `BufReader` alive across calls to `read_line`. It preserves subsequent
lines that arrive in the same underlying read, handles split UTF-8 sequences
and long lines, and returns None at clean EOF. A returned line excludes LF;
a final unterminated line is returned once. Invalid UTF-8 returns EINVAL.
Stdin and stdout/stderr wait for readiness through WaitSetWait on EAGAIN;
they do not implement a yield/retry polling loop.

File ownership is unique. Drop closes its descriptor once; explicit `close`
reports native close errors. `sync_all` requests the native filesystem flush.
Helpers do not silently truncate reads or discard partial write errors. Open
options reject missing access modes and creation/truncation without write
access. The existing kernel's path, permission and filesystem limits remain in
effect; `/persist` is DunitFS while the root is MemFS.

## Time and children

`Instant::now` uses the monotonic kernel clock. `sleep(Duration)` blocks through
a wait-set and inherits PIT clock granularity. `Instant::deadline(token)`
creates a timer interest. Durations that cannot be represented as a finite
native nanosecond timeout return EINVAL rather than becoming infinite waits.

`Child::spawn` uses existing native program lookup and SpawnProcess semantics.
This first API does not accept custom argv/env or file actions. Waiting observes
child completion with WaitSetWait, then consumes the actual native wait status
once. Further waits return the cached status. `try_wait` and `wait_timeout` do
not reap a child that is still running; a timed-out child remains owned.
Negative exit codes and faults are distinguished by `ExitStatus`.
Drop does not kill or reap a child; finish its lifecycle explicitly.

## Migration and tests

`calc` now uses the generated entry, SDK formatted output and buffered stdin.
It has no manual ABI/entry/panic boilerplate and no direct libdunit dependency.
The existing `dsh_calc_test` drives its cooked PTY interaction and arithmetic.
`gui_stat` continues using the SDK event loop; its rendering is still immediate.

Host tests exercise generic stream behavior, initialized entry data and entry
macro validation. Native allocation lives in libdunit's target-only allocator
module; host tests use the host allocator and do not execute native syscalls.
There is no host OS backend for the application APIs.

```sh
cargo test --manifest-path runtime/dunit-macros/Cargo.toml
cargo test --manifest-path runtime/dunit/Cargo.toml
python3 tools/qemu_test.py --build --markers-file tools/sdk_api_markers.json \
  --post-boot-delay 3 --settle 4 --boot-timeout 180 --cmd-timeout 120 --timeout 360
python3 tools/qemu_test.py --markers-file tools/sdk_entry_panic_markers.json \
  --post-boot-delay 3 --settle 4 --boot-timeout 180 --cmd-timeout 60 --timeout 240
```

CI runs these native contracts with TCG before publishing the two release ISOs.
The full M7 gates remain open, including `#[dunit::test]`, application manifests,
cargo tooling, thread/sync APIs, Component/widgets, live damage and clipboard/DnD.

Verified on 26 October 2026: 10 SDK host tests and 2 macro tests passed;
`tools/gen_abi.py --check` still reports 89 syscalls. The application API and
panic contracts passed on both KVM and TCG, including the unchanged cooked
PTY calculator test. The existing boot/runtime-stress/IPC marker budget and
the SDK monitor's refresh/close regression also passed. All completed QEMU
runs reported `Result: OK (completed)` and `Markers: OK (marker budget satisfied)`.
The compiler warning budget remained 13/99.
