# Pointer confinement and relative lock

Approved by the user on 10 November 2026 after review of the compatible public
input and gui-v1 extension. This is application input policy in the compositor;
the kernel exposes original device motion, not window ownership or cursor policy.

## Modes and lifetime

- `Normal`: ordinary visible desktop pointer and absolute local coordinates.
- `Confined`: visible pointer bounded by the focused window's client area.
  Bounds follow window geometry. Relative devices move the confined pointer from
  original deltas, so reversing at an edge responds immediately.
- `Locked`: hide and freeze the pointer, deliver signed original device deltas
  without an artificial display-edge limit. This is the mode for a game camera.

A request is authenticated against the requesting connection's own focused,
visible live window. Background acquisition fails with EACCES. A rejected request
preserves the previous effective mode. Lock requires a relative device source;
absolute-only input reports EOPNOTSUPP rather than inventing motion from clamped
positions. Source changes revoke an active relative lock.

Focus loss, workspace changes, minimization, client exit/disconnect, display or
session reset, lost input history and shell focus scopes release capture. Focus
return does not reacquire it. The configured `[shortcuts] cmd_mod` plus Escape
(default Super+Escape) releases capture in the compositor, independently of client
responsiveness. The release cancels retained pointer capture/selection through
Leave. Desktop shortcuts remain compositor-owned.

After a relative capture, the visible pointer continues from its logical position
using device deltas, avoiding a jump to the kernel's separately clamped physical
projection. A normal absolute-source event supplies its ordinary position.

## Rust

`Window` and `ComponentWindow` expose `supports_pointer_mode()`,
`request_pointer_mode(PointerMode)` and `pointer_mode()`. The request is
asynchronous: success returns a request id, not a grant. The effective state
changes only on authenticated compositor input.

- `IN_POINTER_MODE`: `x` is the request id, `y` is zero or a negative native errno,
  and `value` is the effective mode. Unsolicited revocation has id zero,
  `y = -EINTR` and Normal mode.
- `IN_RELATIVE`: `x/y` are signed device deltas, with no window scaling or SDK
  acceleration. `value` packs current buttons and physical modifiers through the
  existing helpers. Do not feed these deltas to absolute widget hit testing.
- Buttons, wheel, keyboard and focus continue using their existing event kinds.

## C

The existing native callback in `DunitTransfersV1`, supplied through
`DunitUiExtendedV1`, receives these events in `event->input`. The session is valid
only during that callback, on the UI thread:

```c
/* Request from a click/key handler; zero means queued. */
int rc = dunit_ui_pointer_mode_v1(session, DUNIT_POINTER_LOCKED);

/* In the same native-event callback: */
const DunitUiNativeInputV1 *i = event->input;
if (i && i->kind == DUNIT_GUI_IN_RELATIVE) {
    /* Rotate the camera by i->x and i->y. */
}
if (i && i->kind == DUNIT_GUI_IN_POINTER_MODE) {
    /* i->value is effective; i->y reports refusal/revocation. */
}
```

Use `DUNIT_POINTER_CONFINED` or `DUNIT_POINTER_NORMAL` for the other modes. One
mode command may be queued per callback; another returns EBUSY. Invalid modes
return EINVAL and an unsupported compositor returns EOPNOTSUPP. The command is
sent after the callback releases its borrowed native session; the acknowledgement
arrives as a subsequent native input event. Do not retain or reuse the session.

Existing C v1 record layouts, callbacks, exports and values are unchanged. The
new function, constants and checks are generated through the existing schema and
header generator. No second C input implementation is introduced.

## Wire and kernel compatibility

Optional gui-v1 feature bit 3 (`FEATURE_POINTER_MODE`) gates the new events.
The existing 20-byte INP1 layout remains unchanged. `PTR1` is a 16-byte sideband
request: magic, nonzero request id (at most INT32_MAX), mode, reserved zero u32;
all fields are little endian. Malformed/unnegotiated requests do not acquire the
seat. At most one request per client is pending; excess receives EBUSY.

ReadPointerEvent remains syscall 90 with the same INPUT_MASTER check and
transactional failed-copy behavior. Length 32/40 retains exact v1/v2 bytes.
Explicit length 56 requests v3: bytes 0..40 are v2, signed i32 dx/dy at 40/44,
u32 relative-source flag (0/1) at 48, and four reserved zero bytes at 52. An
absolute-source record must have zero relative deltas. Old kernels return EINVAL
for the new length. Pure motion coalesces by summing deltas, preserving source,
button and modifier boundaries; arithmetic overflow is explicit stream loss.
No syscall number, right, privilege or disk format changed.

The native acceptance source is `assets/tcc/pointer.c`; its mouse/key contract is
`tools/c_pointer_markers.json`. It compiles and links with TCC inside Dunit.
Run `python3 tools/test_c_pointer.py --accel tcg` for the complete native contract
and framebuffer checks. `--reuse-image` skips rebuilding; `--verify-only` checks
existing screenshots. Cursor checks match the actual opaque cursor asset pixels,
require absence while locked, and require restoration at the same anchor after
emergency release and abrupt client exit. They also compare the surrounding client
pixels before lock and after release.

## Verified acceptance

The native C client locked before its first physical movement, reached all four
confined edges, and received **+5080 / -5080 on each axis** while locked. Both
mouse transitions, both wheel directions and keyboard input arrived. Emergency
release, Alt-Tab, workspace changes, background refusal and abrupt `_Exit(0)`
passed; focus return required an explicit new lock request. Exact cursor-asset
pixels proved visible → hidden → restored → restored after exit at **350,220**.
Host pointer/queue/protocol/SDK/C tests passed (**209 tests**), along with generated
ABI and frozen C-v1 layout/signature checks.

Existing native pointer-queue and full GUI smoke budgets passed unchanged; the
30-marker GUI action replay used the terminal image with its original waits
attached to the `exec /app/gui_server` command. The existing native C counter
passed its marker contract and exact Rust pixel references (initial and updated).
The frozen C-v1 ABI and archive-export packaging checks also passed.
