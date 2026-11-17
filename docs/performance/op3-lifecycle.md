# Exit publication after cleanup

17 November 2026. This is an OP3 lifecycle ownership slice; the application
scheduler remains UP and remote cancellation is still open.

## Problem and implementation

The previous runner published `Dead/status` and woke child-exit wait sets before
thread TLS, owner threads and VM resources were retired. The external kill path
also published status before releasing fd/VM resources. `wait_for_child` checked
execution ownership, but the non-consuming child-exit snapshot accepted status
without that check. A concurrent waiter could therefore observe completion while
cleanup still owned resources; an external kill with no entered lease could be
reaped during cleanup itself.

Execution metadata now has a separate finalization fence. Admission closes under
the existing task -> registry lock order, the registry status remains absent,
and `busy` remains true throughout cleanup even after the execution lease drops.
External finalization can revoke an unused reservation, but rejects an entered
executor. The local runner must prove that its ticket reached Returned on the
same CPU. A finishing sibling also excludes owner teardown until its own cleanup
has completed. Reap, replacement, child readiness and join retain these gates.

The runner releases fd resources, thread/TLS resources and owned threads, restores
the previous root, retires VM resources and drops the dispatch frame before the
single status publication. Detached thread records are removed only at that
publication boundary. External kill uses the same fence and releases resources
before publication. Notifications and retired object destructors run outside
the task and registry locks.

TLS teardown uses a private idempotent reservation cleanup. Applications can
already remove TLS pages before ThreadExit; the public strict `unmap_range`
rejects holes and must retain that behavior. Private cleanup removes remaining
pages, updates shared-frame references and retires overlapping guard bookkeeping.
TLS allocation metadata is retained until that cleanup succeeds. Repeated cleanup
does not free pages or shared references twice. Invalid kernel bookkeeping and
missing owners still return an error; they do not clear the fence or publish a
false completion.

No syscall, public process state, rights, GUI protocol or exit-status encoding
changes. In particular, remote active-group kill still rejects rather than
pretending that a pin or notification has stopped another CPU.

## Verification

```sh
RUSTC=/path/to/rustc python3 tools/test_lifecycle_publication.py
RUSTC=/path/to/rustc python3 tools/test_execution_ownership.py
python3 tools/test_process_resources.py --cpus 4 --accel tcg
```

The first host test compiles the actual registry, execution leases, finalization
module and child-exit snapshot with CPU/IRQ and external-resource transport
shims. It checks 8,000 contended admissions, one cleanup owner, a forced waiter
observation during cleanup, separate HAL-return/lease-drop/publication gates,
reservation revocation, sibling/owner exclusion and reentrant wake/destructor
paths. A second fixture compiles the actual TLS cleanup and range validation:
partial/already-unmapped reservations, repeated cleanup, one-shot shared-reference
release, guard bookkeeping and invalid-range rejection are checked.

The feature-gated native execution smoke additionally uses real task/VM owners
and child wait/reap APIs. It checks active rejection, absent status through
cleanup and lease return, final VM pin release, successful status consumption,
and partial/already-unmapped TLS cleanup while public unmap stays strict:

```
[EXIT-PUBLICATION] wait/reap fenced through cleanup, VM pins and HAL lease return OK
```

The resource and thread-lifecycle suites remain necessary for fd/PTY/capability
cleanup, real TLS/FPU and detached/fault behavior. Host transport shims do not
prove those hardware paths. Native run receipts belong to the integrating
verification report; this document does not claim a run from reading old logs.

## Remaining gates

This change supplies the post-cleanup publication boundary for a future stop/ack
protocol. It does not implement remote stop requests, interruptible syscalls,
reschedule IPIs, nested remote cancellation, AP application execution, full
shared-frame/DMA final-reader ownership or SMP scaling. A remote cancel handler
must return through IRET and must not escape directly from an NMI: bypassing IRET
would retain NMI blocking and break subsequent TLB acknowledgements.

Fresh TCG native checks and their source/image hashes are recorded in the
[17 November integration receipts](../verification/multitrack-2026-11-17.json).
