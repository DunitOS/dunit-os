# M7.6 widget acceptance and dependency gates

This matrix records the implementation and verification scope for the complete
SDK widget set. It separates the per-widget acceptance criteria from the
roadmap's cross-milestone Definition of Done. It does not declare the complete
milestone closed while those dependencies remain open.

## Common contract

Widgets use the same typed `Component::update`/`view`, model-owned values,
stable keys, shared DSS states, renderer geometry and native window adapter.
Rebuilding or reordering a stable kind preserves valid transient state; removal,
replacement and disable transitions invalidate it. Tests check controlled
rejection rather than retaining an independent authoritative widget value.
Shared focus/routing enforces modal scope, roving groups and capture cancellation.

The roadmap requires state transitions, mouse and keyboard behavior, focus,
role/name metadata and resize behavior for each widget. Passive text/indicators
deliberately do not become focus stops or dispatch model actions. Plain `Text`
exposes `statictext` with its current content as its name; pure layout containers
remain roleless. The metadata APIs describe controls for a future accessibility
bridge; they do not claim a running screen reader or system IME.

## Family matrix

Host paths below are relative to `runtime/dunit-component/tests/` unless stated
otherwise. Native fixture names mean
`python3 tools/test_sdk_gui_fixture.py NAME --accel tcg`, which invokes the sole
OS harness, `tools/qemu_test.py`. Existing marker contracts stay authoritative;
the matrix does not replace their assertions with checklist entries.

| Family | Behavior and state coverage | Host contract | Native contract |
| --- | --- | --- | --- |
| Text and containers | Current role/name, passive focus behavior, message-driven content, retained pixels | `components.rs`, `layout_style.rs` | `theme` |
| Push buttons | Matching pointer press/release, cancellation, Tab, held Space/Enter, one activation on break | `buttons.rs`, `input_lifecycle.rs` | `buttons`, `lifecycle` |
| Toggle, checkbox, radio, switch | Model-owned checked/pressed metadata, keyboard/pointer activation, radio roving and disabled skip | `checkable.rs`, `input_lifecycle.rs` | `checks`, `lifecycle` |
| Input | Grapheme-safe editing/selection/history, placeholder/password/read-only/limits, model rejection, horizontal viewport | `input.rs`, `click_selection.rs`, `input_lifecycle.rs` | `input`, `collections`; pointer modifier gate below |
| Textarea | LF editing, vertical navigation, visual soft wrap/affinity, selection and shared scrollbars | `multiline.rs`, `soft_wrap.rs`, `input_scroll.rs` | `multiline`, `collections`, `scroll_xy` |
| Text gestures | Single/double/triple clicks, word/line drag, Shift/Ctrl anchors, stationary edge autoscroll | `click_selection.rs`, `input_lifecycle.rs`, `input_scroll.rs` | `collections`, `autoscroll`; pointer modifier gate below |
| Scroll areas | Both axes, nested wheel bubbling, focus reveal, track/thumb capture, clamp, resize, timed auto-hide | `scroll.rs`, `scroll_xy.rs`, `scroll_visibility.rs` | `scroll`, `scroll_xy`, `autoscroll` |
| Sliders | Horizontal/vertical geometry, integer bounds/steps, keyboard/track/drag, controlled rejection and cancellation | `sliders.rs`, `vertical_sliders.rs` | `slider`, `vertical_slider` |
| Spinbox | Numeric draft, commit/cancel, bounds/steps, buttons, keyboard, read-only/disabled and rejected edits | `spinboxes.rs`, `focus_order.rs` | `spinbox` |
| Progress and busy | Range metadata, passive input, timed progress pixels, pause/disabled/reduced-motion/clipping limits | `progress.rs` | `progress` |
| Select and menus | Controlled select, nested action menus/separators, menu-bar roving/mnemonics, context trigger, pointer/keyboard capture | `menus.rs`, `input_lifecycle.rs` | `menus`, `lifecycle` |
| Tooltips | Delayed appearance, clipping/wrapping, noninteractive metadata, input/focus/rebuild cancellation | `menus.rs`, `input_lifecycle.rs` | `menus`, `lifecycle` |
| Tabs | Controlled selected page, disabled skip, roving headers/Ctrl+Tab, only active page mounted | `workspace.rs` | `workspace` |
| Splitter | Both axes, shared paint/hit geometry, keyboard/drag, pane minima, resize and controlled rejection | `workspace.rs` | `workspace` |
| Toolbar and status | Single roving toolbar stop, Home/End, stable remembered focus; passive named status | `workspace.rs`, `focus_order.rs` | `workspace`, `collections` |
| Dialog/modal/message box | Client-local focus scope, default/cancel, opener restoration, disabled background, bounded layout | `workspace.rs`, `focus_order.rs` | `workspace` |
| List | Single/multiple/range selection, modifiers, stable row keys, empty/disabled/model rejection, virtual rows | `list.rs` | `list`; pointer modifier gate below |
| Tree | Validated hierarchy, expansion/collapse/promotion, visible-row selection and tree metadata | `tree.rs` | `tree`; pointer modifier gate below |
| Table, sort and rename | Stable typed sorting, columns/cell metadata, virtual paint, F2 draft/commit/cancel, stale-target cancellation | `table.rs` | `collections`; pointer modifier gate below |
| File dialog | Open/Save/Directory, native listing, navigation/filter/history, actual metadata, overwrite/cancel without writes | `file_dialog.rs` | `collections` |
| Shared focus and routing | Stable Tab order, scoped mnemonics, capture/bubble once, prevent/default, revalidation after callbacks | `focus_order.rs`, `routing.rs`, `input_lifecycle.rs` | `collections`, `workspace`, `lifecycle` |
| DSS layout and appearance | State-aware flex/grid constraints, clipping, resize, rollback, light/dark retained-vs-fresh pixels | `layout_style.rs`, `themes.rs`, `effects.rs`, `backdrop.rs` | `theme` |

