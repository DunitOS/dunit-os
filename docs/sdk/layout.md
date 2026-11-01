# State-aware SDK layout

`View::row`, `column`, `grid`, `stack` and `scroll` use the shared
`dunit_ui::layout` engine. Typed `Attrs` provide the baseline. A matching DSS
layout property overrides that baseline after classes and widget states have
been resolved, before measurement and arrangement. Thus `:focus`, `:active`,
`:disabled`, `:checked` and other supported states can change geometry as well
as paint. Event behavior remains in the control implementation.

Supported DSS properties:

| Property | Values |
| --- | --- |
| `width`, `height` | nonnegative pixels, `0%`–`100%`, `auto`, `grow`/`fill` |
| `min-width`, `min-height`, `max-width`, `max-height` | nonnegative pixels, `none` to clear a typed bound |
| `flex-grow` | nonnegative weight along the parent flex line |
| `gap` | nonnegative distance between children |
| `justify-content` | `start`, `end`, `center`, `stretch`, `space-between`, `space-around` |
| `align-items` | the same alignment keywords |
| `grid-columns` | integer `1`–`4096` |

Numeric geometry is finite and bounded to 65536 pixels. Negative sizes,
invalid units/keywords and nonintegral grid counts are errors. Theme parsing
rejects invalid declarations and contradictory bounds within a rule. The
resolved combination of typed attributes and matching rules is checked again
before layout; `min > max` returns `InvalidLayout` without replacing the last
rendered frame.

Row and column children share positive free space by grow weight; each final
size obeys its own min/max constraints. Explicit `flex-grow: 0` disables the
implicit main-axis weight of `grow` sizing while retaining cross-axis fill.
Explicit cross-axis sizes take priority
over stretch. Alignment positions the resulting constrained rectangle. The
root always fills the renderer viewport, independent of its own requested
size.

Grid columns have equal slots; rows take the largest measured child height in
that row. Child `auto`/`grow` fills its slot, while explicit or percentage sizes
resolve within the slot. Each child is constrained by min/max, then placed
using the grid's horizontal `justify-content` and vertical `align-items`.
Gaps remain between slots even when a child is smaller than its slot.

Layout changes are immediate. Paint transitions do not animate geometry.
Scroll controllers, clipping, caret placement and hit testing consume the final
arranged rectangles, so a state change does not create a second geometry model.

`runtime/dunit-component/tests/layout_style.rs` covers state-driven reflow,
resize/percentages, flex constraints/alignment, grid slots and theme changes,
invalid candidates, failed-render frame preservation and fresh-render pixel
equality. `runtime/dunit-ui/tests/layout_tests.rs` also checks the grid contract
without fonts or widgets.
