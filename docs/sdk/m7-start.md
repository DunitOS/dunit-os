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
`cargo dunit`, `Component`, live damage tracking or the full SDK; the initial
wait-set and native event loop are described below.
Low-level ABI fixtures remain on `libdunit`. The later monitor integration uses
the event-loop layer only; its GUI has not been migrated to Component.

Verified on 26 October 2026:

- All 68 protocol host tests passed, including seven new control-channel tests
  for exact bytes, negative coordinates, short packets, legacy quit/leave,
  bounded text and UTF-8.
- Target checks passed for the compositor and all seven GUI clients.
- QEMU `gui_smoke`, production DWM `settings_ui` and `dwm_engine_v2` passed with
  `Result: OK (completed)` and `Markers: OK (marker budget satisfied)`.
  These exercise surface creation, input, settings reload and desktop profiles.
- The shared control-channel change preserved the native ABI; the separately
  approved wait-set addition below extends it compatibly.

The full M7 milestone remains open; these results establish only the shared
control-channel foundation.

## Accepted M7.1 contract — initial level-triggered primitive

The project owner approved this contract on 26 October 2026. `WaitSetWait`
is syscall 89, allocated after fetching and checking the ABI manifest.

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

## Implementation and integration

The fixed-width Rust records live once in `abi/wait-set`; the kernel and
`libdunit::wait_set` depend on that crate. `runtime/dunit` starts the native SDK
with an owning `event::EventLoop`; `gui_stat` uses it for inbox readiness and
refresh deadlines. This does not migrate the monitor's rendering to Component.

Kernel checks and registration are atomic under the existing UP IRQ guard.
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

## Event bits and limits

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

## Reproducing the current checks

```sh
python3 tools/gen_abi.py --check
cargo test --manifest-path abi/wait-set/Cargo.toml
cargo test --manifest-path protocols/gui-v1/Cargo.toml
python3 tools/qemu_test.py --build --markers-file tools/waitset_markers.json \
  --post-boot-delay 3 --settle 4 --boot-timeout 180 --cmd-timeout 130 --timeout 300
python3 tools/qemu_test.py --markers-file tools/sdk_event_loop_markers.json \
  --post-boot-delay 3 --settle 1.5 --boot-timeout 180 --cmd-timeout 100 --timeout 300
```

The latter two contracts also run with `--accel tcg`, as in CI. The monitor
fixture explicitly declares its application registry and uses system defaults
for the remaining desktop settings. Successful refresh requires an actual
wait timeout and advancing uptime; successful close requires decoding QUIT.

## Verified wait-set and SDK results (26 October 2026)

- The ABI record tests passed (2/2); the existing GUI protocol suite passed
  (68/68). `gen_abi.py --check` reports 89 syscalls with generated outputs and
  kernel definitions in sync.
- `waitset_markers.json` passed on KVM and TCG, including a pending output
  buffer made read-only by a second thread: EFAULT leaves the IPC message
  readable. Multiple ready records preserve registration order.
- `sdk_event_loop_markers.json` passed on KVM and TCG: System Monitor rendered,
  waited for a timed refresh, observed advancing uptime and decoded QUIT.
- The full existing `boot_markers.json` regression passed: runtime stress,
  child lifecycle, IPC and descriptor cleanup remain green.
- Compiler warning budget passed at 13/99, unchanged from before this addition.

Every QEMU run above completed with `Result: OK (completed)` and
`Markers: OK (marker budget satisfied)`. The code still targets the existing
UP kernel. Producer notifications may rescan unrelated wait-sets; they do not
reset deadlines or require a userspace polling loop. Persistent registrations,
native edge delivery, sockets and Component remain separate unfinished M7 work.
The later native test and CLI/TUI tooling slice is described below.

## Next implementation: native application APIs

`#[dunit::main]`, fs/io/process/time/args/env/system wrappers and the `calc`
migration are described in [Native application API](native-api.md). The native
ABI remains unchanged in this step; no new application-manifest format or
privilege mechanism is introduced. Thread/sync remains open.

## Accepted application manifest and native tooling

On 26 October 2026 the owner approved [dunit.toml v1](app-manifest-proposal.md).
The [cargo dunit workflow](cargo-dunit.md) implements native CLI/TUI/GUI generation,
Cargo artifact selection, image overlays and guest test discovery through
`#[dunit::test]`. No new syscall or archive format is needed.
[GUI launcher/dock manifest discovery](desktop-catalog.md) is implemented.
The [native window helper and GUI template](window.md) are implemented.
[Typed root components and end-to-end damage](components.md) are implemented and
used by System Monitor. Partial upload/composition/presentation uses conservative
full-frame fallbacks for scene changes and effects. Widget behaviour/state, other
legacy-client migrations and CLI/TUI desktop launching remain open.
