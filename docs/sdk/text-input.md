# Single-line SDK text input (M7.6)

`View::input` adds an editable field to the typed component tree. It uses the
same focus order, native window session, shared DSS and damage pipeline as the
[button family](checkable-controls.md).

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

## Editing and state

- Printable characters replace the current selection; all indices stay on
  UTF-8 scalar boundaries. Native hardware input currently supplies cooked ASCII;
  Unicode values and edits are supported by the pure API. IME and grapheme-aware
  editing are not implemented.
- Left/Right, Home/End and Shift variants move or extend selection. Ctrl+Left/Right
  and Ctrl+Backspace/Delete use whitespace-delimited runs. Ctrl+A selects all.
  Backspace/Delete remove the selection or adjacent scalar/run.
- Ctrl+Z undoes; Ctrl+Y or Ctrl+Shift+Z redoes. Each successful text mutation is
  one action; history is bounded to 64 snapshots. A new edit clears redo.
  Replacing selected text with itself collapses selection without creating a
  model message or undo entry.
- Primary click places the cursor; dragging selects within the client, including
  across another node in the same window. Pointer Leave cancels capture. The
  current native channel does not report pointer modifiers, so native Shift+click
  is not implemented. Double/triple click remains open. Timed
  [selection edge autoscroll](selection-autoscroll.md) supports horizontal drag
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

## Appearance and validation

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
The existing protocol's lack of keyboard-focus-out notification still applies.

Invalid model values are rejected as `UiError::InvalidInput`: control characters,
line separators, values over `max_chars` Unicode scalars or the technical
64 KiB byte bound. Placeholders use the same single-line/byte validation. A
rejected keystroke at a limit leaves text, selection and history unchanged.
Theme validation rejects invalid cursor colors, dimensions and blink duration.
Validation failure follows the existing component contract: no model rollback;
the previous valid view remains and the native session is poisoned.

## Verification and remaining work

```sh
cargo test --manifest-path runtime/dunit-widgets/Cargo.toml
cargo test --manifest-path runtime/dunit-render/Cargo.toml
cargo test --manifest-path runtime/dunit-component/Cargo.toml
python3 tools/test_sdk_gui_fixture.py input --accel kvm
python3 tools/test_sdk_gui_fixture.py input --reuse-image --accel tcg
```

Host tests cover Unicode boundaries, word navigation, selection replacement,
atomic limits, history/read-only/model acceptance and rejection, stable identity,
password/placeholder pixels, scroll/hit testing, clipping, document-order overlap
and incremental rendering against a fresh full render.

The native contract requires 20 markers: focus and real timer-driven cursor
change, editing after an initial UTF-8 value, actual raw Backspace and
Ctrl+Backspace, Shift selection, Home/Delete, undo/redo, long-line scrolling,
password/limit/read-only, resize, pointer selection/replacement, `[INPUT] ok`
and guest exit 0. All guest launches use `tools/qemu_test.py`.

[Multiline fields](multiline-input.md) extend the same editor with LF lines and
a vertical viewport. Double/triple click, native pointer modifiers, grapheme/IME,
clipboard and integration into migrated applications remain open. There is no
local imitation of the system clipboard; Ctrl+C/V depend on M7.7.

Validation on 28 October 2026: 47 component, 22 widget, 36 renderer, 13 native
SDK, 18 style and 16 Python tooling checks passed (152 total). Final native
input runs passed 20/20 markers in KVM and TCG; checkable regression 18/18,
theme regression 7/7 plus exact dark/light/restored framebuffer comparisons,
and GUI smoke 33/33. Forbidden markers were absent. ABI remains in sync at
89 syscalls, 25 errno and 7 rights. The real input screenshot was inspected.
