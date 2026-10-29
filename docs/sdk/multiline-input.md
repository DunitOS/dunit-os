# Multiline SDK text input (M7.6)

`View::text_area(key, label, value, on_change, options)` extends the
[single-line editor](text-input.md) using the same controlled model, stable
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

## Editing and viewport

Enter replaces a selection with LF. Backspace/Delete join lines when removing
a separator. Selection, replacement and undo/redo include LF bytes. Home/End
move to the logical line edges; Ctrl+Home/End move to document edges. Up/Down
retain the preferred scalar column across shorter and empty lines; Shift
extends the selection. Horizontal movement, pointer placement and edits reset
the preferred column. This is scalar navigation; proportional font widths and
grapheme clusters do not define the preferred column.

Rendering uses explicit LF lines without soft wrapping. The caret is revealed
on editing, navigation, focus, resize and font geometry changes. Pointer hits
use both axes and the current scroll offsets. Drag selection crosses lines;
selection and glyphs are clipped to the content box. Visible glyph work is
bounded using font-wide ink bounds, including overhang from adjacent logical
rows. Pixel-reference tests cover negative bearings, positive overhang, tall
accents and deep descenders. Timed drag autoscroll and
scrollbars are not implemented.

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

## Verification

The native fixture is `tools/fixtures/sdk_multiline.rs`, driven by
`tools/sdk_multiline_markers.json` through `tools/qemu_test.py`:

```sh
python3 tools/test_sdk_gui_fixture.py multiline --accel kvm
python3 tools/test_sdk_gui_fixture.py multiline --reuse-image --accel tcg
```

Its 19 required markers cover focus, LF editing, cross-line replacement,
selection history, UTF-8 boundaries, caret scrolling, real mouse wheel,
viewport stability over blinking, document Home, pointer selection across
lines, resize, `[MULTILINE] ok` and guest exit 0. Forbidden markers reject
kernel/SDK panic and application/compositor failures. Host tests also compare
incremental pixels with a fresh render and cover mode changes and limits.

The shared harness supports literal `type_text` actions with `submit: false`;
the default still submits terminal commands with Enter. Host tests verify
both paths. GUI terminal wheel consumption follows the same documented native
sign convention, with regression coverage for backlog direction and clamping.

Soft wrapping, grapheme/IME, double/triple click, native pointer modifiers,
clipboard (M7.7), generic scroll containers/bars and application migration
remain open roadmap work.

Validation on 29 October 2026: 55 component, 28 widget, 43 renderer, 10 text,
13 native SDK, 18 style, 75 protocol and 19 Python tooling checks passed.
All 111 DWM host checks passed, including terminal wheel direction/clamping.
Final multiline runs passed 19/19 markers in both KVM and TCG, with no
forbidden markers and guest exit 0. Regressions passed single-line input
20/20, checkables 18/18, theme 7/7 with exact dark/light/restored framebuffer
comparisons, and GUI smoke 33/33. ABI generation remains in sync (89 syscalls,
25 errno, 7 rights). The native multiline screenshot was inspected.
