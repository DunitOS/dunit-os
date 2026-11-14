# Exclusive task execution and HAL return handoff

14 November 2026. Internal OP3/OP4 admission work; application dispatch remains UP.
This extends [registry/task ownership](process-registry.md), which pinned objects
but did not give a frame an exclusive right to execute them.

## Changes

Each registry record now has a separate execution state with a monotonically
increasing ticket generation and CPU owner. Lifecycle Ready/Blocked is not an
execution claim. The transitions are:

```
Idle -> Reserved(cpu, generation) -> Active -> Returned -> Idle
```

`dispatch::prepare` copies the hardware context/backing under the task lock and
reserves that record in the same metadata transaction. A second frame cannot
reserve it. Immediately before touching CR3/FPU/TLS or entering HAL, the lease
checks task Arc identity, owner lifecycle, Ready state, ticket and local CPU,
then publishes Running/has_run. The old separate `mark_process_started` path is
removed. The lease is CPU-bound and cannot be sent to another thread.

Yield, block, timer preemption and wake can publish a new lifecycle state while
the old frame still owns execution. `is_pid_runnable` excludes every outstanding
reservation/entry/return. Yield no longer enqueues its own task before HAL has
returned. Wake/preemption admission may be rejected while the lease is busy;
the lease retries Ready admission on release, outside all task/registry guards.
A Reserved frame dropped without entering also releases admission. Neither a
stale ticket nor a frame retaining an old task Arc can release a replacement.

HAL return resets TLS, user FPU and syscall stack policy before marking Returned.
The dispatcher retains the execution lease through exit/fault resource handling,
then releases it before selecting the next candidate. No mutex or IRQ guard spans
HAL/user execution. Normal thread teardown releases its Returned slot only after
TLS cleanup, in the same metadata transaction that detaches the completed task.
Join/wait cannot consume a still-owned exit record.

External autoreap/reap and group kill check execution ownership. Kill checks the
whole owner/thread group and publishes Dead/extracts resources in one metadata
transaction, so an already reserved sibling cannot enter afterward. An entered
group returns the existing NotRunnable/EAGAIN rather than freeing its active
stack/address space. Remote cancellation/stop acknowledgements are **not**
implemented by that refusal. An ordinary UP self-kill keeps the syscall escape
path. Owner exit likewise refuses to detach an entered sibling. AP application
dispatch stays disabled until complete stop/wake/mapping/progress gates pass.

## Validation

```sh
python3 tools/test_execution_ownership.py
python3 tools/test_process_access.py
python3 tools/test_user_word.py
python3 tools/test_scheduler_queue.py
python3 tools/test_wait_queue.py
python3 tools/test_process_resources.py --cpus 4 --accel tcg --output build/performance/execution-resources
python3 tools/test_execution_native.py --reuse-image --cpus 1 2 4 8 --accel tcg --output build/performance/execution-core
```

The host suite compiles the actual execution state and, separately, the actual
registry, task SpinLock and lease code with privileged IRQ/CPU/HAL shims. Eight
threads complete 8,000 admissions without overlapping ownership of one task;
Ready is published before the simulated hardware return and all competing
reservations must fail. The real group-quiescence helper rejects teardown while
a member is entered. A reserved child cannot enter after owner death. Replacing
a PID cannot let its stale lease start or release the new object's reservation.
A compile-negative case requires moving a lease between threads to fail. A
second 8,000-cycle state test checks CPU/generation identity, Active-release
rejection and Returned-phase retirement. These are synchronization tests, not
proof of concurrent native userspace.

The feature-gated native smoke uses real Process/AddressSpace, registry, leases,
kill/autoreap and wake callbacks. It verifies wake-before-return exclusion,
retirement rejection with VM ownership preserved, admission after final return,
and same-PID/first-ticket replacement. It deliberately does not execute synthetic
user instructions. Real resource, 1,000 join/1,000 detach, fault/lifecycle,
FPU/TLS, pthread/condvar, fixed-work CPU/allocator/I/O, GUI and C pointer scenarios
exercise actual HAL returns with the new leases. Existing marker budgets remain;
resource and hardware matrices additionally require TASK-EXECUTION success.

Nine native runs passed on the final images.
[The structured receipt](task-execution.json) records final images, commands,
source hashes and results. The production run forbids feature-gated proof
markers. Fixed-work counts and checksums must match the preceding production
receipt; PIT/TCG timing does not establish an SMP speedup or input latency.

## Remaining work

Exclusive admission and return handoff are one prerequisite for OP4, not SMP
completion. Remaining gates include atomic producer-predicate/check/registration
across pipe/PTY/IPC/wait-set, remote cancellation and group-stop acknowledgements,
interruptible syscall/IPI progress, full allocator/VM/driver lock ranking,
active address-space CPU masks and acknowledged TLB shootdown before page reuse.
Remote queues, idle/reschedule and real overlapping application execution are
still absent. APs run isolated hardware/PMM probes and park. KVM performance and
the remaining OP0/OP5–OP9 work cannot be accepted from these UP/TCG checks.
