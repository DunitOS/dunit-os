# Internal current-task scopes — OP3

13 November 2026. Completes removal of the private current Process/thread getters
left behind by the [public access boundary](process-access.md). This is a lifetime
and callback-order change; no process registry/task mutex is introduced.

## Implementation

`current_process`, `current_process_mut`, `current_thread` and `current_thread_mut`
previously returned static references into boxed tasks. These functions are now
removed. Internal FD allocation/snapshots, TLS and thread setup/cleanup, child
wait snapshots, current exit/fault requests, futex clear-child-tid and dispatch
caller backing selection all use non-escaping scopes. Owned values and Arc pins
leave the callback; Process and borrowed field references cannot leave it.

`current_access.rs` now owns lookup and the callback boundary instead of wrapping
static-reference getters. Current owner/thread mutation and explicit-PID
`with_process_mut` share one implementation. It preserves NoSuchProcess versus
ProcessNotPrepared, callback Result payloads and deferred fd wake notifications,
including mutation callbacks returning an error. The mutable table/Process borrow
ends before the readiness rescan. The common explicit-PID callback has an explicit
higher-ranked lifetime bound too.

Previously `Process::handle_signal` mutated the remote target through the registry
while holding a mutable borrow of the source task. It special-cased self-delivery
to avoid aliasing. Signal authorization now reads the source handle/right and
copies the target PID; only after that read scope ends does delivery mutate the
target. Both self and remote delivery use that same path under the original UP
IRQ boundary. Rights checks, saturating counters and errno mapping are preserved;
this changes neither the capability model nor any public ABI.

## Validation

```sh
python3 tools/test_process_access.py
python3 tools/test_user_word.py
python3 tools/test_process_resources.py --accel tcg --cpus 4
```

The actual access module is compiled with a minimal owned-record/IRQ fixture.
All old compile-negative checks remain: Process/field references cannot escape,
callbacks requiring a static Process argument are rejected, and owned values
compile. Added cases cover read-only current-thread scopes and explicit-PID
mutation escapes. Runtime checks distinguish owner from thread, preserve missing
and unprepared task errors, exercise deferred notifications on error, and test
self/remote signal delivery plus rights/type/missing-target errors. The updated
futex host test uses scoped thread callbacks, including clear-child-tid.

The original native handle assertions remain. A new independent child drains
7+3 signals to exactly 10; the parent waits/reaps it and verifies ENOENT on the
same still-open endpoint afterward. New required markers strengthen the original
handle contract. The first version of the new fixture incorrectly assumed wait
blocked and failed on EAGAIN. The fixture now retries with yield, preserving all
original signal/count/exit assertions. Native regression/image provenance is in
[the receipt](task-scopes.json).

Final validation uses one full ISO: strengthened handle fixture on 4 vCPUs,
15 positive resource scenes (77 required / 44 forbidden entries) and the separate
thread lifecycle fixture on 4 vCPUs, full boot/runtime fault/IPC on 1 vCPU
(46 / 9), GUI smoke on 4 vCPUs (30 / 13). All final budgets passed. Production
and boot-smoke kernel compile checks, ABI/layout and kernel ELF checks pass.
The terminal-ISO GUI adapter follows the unchanged action/marker reconstruction
in the [wait queue receipt](wait-queue.md). No benchmark speedup is claimed.

## Remaining gates

`ProcessTableCell` and the private static mutable Vec getter remain UP-only.
Callback result lifetime bounds do not provide task pinning, exclusion or general
reentry safety. Other callbacks (including VFS/ProcFs) still need a complete
registry/task lock order. Atomic execution claims, cross-CPU wait/cancel and remote
TLB lifetime gates remain open. Self-signal delivery now uses the common target
lookup; no latency/throughput improvement is claimed or inferred from test wall
times. AP application dispatch remains disabled (`dispatch_cpus=1`).
