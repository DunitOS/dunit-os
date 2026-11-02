# M7.6 input lifecycle extension — approved contract

The user explicitly approved this proposal with “Разрешение дано, делай”. The
negotiated gui-v1 keyboard/focus extension is implemented. This records the
approved scope under CLAUDE.md §1; it does not close the entire M7.6 milestone.

## Negotiated contract

- `FEATURE_INPUT_LIFECYCLE` (feature bit 1) is offered in HELLO and accepted
  through WELCOME. Clients offering no bit keep their established events/bytes;
  a new native client falls back to legacy behavior if the server omits the bit.
- The 20-byte INP1 envelope and existing kind values are preserved. KEY_UP=7,
  FOCUS_IN=8 and FOCUS_OUT=10 are sent only to negotiated clients. Key-up carries
  scancode/modifiers and no cooked text. Focus messages have zero payload fields.
- Native push/checkable activation arms on Space/Enter make, exposes pressed
  state while held, suppresses repeated activation and commits once on its
  matching release. Escape, focus loss or an invalidated target cancels it.
- Focus-out cancels pressed/captured gestures, pending keyboard activation,
  menus/tooltips, text drag and selection timers. Focus-in restores a still-valid
  stable focus owner without reopening a popup or completing a cancelled press.
- The pointer payload keeps the button in low 8 bits and reserves bits 8..11 for
  Shift/Ctrl/Alt/Super. Central codec helpers validate/decode it; legacy projection
  strips those bits. SDK selection/routing consumes modifiers supplied by that
  payload. **The live compositor currently sends zero pointer modifiers:** correct
  device snapshots require the separate queue contract described below.
- The kernel key-event ABI already contains make/break and modifiers and is
  unchanged. No syscall, privilege or kernel/userspace boundary change is part
  of the implemented keyboard/focus extension.
- Feature-aware decoding rejects unknown kinds, invalid modifiers/reserved fields,
  nonzero key-up text and malformed focus payloads. The historical legacy decoder
  and its accepted short quit/leave forms remain unchanged.

## Pointer snapshot decision remains separate

Keyboard and pointer records come from independent queues without a common
ordering/timestamp. Sampling the last drained keyboard state cannot reliably
recover modifiers at a queued click. The approved proposal required another
human decision before changing that public pointer-queue contract.
[Pointer modifier snapshots](pointer-modifier-snapshot-proposal.md) now describes
the proposed compatible length-selected ReadPointerEvent extension. It is not
implemented or covered by the earlier approval. Native Shift/Ctrl pointer
selection remains pending; no approximate modifiers are fabricated.

## Verification coverage

Protocol and SDK host tests cover negotiation, exact legacy projection, strict
payload validation, held/repeated/released keys, cancellation, focus restoration,
selection state and routing. The native `lifecycle` fixture pairs a negotiated
SDK client with a deliberately legacy peer and checks held Space/Enter, Escape,
focus transfer, popup dismissal and unchanged legacy key/pointer bytes through
`tools/qemu_test.py`. On 2026-11-02 all lifecycle markers passed and both clients
exited with code 0. Resize recovery and three theme framebuffer references also
passed on the same final sources in TCG.

This approval does not authorize popup surface roles, global grabs, clipboard,
IME wire messages or privilege changes. Those remain separate contracts.
