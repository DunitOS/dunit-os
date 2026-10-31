# Queued pointer input proposal

Status: proposed, awaiting the project owner's ABI decision under roadmap §31
and contributor rules §1. No syscall or wire-format change is implemented by
this document.

## Problem and evidence

The compositor currently reads GetMouseState=60 once per desktop iteration and
derives button edges from two snapshots. A complete down/up between snapshots
is invisible. The kernel already records MouseEvent values in a bounded ring,
but there is no userspace operation to consume them.

On ed32e1f under QEMU 10.0.13 TCG, the native SDK buttons scenario failed twice
at the final pointer activation with its original 200 ms intervals. The same
required/forbidden markers passed with 1000 ms intervals. Themes and SVG UI
also passed only with longer pauses. This is evidence of input timing sensitivity;
the snapshot implementation explains how edges can be lost. Increasing delays
is diagnostic, not a production fix.

## Recommended additive contract

Add ReadPointerEvent through abi/syscalls.abi and the existing generator. Pick
its number only after fetching and checking the current manifest and kernel
number space; 90 is currently free, but is not reserved by this proposal.

Arguments are an INPUT_MASTER handle, an output address, and an output byte
length. The operation accepts exactly a 32-byte v1 record. The handle must be
owned by the caller and carry the existing INPUT_MASTER right. No new rights
bits or ambient input privilege are introduced.

The little-endian record has the following fields:

| Offset | Type | Meaning |
| --- | --- | --- |
| 0 | u32 | absolute x in the published display bounds |
| 4 | u32 | absolute y in the published display bounds |
| 8 | u32 | button mask: left/right/middle in bits 0/1/2 |
| 12 | i32 | wheel delta belonging to this event |
| 16 | u64 | event sequence, in production order |
| 24 | u32 | flags; bit 0 means events were lost before this record |
| 28 | u32 | reserved, zero |

Success returns 0 and consumes one event. An empty queue returns EAGAIN;
wrong length returns EINVAL, invalid user output returns EFAULT, and an
unauthorised handle returns EACCES. Failed copies and invalid arguments do not
consume input. Capability acquisition/release must reset the consumer baseline
and prevent a new owner receiving a previous session's queued input.

The queue stays bounded and IRQ-safe. Consecutive pure motion events may be
coalesced only with identical button state and without discarding wheel deltas.
Button changes preserve order. Saturation must be explicit through LOST_BEFORE;
the consumer cancels capture/press state and resynchronises instead of fabricating
a click. Kernel code stores device facts; userspace chooses cancellation policy.

GetMouseState=60 and its wheel-accumulator semantics remain unchanged. No gui-v1
wire bytes change. A safe libdunit wrapper exposes Result<Option<PointerEvent>>.
The compositor drains queued input and routes each transition in order before
rendering, using the event's coordinates rather than the latest snapshot.
A bounded processing budget may defer queued work to another iteration without
dropping it. Existing GUI control messages remain the client transport.

## Alternatives

Keep snapshots and proceed to the next M7.6 widget: no ABI change, but the known
short-click failure remains. Raising polling frequency or slowing test input
does not guarantee preservation of transitions between reads. Reinterpreting
GetMouseState as a queue read silently changes its existing contract and is not
recommended.

## Verification before commit and push of implementation

- ABI generator consistency; existing syscall numbers and snapshot behaviour.
- Queue host tests: down/up without intervening reads, ordering/coordinates,
  coalescing, saturation/resynchronisation and bounded storage.
- Native negative tests: INPUT_MASTER isolation, wrong length, invalid output,
  EAGAIN, and no event loss on failed copies.
- A QEMU regression injecting a complete click while the compositor cannot
  sample it, then asserting exactly one activation and correct cancellation.
- Original SDK buttons, checks, input, scroll and autoscroll scenarios with their
  original timing and marker expectations; theme/SVG UI and GUI smoke.
- Headless GUI protocol tests and boot regression. Investigate the separate
  high-resolution switching failure; do not declare it fixed by input changes
  without a successful original scenario.
