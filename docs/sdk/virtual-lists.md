# Virtual lists (M7.6, collection foundation)

`View::list` adds a controlled, uniform-row-height collection to the typed SDK.
The component owns row data and stable-key selection. A single UI node represents
the collection: it does not create a View, layout box or widget per model row.
The renderer snapshots and rasterizes only rows intersecting the viewport and ancestor/display clips.
Total model storage and selection validation still scale with model length;
this is rendering virtualization, not an asynchronous/paged data provider.

```rust
View::list("files", "Documents", rows, selection,
    SelectionMode::Multiple, Msg::Selection)
    .layout(Attrs {
        width: Sizing::Grow(1.),
        height: Sizing::Grow(1.),
        ..Default::default()
    })
```

Rows are `ListItem::new(stable_key, label).disabled(bool)`. Keys must be nonempty
and unique inside the collection; they are independent of global UI node keys.
`ListSelection` contains a set of selected keys, a current key and a range anchor.
The model accepts/rejects proposed state through `fn(ListSelection) -> Message`.
No callback is dispatched for an unchanged proposal. Invalid or disabled keys
in selection fail view validation. After removing/disabling rows, the model can
call `selection.retain(&rows)` to remove stale keys. Sorting/reordering the model
preserves selection identity. No implicit sorting or rename callback is claimed.

## Input and focus

The whole list is one Tab stop. Empty/all-disabled lists remain focusable but
have no eligible selection. Up/Down skip disabled rows without wrapping;
Home/End choose endpoints; PageUp/PageDown move by the visible page's enabled
row count minus one. At endpoints navigation is consumed without duplicate
messages. Single mode always selects just the current row.

Multiple mode supports:

- plain navigation/click: replace selection and establish a range anchor;
- Shift navigation: select an inclusive range from the anchor, excluding disabled rows;
- Ctrl navigation: move current without changing selection;
- Space: toggle current and establish a new anchor;
- Ctrl+Shift navigation: add a range to the existing selection;
- Ctrl+A: select all enabled rows.

A primary pointer Down selects the hit row immediately; release does not
redispatch. Blank space and disabled rows preserve selection. Pointer geometry
comes from the last painted viewport and the same row-height/scroll offset as
painting. The native pointer stream currently has no modifier bits, so native
pointer selection is plain selection; keyboard Shift/Ctrl and the explicit
`Application::list_pointer(..., modifiers)` API support ranges/multiple selection.
Pointer range dragging, touch, typeahead and double-click activation are outside
this slice. The list is passive with respect to Enter; applications may bind it.

Shared focus and modal scopes apply: a dialog blocks base collection input and
closing restores the list opener. Public `list_command(command, modifiers, page)`
lets non-native adapters use the same behavior; `Application::navigate` also
routes plain Up/Down. Root shortcuts retain existing routing after list-specific
keys. There are no new syscalls or gui-v1 fields.

## Geometry, scrolling and painting

`ListGeometry` shares `visible()`, `row_at()` and `page()` between adapters,
paint snapshots and tests. A partially visible first/last row is included and
clipped. Current changes/focus entry/resize reveal the current row. Wheel and
scrollbar gestures retain manual offset until another reveal trigger. The
existing ScrollAxis controller owns clamping, wheel bubbling, thumb/track input,
auto-hidden bars and a reserved gutter. Shrinking the model clamps the offset.
Disabled lists stop input and show their content at the start.

DSS controls `list-row-height`, text/font/padding, border/focus state,
`list-selection-background`, `list-selection-color`, `list-disabled-color`,
`list-current-color`, and the shared scrollbar properties. Both SDK profiles
provide row height. Nonpositive/nonfinite row height fails layout; invalid theme
values fail transactional Theme parsing. Long row labels clip horizontally;
variable heights, word wrap and horizontal scrolling are not implemented here.

Snapshots store visible labels, indices, selection/current/disabled state and
geometry. Retained invalidation includes them, so selection/scroll/data changes
repaint correctly; unchanged frames do not repaint. `Renderer::list_visible_rows`
and `list_geometry` (also exposed by ComponentWindow) allow observing this bound.
`View` supplies listbox role, supplied label, `accessibility_multiselectable`,
`list_items` and `list_selection` for a future accessibility bridge, including
unmounted/offscreen row data. There is no platform accessibility service yet.

## Verification

Fourteen host scenarios cover selection modes/ranges, modifier behavior,
reorder/removal, disabled/empty boundaries, ancestor clipping and scrollbar capture, model rejection, modal and popup isolation,
validation, shared hit geometry, 10,000-row virtualization/reveal/wheel/clamp,
and retained-vs-fresh pixels after resize in both profiles.

```sh
cargo test --manifest-path runtime/dunit-component/Cargo.toml --test list
python3 tools/test_sdk_gui_fixture.py list --accel tcg
```

The native fixture verifies actual Tab/arrows/Shift/Ctrl/Space/Ctrl+A,
End/PageUp reveal, pointer hit testing, wheel, stable keys on reorder, window
resize, shrink and rejected proposals. It captures selection/end/shrink frames.
[Tree views](virtual-trees.md), [tables and sorting](tables.md), and controlled
in-place renaming now share this collection foundation. Their host/native
contracts and the current final verification status are listed in the
[M7.6 acceptance matrix](m7-6-completion.md).

## Kernel entry regression found by the native workload

The 10,000-row workload exposed intermittent kernel instruction faults at
RIP/address 0. Debugger capture showed kernel RFLAGS `0x10406`, including DF=1,
and a damaged scheduler return frame. The interrupt stub entered Rust without
clearing the interrupted direction flag; compiler-builtins' backward memmove
sets DF temporarily. A timer interrupt inside that copy could therefore enter
kernel C/Rust string operations with the wrong direction.

The HAL now executes CLD after hardware has saved the interrupted flags and
before calling Rust. SYSCALL's SFMASK clears IF and DF; saved user R11/IRET flags
are unchanged. No syscall number, wire layout or privilege policy changes.
The existing preemption child explicitly holds DF during CPU-bound work, checks
its preservation alongside XMM state, and performs a syscall with DF set. Boot
smoke builds assert that both kernel entry paths see DF clear, require an actual
user DF-set interrupt, and emit `[DF-TEST] syscall+irq flags preserved` only when
the preemption child and scheduling checks also succeed. Both boot and list
marker contracts require this result.

Two fixture issues were also corrected without removing forbidden markers:
PageUp checks its exact target and visibility rather than requiring unnecessary
scrolling; pointer Down and Up use separate QMP reports, with the observed Down
result between them. Debugger runs are diagnostics, not acceptance runs.

The kernel entry contract and its regression are documented in
[kernel-entry-flags.md](../architecture/kernel-entry-flags.md).
