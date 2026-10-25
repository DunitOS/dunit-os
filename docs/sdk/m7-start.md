# M7 SDK foundation

## Accepted runtime

On 26 October 2026 the project owner selected option A from M7.3: a native
`no_std + alloc` SDK over `libdunit` and the existing runtime crates. Porting
Rust `std` over musl is not the current M7 runtime. The choice does not change
the kernel/userspace boundary, privileges or native ABI.

## First implementation: shared live GUI control channel

`protocols/gui-v1/src/control.rs` owns the existing CAP1, INP1, RLD1 and NOT1
layouts. The compositor and all seven GUI clients use it to encode and decode
capability announcements, input, reload signals and notifications. Old wire
bytes and gui-v1 versions remain unchanged. Capability mapping permissions and
actual backing lengths still come from the kernel, not from the announced size.

This is groundwork for M7.8 and the future SDK event loop. It does not implement
a wait-set, `cargo dunit`, `Component`, live damage tracking or the full SDK.
Low-level ABI fixtures remain on `libdunit`; applications have not yet been
claimed as migrated to the new runtime.

Verified on 26 October 2026:

- All 68 protocol host tests passed, including seven new control-channel tests
  for exact bytes, negative coordinates, short packets, legacy quit/leave,
  bounded text and UTF-8.
- Target checks passed for the compositor and all seven GUI clients.
- QEMU `gui_smoke`, production DWM `settings_ui` and `dwm_engine_v2` passed with
  `Result: OK (completed)` and `Markers: OK (marker budget satisfied)`.
  These exercise surface creation, input, settings reload and desktop profiles.
- `tools/gen_abi.py --check` confirms that the native ABI remains unchanged.

The full M7 milestone remains open; these results establish only the shared
control-channel foundation.

## Proposed M7.1 contract — requires owner approval

The roadmap requires a human decision for new syscalls. This section is a
reviewable proposal; no syscall or generated ABI file is changed by it.

Propose one new `WaitSetWait` syscall with six arguments: interests pointer,
interest count, output pointer, output capacity, relative timeout in monotonic
nanoseconds, and flags (initially zero). Allocate its number only after fetching
and checking the current ABI manifest. Existing calls remain compatible.

Each call snapshots and registers up to 64 interests. Proposed fixed-width
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
buffers. Require output capacity at least equal to interest count so a ready
result cannot be silently dropped.

Registration and sleep must be atomic with respect to readiness changes. Closing
a watched descriptor must wake with hangup/error; reusing its descriptor number
must not silently switch the watch to another object. Keep output copy faults
recoverable and consume no underlying IPC, file data or child exit status.

The initial primitive would be level-triggered and scoped to one blocking call.
Applications cancel a wait by sending a normal message to their own registered
IPC inbox using the existing IPC API. Persistent registration and native edge
tracking remain a separate design step; this proposal does not claim that the
entire M7.1 acceptance gate is complete.

After approval, implementation and tests must cover simultaneous pipe/IPC/timer
interests, opaque token preservation, child termination, timeout, hangup,
concurrent close/reuse, cancellation and lost-wakeup races. Then connect the
primitive to the actual SDK event loop rather than adding an unused syscall.
