# Text multi-click selection (M7.6)

Input and Textarea support primary single, double and triple clicks through the
same pure `Application::pointer_text_at` path used by the native adapter. A
single click places the caret, a double click selects the whitespace/non-whitespace
run, and a triple click selects the logical line including its LF separator.
The fourth click starts a new single-click sequence. Empty final lines remain
valid caret positions. This selection does not change the text or create undo
history; replacing it is one ordinary controlled edit.

DSS supplies `input-multiclick-interval` (duration) and
`input-multiclick-distance` (pixels). Both dark and light profiles define these
through named tokens. An interval of zero disables multi-click classification;
a missing property preserves ordinary single-click selection. A click streak
requires the same stable field key, unchanged text, unchanged policy, monotonic
time and Euclidean distance within the configured threshold. Keyboard input,
wheel, resize, window leave and interactions with another control cancel it.

Dragging after a double/triple click retains the original word/line as its
anchor and extends by whole words/lines, including when reversing direction.
The stationary edge-autoscroll timer uses the same anchor. Release ends capture;
focus loss, disable, removal or externally replaced text cancels capture.
Read-only fields still support selection. All indices normalize to valid UTF-8
boundaries; this does not implement grapheme segmentation or Unicode shaping.
Native Shift+click extends the existing anchor; Ctrl+click selects a word and
Ctrl+Shift+click extends to its word boundary. The compositor forwards the
[enqueue-time modifier snapshot](pointer-modifier-snapshot-proposal.md), including
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