The list/tree/table host workloads use 10,000 entries and assert viewport-bounded
materialized rows. The native `collections` showcase asserts fewer than 30 visible
rows in its 10,000-row table and emits `[WIDGETS] ok` only after its routing,
editing, sorting, picker, gesture and resize assertions have completed.

## Final verification status

Final source acceptance on 2 November 2026: **650 host tests**, **19 Python
harness tests**, and **20 native QEMU TCG contracts** passed. The native contracts
satisfied **433 required marker checks** with no forbidden hits under each
contract's original scope. Dark, light and restored theme framebuffer pixels
matched all three references exactly. [Machine-readable case results](m7-6-results.json)
list every contract and marker count. Historical increments remain documented in
[m7-6-validation.md](m7-6-validation.md).

The final native set is buttons, checks, input, multiline, scroll, scroll_xy,
autoscroll, slider, vertical_slider, spinbox, progress, menus, workspace, list,
tree, collections, lifecycle, theme, modifiers and pointer queue. The new
`modifiers` fixture checks 20 real Shift/Ctrl/Ctrl+Shift gestures across Input,
Textarea, List, Tree and Table. Text/undo remain unchanged, selections match exact
expected ranges/sets, and all three 10,000-item collections keep bounded rows.
The lifecycle fixture now also holds Shift while clicking the legacy peer;
legacy pointer bytes remain unchanged.

The menus scenario initially raced native Enter release before the popup had
reopened. It now requires an additional observed-popup marker before its outside
click. All earlier markers and the apply==0/no-click-through assertion remain;
the strengthened full contract passed. This was a test input sequencing error,
not a removed failure gate. No tests or marker assertions were disabled.

A combined overlay exceeded the existing 128 MiB installer FAT payload. Final
native contracts therefore used individual cached builds through the standard
wrapper. The image/partition format and payload checks remain unchanged. Existing
imported-module warnings remain in the standalone DWM host harness; changed crate
suites pass without warnings. ABI generation verifies 90 syscalls, 25 errno and
7 rights in sync.

The newly approved [pointer snapshot](pointer-modifier-snapshot-proposal.md)
preserves the exact 32-byte v1 record and adds an explicitly requested 40-byte
record carrying modifiers sampled at enqueue. The passing host/native contracts establish:

- Strict v1/v2 codecs, reserved bits and byte compatibility.
- No motion coalescing across modifier changes; wheel/button order and loss
  resynchronization remain intact.
- Failed user copies retain the queued record; rights checks remain unchanged.
- A delayed reader observes Shift/Ctrl on an earlier pointer event even after
  those keys have been released, through kernel, compositor and SDK selection.
- Legacy clients still receive their earlier pointer bytes and event kinds.

Full host suites and the selected native contracts passed on the final source
state. Test/dev debug information and incremental compilation may be disabled to
save workspace space; assertions and marker gates must remain enabled. All QEMU
builds/runs are serialized. Screenshots are useful artifacts, while state,
pixel-reference and event-count assertions determine acceptance.

## Explicit dependency and future-contract gates

The M7.6 roadmap's original per-widget criteria are the matrix above. Its stated
Definition of Done additionally says migrated applications use these widgets
instead of manual drawing/hit testing. That is the explicit M7.11 dependency;
it is not fulfilled merely by a standalone widget showcase. The input-field
clipboard dependency belongs to the system clipboard/DnD work in M7.7. Neither
dependency is replaced with a local imitation or silently removed here.

Cross-surface popup roles, global outside-click/grab semantics and a real system
IME require their own system/protocol contracts. Current menus are deliberately
client-local; negotiated focus loss dismisses them without claiming observation
of every global outside click. Current text editing supports Unicode values and
grapheme boundaries, while hardware text input still uses the documented cooked
seat path. The requirement for role/name metadata “для будущих a11y/IME” is not
an implementation of an IME service. This document does not claim those future
contracts are completed or change their approval requirements.
