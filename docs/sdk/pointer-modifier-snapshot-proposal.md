# Pointer modifier snapshots — approved compatible ReadPointerEvent extension

The approved gui-v1 lifecycle proposal explicitly requires a separate decision
if pointer-queue public semantics must change. That condition was confirmed:
keyboard and pointer have independent queues, and the compositor drains a queued
pointer record before draining keyboard records. Neither queue shares a timestamp
or sequence. Sampling current/last keyboard modifiers can therefore apply the
state from a later release to an earlier click. Draining keys first merely changes
the race. This must not be represented as correct pointer selection.

Approved on 2 November 2026: after the 32/40-byte decision was presented, the user
instructed “Делай сразу весь M7.6”. The compatible extension below is implemented.
The original keyboard/focus approval alone did not cover this kernel change.

Contract:

- Keep syscall 90, its three existing arguments (INPUT_MASTER handle, output,
  length), privilege checks, EAGAIN and transactional EFAULT behavior.
- Preserve length 32 as exact existing v1 bytes. Reserved bytes remain zero.
  Existing kernels/clients and v1 decoders are unchanged.
- Add an explicitly requested length-40 v2 record. Bytes 0..32 retain v1 layout;
  bytes 32..36 hold a u32 Shift/Ctrl/Alt/Super mask (bits 0..3), sampled in the
  input IRQ critical section when the pointer sample is enqueued. Bytes 36..40
  are reserved zero. Unknown modifier bits fail decoding.
- The existing key-event ABI stays unchanged. The kernel exposes device modifier
  facts, while selection/focus policy remains in userspace. No new syscall number,
  rights, manifest format, or event timestamps are introduced.
- Queue entries retain the snapshot even if keys change before the reader runs.
  Motion must not coalesce across a modifier change. Overflow/handover keep the
  explicit LOST_BEFORE capture-reset semantics; an ownership reset snapshots the
  current modifier state too.
- New libdunit `read_with_modifiers` requests 40 bytes; old `read` requests 32.
  An old kernel returns EINVAL for the new length, rather than inventing modifiers.
  The compositor must not silently approximate missing pointer snapshots.
- Tests: exact v1 codec preservation; v2 validation; failed-copy retention; queue
  modifier ordering/coalescing; native held-Shift/Ctrl click with release before
  reader resumes; denied INPUT_MASTER; existing pointer queue and GUI regressions.

Alternative: a unified timestamped/ordered keyboard+pointer stream. This is a
larger ABI and delivery change, unnecessary for a per-pointer modifier snapshot.
The length-selected v2 is the approved narrow contract for this behavior.

Verification: the native pointer queue contract passed v1 short clicks, v2
Shift/Ctrl snapshots after key releases, failed-copy retention, access denial
and ownership handover. The `modifiers` GUI fixture passed 20 actual pointer
gestures across Input, Textarea, List, Tree and Table, with exact selections,
unchanged text/undo state and bounded rows for 10,000-item collections. The
compositor explicitly requests v2 and fails on an unsupported old kernel rather
than approximating modifiers from the independently drained keyboard queue.
