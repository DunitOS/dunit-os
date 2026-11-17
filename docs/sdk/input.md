# SDK input

Native adapters and pure host applications share routing, focus and editing
controllers. Public device and gui-v1 extensions below preserve older record
lengths and negotiated legacy behavior.

## Focus ordering

All enabled focusable controls participate in stable document order by default.
`.tab_index(n)` orders positive values first, then zero-valued controls; ties retain
document order. Negative values skip Tab, while allowing pointer/programmatic
focus. Disabled controls, inactive tab pages and controls outside the active modal
scope are excluded. Radio groups and toolbars keep their roving single Tab stop.
Shift+Tab reverses the same order with wrap.

`.mnemonic('f')` binds Alt+F to focus a control and invoke its action if present.
Inputs receive focus without inserting text. Disabled controls are skipped.
Duplicate mnemonics within a focus scope are validation errors; a dialog can reuse
a background mnemonic because its scope is exclusive. Popup menus retain their
existing mnemonic behavior. Focus/disabled appearance and layout changes resolve
through DSS state selectors (`:focus`, `:disabled`).

Committing a focused spinbox may rebuild the entire view or open a modal. Pointer
and mnemonic dispatch recheck scope and availability after that commit, preventing
an action on the background with the same event. Host tests in `focus_order.rs`
cover ordering, reverse traversal, negative indices, disabled/field mnemonics,
modal restoration and that reentrancy case.

## Event routing

`View::on_capture` and `View::on_bubble` register typed handlers returning
`EventResponse<Message>`. Each `UiEvent` carries the original stable `target`,
`current_target`, phase, payload and accumulated `default_prevented` flag.

The route runs from root to target in capture, then target to root in bubble.
Both phases precede the widget default action. `Response::message(message)`
updates the component immediately. `.stop()` stops later handlers but permits
the default; `.prevent()` suppresses the default without stopping propagation.
They may be combined. `Response::default()` observes without changing the model.
Message composition maps the entire response, preserving both flags.

After every handler, routing checks that the target still exists, has the same
kind and ancestor path, remains enabled and within the same modal scope. A
keyboard event additionally requires unchanged focus. If any condition fails,
the stale route and default are cancelled. A modal dialog limits handlers to its
own subtree; background ancestors do not receive dialog events.

`Application::key`, `pointer`, `pointer_at`, `scrollbar_pointer` and
`pointer_text_at` route their input before performing defaults. Captured pointer
Move/Up/Leave routes to the gesture owner, while activation still requires a
release over the original control. A prevented Down/Up/Leave cancels transient
press/drag state without emitting a click or modifying text selection.

`ComponentWindow` routes each native packet once before all SDK defaults,
including raw arrow/function keys, collection navigation, text editing,
scrollbars, wheel, menus, sliders and splitters. Nested application helpers are
suppressed only for that dispatch; errors restore routing for the next event.
Native make packets use `UiEventKind::NativeKey`; negotiated releases use
`NativeKeyUp`. Direct cooked key calls use `Key`. Negotiated FOCUS_IN/FOCUS_OUT
are handled as window lifecycle before widget routing: focus-out cancels gestures,
pending activation, overlays and selection timers; focus-in restores a valid
stable owner. Focus changes are not synthesized as widget key events.

