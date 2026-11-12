# OP3 — remove scheduler -> process-table lock acquisition

12 November 2026. The ready queue previously called `is_pid_runnable()` while
holding its IRQ-safe lock. Lifecycle/wakeup paths also call the scheduler while
accessing process state. Adding a process-table lock around those accesses would
therefore create a queue -> process / process -> queue inversion.

The queue now owns only `(pid, generation)` records and its cursor. Admission,
selection and readiness validation call process-state functions **outside** the
queue lock. Selection copies one candidate, advances the cursor, validates its
state, then checks the candidate's identity before returning it. A failed state
check removes only that generation, preserving a newer requeue of the same PID.
Removing a stale entry adjusts the cursor so its successor is not skipped.
Excluded ready tasks remain queued. Traversal is bounded by the initial queue
length and allocates no snapshots or temporary lists on the dispatch hot path.

The outer IRQ guard preserves the existing UP transaction. Generation numbers
protect queue identity across unlocked validation; they do not atomically claim
Ready -> Running on a CPU. `has_ready_tasks` remains a readiness observation:
remote wakeup/check-sleep ordering is still required before SMP idle/dispatch.
Queue admission can become stale and is checked again during dispatch.

## Verification and measured limits

```sh
RUSTC=rustc python3 tools/test_scheduler_queue.py
python3 tools/test_process_resources.py --cpus 4 --accel tcg
```

The host test compiles the actual scheduler and actual SpinLock implementation,
with a no-op host shim only for privileged IRQ masking. It checks round-robin,
duplicate admission, exclusion, stale successor handling, empty/unavailable
queues and 8,000 requeues from eight host threads. Deterministic state callbacks
remove and requeue the same PID during both rejected and accepted observations;
they also re-enter queue operations, detecting a held queue lock by timeout.
Both stale removal and positive confirmation must preserve the new generation.
This tests queue storage, not concurrent execution of user tasks.

Native TCG checks passed on the new image: CPU/allocator/MemFS benchmark on one
vCPU; fifteen positive lifecycle/fd/VFS scenes plus a separate negative thread
fault/join scene on four vCPUs; GUI smoke (30 required markers) on four vCPUs;
C pointer modes and exact cursor hide/restore/exit pixels on eight vCPUs. Full
boot/DF/IRQ assertions remain in the native budgets. Existing host QEMU/guard
and generated ABI/layout checks also pass.

A checked fixed-work benchmark was measured before and after with one warmup
and five timed trials per scene, one vCPU, TCG and 512 MiB. Before used the
previous process-resources image: its hash was checked against that committed
receipt before boot. Thus the before image's build provenance comes from
`process-resources.json`, not the already edited worktree recorded at run time.
Both series preserve identical work sizes/checksums in all 36 records.

| Scene | Before median ms | After median ms |
| --- | ---: | ---: |
| CPU, 1 worker | 50 | 50 |
| CPU, 2 workers | 50 | 50 |
| CPU, 4 workers | 60 | 50 |
| CPU, 8 workers | 50 | 50 |
| Allocator | 240 | 230 |
| MemFS I/O | 30 | 30 |

These are coarse PIT elapsed values with 10 ms resolution and IRQ-off undercount,
not CPU accounting. One-tick differences do not establish a speedup. This small
run shows no large regression at that resolution; it does not bound scheduling
tail latency or KVM scaling. [Raw records and receipts](scheduler-queue.json)
retain the image hashes and matching work checks. All application workers still
execute on the UP scheduler.

The remaining OP3 gates include scoped/pinned process-table access, process and
scheduler lifecycle ownership, atomic wait/wakeup, PMM/VMM lifetimes and TLB
shootdown. This step removes the queue-to-table lock edge without enabling AP
userspace or claiming a complete SMP scheduler.
