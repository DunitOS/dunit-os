# Wait queue wake boundaries — OP3

12 November 2026. This removes a prerequisite for registry/task locking; it
neither enables AP application dispatch nor claims faster scheduling.

## Change and invariant

Previously every IPC, pipe, futex, poll, wait-set and timer wake held WAIT_QUEUE
while `wake_entry` accessed the process table and called scheduler admission.
Admission checks runnable state through the process table again. Meanwhile,
blocking paths touch process state before publishing a wait registration.
Adding process locks to those scopes would introduce lock-order/reentry hazards.

Registration storage now lives in `kernel/src/process/wait_queue.rs`.
A wake pass snapshots the next registration generation. It removes the first
matching older registration under the queue lock, releases the lock, then changes
task state and admits the ready task. Scheduler admission also runs after the
mutable table/Process borrow ends. No temporary batch Vec is allocated on the
wake/IRQ path. Registration insertion retains the existing Vec allocation model.

FIFO selection and futex `max` semantics are preserved (`0` means all). A wake
callback may cancel or re-register the same PID without deadlocking or making
the new registration eligible for the current pass. Generations do not wrap:
exhaustion fails explicitly rather than reusing an identity. Cancellation removes
pending registrations; it does not revoke a registration already selected by a
concurrent waker. The UP IRQ boundary remains in all production callers.

## Validation

`python3 tools/test_wait_queue.py` compiles the actual wait queue and kernel
SpinLock with only privileged IRQ operations replaced for the host. It checks
FIFO, limits, futex owner separation, deadlines, cancellation, nested wake,
same-PID replacement and 8,000 concurrent registrations with exactly 8,000
callbacks. Callback `try_lock` must succeed; retaining the lock would fail.
A callback re-registering itself under wake-all must terminate after the original
registrations. A canceled/replaced waiter must wait for the next pass.

Native commands and results are recorded in [the receipt](wait-queue.json).
All four final runs use the same full ISO: 15 positive scenes (75 required /
44 forbidden entries) plus the separate thread lifecycle fixture on 4 vCPUs;
full boot/runtime IPC on 1 vCPU (46 / 9); GUI smoke on 4 vCPUs (30 / 13).
All passed with no missing/forbidden markers. Total host wall times of 61.80 s,
10.56 s, 13.02 s and the GUI run in the receipt measure test invocation lengths,
not per-wake latency. No speedup is inferred.

For the terminal ISO, the GUI adapter preserves the entire original marker and
action budget; it attaches actions to explicit compositor launch so they start
before that command produces output. Reproduce the temporary manifest with:

```python
import json
from pathlib import Path
m = json.loads(Path("tools/gui_smoke_markers.json").read_text())
m["run"]["command_actions"] = {"exec /app/gui_server": m["run"].pop("actions")}
m["run"]["config"] = "limine_test_terminal.conf"
m["run"]["cmd"] = ["exec /app/gui_server"]
Path("build/wait-queue-gui-markers.json").write_text(json.dumps(m))
```

Use that path for `--markers-file` in the GUI command from the receipt.
The positive lifecycle budget preserves all original filesystem/fd/handle,
wait-set, pipe, condvar and TLS/errno/allocation assertions; the intentional user
fault/join fixture stays in its separate VM. No forbidden markers are relaxed.

## Remaining gates

`futex_wait` still accesses process/address-space state while holding the wait
queue lock for its compare/register transaction. Other check/block paths still
rely on UP, non-preemptible kernel execution and local IRQ masking. Therefore
this change removes wake callback lock nesting, not every queue→task edge.
Registry/task ownership and a single cross-CPU predicate/block/wake/cancel
protocol are still required before AP syscalls. No acknowledged TLB shootdown,
remote wakeup or atomic Ready→Running(cpu) claim is added. APs remain parked;
`dispatch_cpus=1`. There is no ABI, privilege or filesystem format change.
