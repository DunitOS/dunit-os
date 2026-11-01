# Textarea soft wrap

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