Pointer coordinates are available for native events and `pointer_text_at`;
plain host `pointer` calls supply no coordinates. The codec and SDK route supplied
pointer modifiers captured when the device event was enqueued, through the
[approved pointer snapshot contract](input.md#pointer-modifier-snapshots).
Reading the last keyboard state would misorder queued clicks.
Legacy peers retain their earlier event stream; routing invents no releases,
focus changes or device modifier snapshots for them.

Programmatic commands such as `edit`, `table_command` or `update` do not invent
hardware events. Timers and view reconciliation are also separate operations.
For a custom adapter, `route_event` performs handlers only and returns whether
the adapter must suppress its default. Native bracket helpers are adapter
plumbing, not application commands.

## Text input

`View::input` adds an editable field to the typed component tree. It uses the
same focus order, native window session, shared DSS and damage pipeline as the
[button family](widgets.md#checkable-controls).

```rust
View::input(
    "query", "Search", self.query.clone(), Msg::QueryChanged,
    InputOptions { placeholder: "Type a query".into(), ..InputOptions::default() },
)
```

`Msg::QueryChanged` is an enum constructor taking `String`. A function pointer
(or a noncapturing closure) maps the proposed value to a model message. `update`
owns the accepted text; the SDK does not keep an independent authoritative value.
`InputOptions` also supplies `password`, `read_only` and `max_chars`.

### Editing and state

- Printable characters replace the current selection; all indices stay on
  UTF-8 extended grapheme boundaries. Native hardware input currently supplies cooked ASCII;
  Unicode values and edits are supported by the pure API. A system IME remains future work.
- Left/Right, Home/End and Shift variants move or extend selection. Ctrl+Left/Right
  and Ctrl+Backspace/Delete use whitespace-delimited runs. Ctrl+A selects all.
  Backspace/Delete remove the selection or adjacent grapheme/run.
- Ctrl+Z undoes; Ctrl+Y or Ctrl+Shift+Z redoes. Each successful text mutation is
  one action; history is bounded to 64 snapshots. A new edit clears redo.
  Replacing selected text with itself collapses selection without creating a
  model message or undo entry.
- Primary click places the cursor; dragging selects within the client, including
  across another node in the same window. Pointer Leave cancels capture.
  Shift+click extends the existing anchor, Ctrl+click selects a whole word and
  Ctrl+Shift+click extends by whole words. Double/triple click selects and drags
  whole words/logical lines through the shared [text gesture](input.md#multi-click-selection)
  controller. Native modifier delivery requires the approved per-event
  [pointer snapshot](input.md#pointer-modifier-snapshots), rather than sampling
  keyboard state after a queued click. Its final native verification is recorded
  in the [M7.6 acceptance matrix](widgets.md#verification). Timed
  [selection edge autoscroll](input.md#selection-autoscroll) supports horizontal drag
  selection with a stationary captured pointer.
- Enter is not inserted into a single-line field; root application shortcuts may
  handle it. Text and editing shortcuts consumed by a field do not also activate
  root shortcuts. Tab/Shift+Tab traverse fields, buttons and radio groups.

Transient editing state follows the stable key and kind across view rebuilds and
reorder. Accepted proposals retain cursor, selection and history. If the model
returns the pre-edit text, the complete pre-edit editor is restored. If it
transforms a proposal or independently replaces the text, history resets and
positions clamp to valid boundaries. Disabled transitions/removal/retype start
fresh state; disabled fields cannot focus or edit. Read-only fields can focus,
navigate and select but reject text mutations and history traversal.

`TextEditor`, `EditCommand` and `Application::edit` provide the same pure host
path. `View::text_editor()` exposes read-only diagnostics. `Renderer::input_index`
resolves pointer positions using the actual font, password mask and scroll;
`Application::pointer_at` applies this position to selection. The native adapter
does this routing automatically before returning its original window event.

### Appearance and validation

The shared dark/light DSS profiles style Input background, border, focus,
read-only and disabled states. `placeholder-color`, `selection-color`,
`caret-color`, `caret-width` and `caret-blink` supplement the common typography
and padding properties. Appearance values come from profile tokens.

The renderer clips selection, glyphs and cursor inside the padded content box.
It preserves the previous horizontal viewport while fitting the cursor after
editing, navigation, resize or font changes. Password mode paints one bullet per
Unicode scalar; it is display masking, not protected storage or memory erasure.
Empty values display the configured placeholder.

The focused cursor blinks through the existing wait-set/deadline loop. The DSS
`caret-blink` duration controls it; zero means a steady cursor. It does not spin
or extend the caller's requested timeout. Read-only fields also show a cursor.
With negotiated [input lifecycle](input.md#keyboard-and-focus-lifecycle), keyboard
focus-out hides the caret and cancels drag/selection timers without editing the
text or history. Focus-in restores a valid stable owner. Legacy peers keep the
old event stream and cannot infer external keyboard focus loss.

Invalid model values are rejected as `UiError::InvalidInput`: control characters,
line separators, values over `max_chars` Unicode scalars or the technical
64 KiB byte bound. Placeholders use the same single-line/byte validation. A
rejected keystroke at a limit leaves text, selection and history unchanged.
Theme validation rejects invalid cursor colors, dimensions and blink duration.
Validation failure follows the existing component contract: no model rollback;
the previous valid view remains and the native session is poisoned.

Cursor placement, Left/Right, selection endpoints, Backspace/Delete and logical
vertical columns follow Unicode extended grapheme boundaries using the
`no_std` `unicode-segmentation` 1.12.0 implementation. Combining sequences, emoji
skin tones, ZWJ families and regional-indicator flags cannot be split by these
operations. Arbitrary pointer byte offsets snap to the beginning of their cluster.
Insertion/deletion that merges neighboring clusters moves the caret to a valid
boundary after the resulting cluster. Undo/redo retain complete text/selection
snapshots. Word commands still use whitespace runs (not linguistic words), now
on whole clusters. `max_chars` remains the existing scalar-count limit.

Shaping and system IME remain separate facilities.
Host `dunit-widgets/tests/graphemes.rs` checks composition, deletion, navigation,
vertical columns, selection, history and merging neighboring emoji clusters.

## Multiline input

`View::text_area(key, label, value, on_change, options)` extends the
[single-line editor](input.md#text-input) using the same controlled model, stable
identity, focus traversal, undo/redo and shared DSS pipeline.

```rust
View::text_area(
    "document", "Document", self.text.clone(), Msg::TextChanged,
    InputOptions::default(),
)
```

The default height grows with its parent; explicit layout attributes can bound
it. The accessibility role is `textbox` with the supplied label. Values accept
LF line separators. CR, tabs and other controls remain invalid; native Enter is
translated to LF. Password mode is rejected for a text area. Limits remain
`max_chars` Unicode scalars and 64 KiB of UTF-8, including separators.

### Editing and viewport

Enter replaces a selection with LF. Backspace/Delete join lines when removing
a separator. Selection, replacement and undo/redo include LF bytes. Home/End
move to the logical line edges; Ctrl+Home/End move to document edges. Up/Down
retain the preferred scalar column across shorter and empty lines; Shift
extends the selection. Horizontal movement, pointer placement and edits reset
the preferred column. Logical-line navigation retains an extended-grapheme column; soft-wrap
navigation retains a pixel X from the shared visual-line map.

Rendering uses explicit LF lines by default; `soft_wrap` enables visual wrapping. The caret is revealed
on editing, navigation, ordinary text focus, resize and font geometry changes. Pointer hits
use both axes and the current scroll offsets. Drag selection crosses lines;
selection and glyphs are clipped to the content box. Visible glyph work is
bounded using font-wide ink bounds, including overhang from adjacent logical
rows. Pixel-reference tests cover negative bearings, positive overhang, tall
accents and deep descenders. Timed
[selection edge autoscroll](input.md#selection-autoscroll) supports stationary captured
pointers on both axes. The shared [scroll controller](layout-window.md#scroll-areas) supplies
both scrollbars.

The native adapter routes wheel input to the hovered node, falling back to the
focused field when there is no hovered target. Negative native wheel deltas
move up; positive deltas move down. The shared DSS `scroll-step` property sets
lines per notch (dark/light token `input-scroll-step`, currently 3); zero
disables wheel movement. Validation accepts finite values from 0 through 1024.
The renderer clamps the viewport to the document. Manual scrolling survives
caret blinking and no-op model rebuilds; the next caret action reveals the
insertion point even if its byte index is unchanged.

`Renderer::input_index_at`, `input_scroll_y`, `scroll_input` (logical lines) and
`scroll_input_wheel` expose the pure path. `ComponentWindow::input_scroll_y`
exposes native diagnostics. View rebuilds retain editing state only for a
matching key and kind: changing between Input and Textarea starts fresh state,
including when the model rejects the proposed text.

## Soft wrap

`InputOptions { soft_wrap: true, ..Default::default() }` enables word wrapping
for `View::text_area`. The default remains logical LF lines with horizontal
scrolling. Enabling it on single-line Input or Spinbox is invalid.

Wrapping never inserts or removes bytes. `dunit_render::VisualLines` maps
extended grapheme boundaries to visual rows using the same font advances as
painting. It prefers whitespace boundaries; an unbroken word wraps between
grapheme clusters. An oversized cluster occupies one row and may overflow
horizontally rather than being split. Zero-width viewports still make progress.
LF creates a hard break, including empty and trailing lines. Placeholder text
uses its own unmasked layout and never changes scroll extents.

The line map is shared by text/selection/caret painting, pointer hit testing,
scroll extents, caret reveal and visual navigation. Scrollbar gutters stay
reserved, so hiding a bar does not rewrap text. Width/font changes recompute the
map while preserving the model and byte-based selection. Wrapping does not
introduce an editing history entry.

At a soft break, one byte offset has two caret positions. `TextPosition` carries
`CaretAffinity::Upstream` (end of previous row) or `Downstream` (start of next
row). Native click, drag and selection-autoscroll preserve that affinity.
Compatibility `input_index*` APIs return only the byte offset; callers needing
visual placement use `input_position_at`/`input_drag_position_at` and
`Application::set_input_affinity` after placement.

Native Up/Down navigate visual rows and retain a preferred pixel X across short
rows; Home/End target the current visual row. Shift extends the existing
selection. Ctrl+Home/End retain document semantics. The native adapter calls
`Renderer::edit_input`; host event drivers should use the same method after
rendering the current view. Without a current matching render snapshot, it
falls back to the editor's logical-line navigation. Ordinary `Application::edit`
remains a geometry-independent editing API. Text changes, horizontal navigation,
mouse placement and Home/End reset the preferred X. Multi-click word/line
selection retains its documented whitespace-run/logical-line semantics.

Tests in `dunit-render/tests/soft_wrap.rs`, `dunit-render/tests/text_input.rs`
and `dunit-component/tests/soft_wrap.rs` check word/grapheme breaks, empty
lines, zero width, both caret affinities, pointer placement, vertical goal,
Shift selection, rejected edits/history, resize/reveal and fresh-render pixels.
Wrapped pixels and both caret positions also match an explicit-LF reference
without mutating the wrapped model.

The retained renderer prepares one immutable `PreparedInput` after final layout
for each input snapshot. Its value `VisualLines` map is reused for caret fitting,
vertical limits, scroll extents, painting, caret positioning, hit testing and
visual navigation. An empty field prepares a separate placeholder map only when
placeholder text is actually displayed. This removes repeated whole-document
geometry passes within a render; it is not a persistent text or glyph cache.

Every render rebuilds geometry from the current text, resolved font/style,
content width, password and wrapping settings, so edits, undo, resize and theme
changes cannot retain a stale map. The current snapshot keeps it through input
hit testing until the next render replaces that snapshot. Scroll/caret/focus
changes do not alter the map itself. Existing `TextInput` struct literals and
standalone methods remain valid; the old painter prepares locally when no
prepared-input lookup is supplied. Custom prepared callers must use geometry
from the same text/font/style/width as the input they paint.

## Multi-click selection

Input and Textarea support primary single, double and triple clicks through the
same pure `Application::pointer_text_at` path used by the native adapter. A
single click places the caret, a double click selects the whitespace/non-whitespace
run, and a triple click selects the logical line including its LF separator.
The fourth click starts a new single-click sequence. Empty final lines remain
valid caret positions. This selection does not change the text or create undo
history; replacing it is one ordinary controlled edit.

DSS supplies `input-multiclick-interval` (duration) and
`input-multiclick-distance` (pixels). Both profiles define these through named tokens. An interval of zero disables multi-click classification;
a missing property preserves ordinary single-click selection. A click streak
requires the same stable field key, unchanged text, unchanged policy, monotonic
time and Euclidean distance within the configured threshold. Keyboard input,
wheel, resize, window leave and interactions with another control cancel it.

Dragging after a double/triple click retains the original word/line as its
anchor and extends by whole words/lines, including when reversing direction.
The stationary edge-autoscroll timer uses the same anchor. Release ends capture;
focus loss, disable, removal or externally replaced text cancels capture.
Read-only fields still support selection. All indices normalize to extended-grapheme boundaries. Unicode shaping
and linguistic word boundaries remain unsupported.
Native Shift+click extends the existing anchor; Ctrl+click selects a word and
Ctrl+Shift+click extends to its word boundary. The compositor forwards the
[enqueue-time modifier snapshot](input.md#pointer-modifier-snapshots), including
when the physical modifier is released before the pointer queue is read.

`TextClickState`, `TextClickConfig` and `TextClickSample` expose deterministic
host input. The native adapter samples its monotonic clock and reads the policy
from the last successfully painted DSS state; no synthetic events, polling
loop or wire-format change is involved. Capture/bubble handlers may prevent
the entire default gesture, including caret movement.

Host regression:

```sh
cargo test --manifest-path runtime/dunit-component/Cargo.toml --test click_selection
```

These tests cover timing/distance/clock resets, click cycling, whole-word and
whole-line drag reversal, edge-autoscroll anchors, UTF-8, empty final lines,
external text replacement, disabled state, controlled replacement and undo.
Native acceptance must additionally exercise real timed QEMU pointer input.

## Selection autoscroll

The SDK extends a captured text selection while the pointer stays outside the
visible text viewport. Input scrolls horizontally; Textarea scrolls on both
axes. The pointer may remain stationary: the native wait-set deadline advances
the viewport and selection without synthesizing mouse events.

`selection-scroll-interval` is a DSS duration and `selection-scroll-step` is
content pixels per tick. Shared dark/light profiles select 32ms and 12px. Zero
interval or step disables autoscroll. Durations must be in 0..60000ms and steps
in 0..1024px. A delayed frame advances one step without a catch-up burst.

Only a focused, enabled field with an active text-selection capture schedules
ticks. Read-only text remains selectable. Each axis stops at its content limit;
returning inside the viewport stops its movement. Release, window Leave, focus
change, field removal/disable and scrollbar gestures end selection capture.
A pending input event is delivered before the next timer step. No timer remains
active when neither axis can move.

Captured pointer positions clamp to the visible viewport, including ancestor
scroll clips. A tick uses the new scroll offsets to find the next UTF-8 scalar
position and extends the original selection anchor. It preserves the model
text and undo/redo history. The pending viewport is applied after caret fitting,
so scrolling across short LF lines does not reset the other axis. Published
paint snapshots remain immutable until the next successful render. Captured
pointer placement pins both offsets as well, so motion between timer ticks
also preserves the other axis. Interval queries use cached axis measurements
without copying text or measuring glyphs.

Pure adapters use `Renderer::input_drag_index_at`, `input_autoscroll_ms` and
`input_autoscroll_step`, then `Application::drag_selection`. The application
method accepts only the currently captured, focused enabled input. Native
`ComponentWindow` routes this automatically using its existing wait-set loop.

## Keyboard and focus lifecycle

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
  payload. The compositor forwards the enqueue-time device snapshot from the
  separately approved 40-byte pointer queue record described below.
- The kernel key-event ABI already contains make/break and modifiers and is
  unchanged. No syscall, privilege or kernel/userspace boundary change is part
  of the implemented keyboard/focus extension.
- Feature-aware decoding rejects unknown kinds, invalid modifiers/reserved fields,
  nonzero key-up text and malformed focus payloads. The historical legacy decoder
  and its accepted short quit/leave forms remain unchanged.

## Queued pointer input

ReadPointerEvent=90 is declared in abi/syscalls.abi; Rust and C syscall numbers
are generated by the existing generator. The number was checked against the
fetched main branch before implementation. `abi/pointer-input` owns the Rust
record codec shared by the kernel and libdunit.

Arguments are an INPUT_MASTER handle, an output address, and an output byte
length. V1 requests use exactly 32 bytes; the separately approved
[modifier snapshot extension](input.md#pointer-modifier-snapshots) adds an
explicit 40-byte request while preserving these v1 bytes. The handle must be
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
consume input. Every successful input acquisition creates an independently
retained handle. Repeated acquisition by the current owner preserves pending
events; closing one handle leaves the others valid. Final close or process
exit/kill releases the owner and discards its queue before a new acquisition
publishes one fresh baseline. Duplication and transfer remain prohibited.

Handle authorization captures the current lease generation while task metadata
is scoped. The syscall pins and validates the output mapping before locking
input, then rechecks that generation through the bounded record copy and pop.
A delayed read cannot consume a later session, including reacquisition by the
same PID. Input callbacks never look up tasks or acquire VM locks.

The queue stays bounded and IRQ-safe. Consecutive pure motion events may be
coalesced only with identical button state and without discarding wheel deltas.
Button changes preserve order. Saturation must be explicit through LOST_BEFORE;
the consumer cancels capture/press state and resynchronises instead of fabricating
a click. The first record on ownership acquisition is also a LOST_BEFORE baseline:
a button already held when the seat is acquired must not synthesize a new press.
Sequence gaps can also mean motion coalescing; only LOST_BEFORE requests resync. Kernel code stores device facts; userspace chooses cancellation policy.

GetMouseState=60 retains its 16-byte record and access policy. Its coordinates,
buttons and accumulated wheel are copied under one input scope; an invalid
output does not drain wheel input. A successful copy clears that accumulator
without consuming queued native events. No gui-v1 wire bytes change. A safe
libdunit wrapper exposes Result<Option<PointerEvent>>.
The compositor consumes one queued record per render iteration and routes
transitions in order, using that record's coordinates. Remaining records stay
queued; pure motion coalesces in the kernel. This bounded budget preserves
render/IPC progress. EAGAIN keeps the last consumed state with zero wheel delta,
rather than reading a newer snapshot ahead of queued events. Events queued
before a live modeset are clamped to the new display bounds. Existing GUI control messages remain the client transport.

## Pointer modifier snapshots

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

## Pointer modes

Approved by the user on 10 November 2026 after review of the compatible public
input and gui-v1 extension. This is application input policy in the compositor;
the kernel exposes original device motion, not window ownership or cursor policy.

### Modes and lifetime

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

### Rust

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

### C

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

### Wire and kernel compatibility

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
