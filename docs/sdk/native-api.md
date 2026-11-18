# Native Rust application API

The SDK lives in `runtime/dunit`. It uses `no_std + alloc` and the native
`libdunit` ABI. It is not a Rust std port. Manifest/tooling and GUI manuals describe the implemented application path.
[Native threads and synchronization](threads.md) provide owning workers, mutexes
and condition variables. The [standalone distribution](standalone.md) packages
these same APIs for offline external builds and uses the existing OS checkout
harness for native execution; native GUI frame/resource acceptance is verified.

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
| `pty` | Owned PTY master, explicit argv/env child spawning and bounded pending input |
| `canvas` (gui) | Private canvas snapshots and cached configured fonts |
| `window`, `ui` (gui) | Owned native windows, typed controls and shared transfer integration |
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
a wait-set and inherits the current monotonic clock granularity. `Instant::deadline(token)`
creates a timer interest. Durations that cannot be represented as a finite
native nanosecond timeout return EINVAL rather than becoming infinite waits.

`Child::spawn` uses existing native program lookup and SpawnProcess semantics.
`Child::spawn` does not accept custom argv/env or file actions. Waiting observes
child completion with WaitSetWait, then consumes the actual native wait status
once. Further waits return the cached status. `try_wait` and `wait_timeout` do
not reap a child that is still running; a timed-out child remains owned.
Negative exit codes and faults are distinguished by `ExitStatus`.
Drop does not kill or reap a child; finish its lifecycle explicitly.
`terminate_and_wait` handles a live child and a natural-exit race. `pty::Pty`
spawns with distinct argv/env entries on an owned PTY master; terminal adapters
retain final child output and reap completion once. `PendingInput` preserves
short-write suffixes within its queue and per-service budgets.

## Wait-set

The project owner approved this contract on 26 October 2026. `WaitSetWait`
is syscall 89 in the native ABI manifest.

The `WaitSetWait` syscall has six arguments: interests pointer,
interest count, output pointer, output capacity, relative timeout in monotonic
nanoseconds, and flags (zero). Existing calls remain compatible.

Each call snapshots and registers up to 64 interests. Fixed-width
little-endian records for x86_64:

```text
WaitInterest (32 bytes):
    kind: u32
    interest: u32
    object: u64
    token: u64
    deadline_ns: u64

WaitResult (24 bytes):
    token: u64
    events: u32
    reserved: u32
    data: u64
```

Interest classes:

- IPC: the calling process's existing inbox, without consuming messages.
  `object` is zero; this does not grant access to another process's inbox.
- Descriptor: existing file, pipe or PTY read/write readiness, with error and
  hangup reported independently of requested read/write bits.
- Timer: an absolute monotonic deadline carried in the interest record.
- Process: termination of an existing child process, without reaping it or
  expanding the permissions of the current wait API.

Tokens are caller-supplied opaque identifiers. Return the number of results,
zero on timeout, or an existing negative errno. Zero timeout checks immediately;
an infinite timeout is represented explicitly by the maximum u64 value. Reject
unknown flags/classes, invalid descriptors, non-child processes and invalid
buffers. Require output capacity between interest count and 64 so a ready result cannot
be silently dropped.

Registration and sleep must be atomic with respect to readiness changes. Closing
a watched descriptor must wake with hangup/error; reusing its descriptor number
must not silently switch the watch to another object. Keep output copy faults
recoverable and consume no underlying IPC, file data or child exit status.

The initial primitive is level-triggered and scoped to one blocking call.
Applications cancel a wait by sending a normal message to their own registered
IPC inbox using the existing IPC API. Persistent registration and native edge
tracking remain a separate design step; this implementation does not claim that the
entire M7.1 acceptance gate is complete.

### Implementation and integration

The fixed-width Rust records live once in `abi/wait-set`; the kernel and
`libdunit::wait_set` depend on that crate. `runtime/dunit` starts the native SDK
with an owning `event::EventLoop`; `gui_stat` uses it for inbox readiness and
refresh deadlines. The monitor also uses the typed `ComponentWindow` for rendering.

Kernel checks and wait registration are atomic with readiness changes.
The thread owns a kernel snapshot across syscall restarts: spurious producer
notifications cannot extend a relative timeout, mutate the input interests or
retarget a closed descriptor. Slot binding identities change on dup/reuse.
Pending snapshots are released on completion, copy fault or thread teardown.
Readiness delivery does not retain pipe-end ownership.

`tools/waitset_markers.json` exercises simultaneous pipe/IPC/timer interests,
opaque tokens, repeated level readiness, registration races, IPC cancellation,
spurious wakes, timeout, peer hangup, close/reuse, child exit without reaping,
PTY readiness, permissions and malformed pointers. CI runs the same contract
with TCG before publishing either release image.

### Event bits and limits

Kinds are IPC=1, DESCRIPTOR=2, TIMER=3, PROCESS=4. Event bits are READ=1,
WRITE=2, HANGUP=4, ERROR=8, ELAPSED=16 and EXITED=32. Descriptor subscriptions
accept READ/WRITE (or zero for hangup only); IPC uses READ, timers ELAPSED and
processes EXITED. Irrelevant fields must be zero. Results preserve registration
order, including duplicate tokens. `data` is zero except for EXITED, where its
upper u32 is the existing wait kind and lower u32 is the signed exit code.

An initially invalid fd returns EBADF; an initially non-child pid returns
ECHILD. Closure/reuse after registration returns HANGUP|ERROR on the original
token; a sibling reaping a registered child has the same closure result.
Unknown classes/bits, flags, excessive counts and insufficient capacity return
EINVAL. Bad mappings return EFAULT, including output mappings made read-only
while blocked. No event is consumed on a copy fault.

## Verification

Host contracts: `runtime/dunit/tests/streams.rs`, `entry_data.rs`,
`runtime/dunit-macros`, and `abi/wait-set` record tests. Native contracts:
`tools/sdk_api_markers.json`, `sdk_entry_panic_markers.json`,
`waitset_markers.json`, `sdk_event_loop_markers.json` and `dsh_calc_markers.json`.
The API probe checks a 200,003-byte binary file roundtrip. Empty/error/panic
suites require real failure exits 1/1/101; successful output cannot override them.
All guest execution uses `tools/qemu_test.py`.
