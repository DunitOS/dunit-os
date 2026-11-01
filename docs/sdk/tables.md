# Controlled tables and in-place rename

`View::table(key, label, model, state, selection_mode, on_change)` uses the same
virtual row selection, viewport, reveal and two-axis scrollbar controller as lists
and trees. `TableModel` owns immutable keyed columns and rows; duplicate or empty
keys, mismatched cell counts and non-finite/non-positive explicit widths fail
construction. Widths omitted by the caller use DSS `table-column-width`.

`TableValue::{Text,Integer,Unsigned}` sorts signed/unsigned numbers numerically
without overflow and text lexically, retaining source order for equal values.
Numbers precede text. Sorting never changes row identity. Header click toggles
ascending/descending order; F4 toggles the current column, Left/Right selects a
column. Existing list selection keys and Shift/Ctrl rules apply to rows. The
model can reject every proposed change. Row/column identity is captured before
committing a focused spinbox, then resolved again after the resulting view update.
A removed, disabled or out-of-modal-scope target cannot receive the stale click.

Only visible rows produce paint payloads. Column text outside the horizontal
viewport is culled. The header stays above the scrolled rows. `table_cell` exposes
role, label, row/column indices and selected/current/disabled/editable metadata.

`.renaming(state, on_event)` enables F2 on list/tree/table. The callback receives
`Begin`, `Change`, `Commit`, `Cancel`; the model owns the draft and its acceptance.
Tables require an editable column. A real shared Input is laid out over the final
scrolled cell/row, clipped to the collection viewport. Opening selects its text;
Enter/Tab commits, Escape cancels, closing restores the collection focus. Rejected
edits preserve editor history/selection. No application painting or hit test is
needed. There can be one active collection editor per view.

Verification: `tests/table.rs` covers numeric extremes/stability, disabled rows,
selection/sort rejection, pointer identity after model rebuild, virtual 10,000-row
scrolling, modal scope, rename lifecycle/history and resize/pixels in both themes.
`tools/sdk_collections_markers.json` drives actual F4/F2/edit/Enter/End/resize in
QEMU and captures the editor and table.
