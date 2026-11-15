# Components, layout and windows

Enable the `gui` feature of `dunit` for typed components and owned native windows.
The component model owns application data; the SDK owns layout, input state,
private pixels and buffer publication.

## Components and rendering

Enable the `gui` feature of `dunit` to use `dunit::ui`. A root component owns
application state, changes it in `update`, and describes its interface in
`view`. The pure `dunit_component` crate runs the same code in host tests.

```rust
extern crate alloc;
use alloc::{format, vec};
use dunit::ui::{Application, Component, Key, View};

struct Counter(u32);
#[derive(Clone)]
enum Msg { Increment }

impl Component for Counter {
    type Message = Msg;
    fn update(&mut self, _: Msg) { self.0 += 1; }
    fn view(&self) -> View<Msg> {
        View::column("root", vec![
            View::text("title", "Counter"),
            View::text("value", format!("{}", self.0)),
        ]).on_key(Key::plain(' '), Msg::Increment)
    }
}

let mut app = Application::new(Counter(0)).unwrap();
app.key(Key::plain(' ')).unwrap();
assert_eq!(app.component().0, 1);
assert_eq!(app.view().children()[1].content(), Some("1"));
```

Views contain text, [push buttons](widgets.md#push-buttons), [checkable controls](widgets.md#checkable-controls),
[single-line fields](input.md#text-input), [multiline fields](input.md#multiline-input), [scroll regions](layout-window.md#scroll-areas) and Row/Column/Stack/Grid
containers. Text is stored directly;
quotes, braces and Unicode do not need markup escaping. Every node has a
nonempty unique key, also used as its DSS ID. The current adapter hosts one root
component. Nested component/widget lifecycle is not implemented.

`View::class(name)` selects a DSS appearance class from the model. The renderer
uses these classes during cascade resolution. Supported appearance changes can
use [timed DSS transitions](themes.md#appearance-transitions). Buttons and fields supply automatic interaction
states to the same cascade.

`Application` exposes immutable component/view access. Dispatching a message
calls `update`, builds and validates the next view, then replaces the previous
one. Duplicate keys, duplicate shortcuts, nested shortcuts, invalid dimensions,
invalid DSS class names and excessive depth/node count are errors. Propagate
validation failures: a model can already have consumed the message, while its previous valid view is
retained for diagnostics. There is no model rollback.

`on_key` binds application shortcuts on the root view. A binding matches the
cooked character and native modifier mask exactly. `Key::plain` uses no
modifiers. Focused buttons consume Tab/Shift+Tab and plain Space/Enter before
root shortcuts. Focused fields consume text/editing keys before root shortcuts;
Enter may still reach the root shortcut. Child bindings are rejected
instead of being ignored.

### Rendering and native windows

`Renderer::new(font, cascade, background)` receives a configured font, DSS
cascade and clear color. It builds the existing `dunit_ui` tree directly, uses
the existing widget measurer/layout and paints through `dunit_render`. Layout
attributes such as width, height and spacing are set through `Attrs` with
`View::layout`. Container DSS padding overrides attribute padding when present;
text padding participates in both measurement and painting. All three layers
use the same DSS padding shorthand decoder.

The renderer retains a private CPU canvas. For an unchanged view, the reported
damage is empty and no pixels are repainted. Text, style or geometry changes
invalidate the bounding rectangle of old and new node boxes. Structural edits,
reordering, resize and a changed style stack invalidate the entire canvas.
Multiple changes currently share one conservative bounding rectangle.

Painting clips every node and its descendants to their layout boxes, including
glyph overhang. A dirty region is cleared and repainted in document order, so
shortened text leaves no old glyphs and overlapping translucent nodes do not
accumulate alpha. Empty intersections skip painting. Host tests compare
incremental pixels with a fresh full render. `RenderStats::damaged_pixels`
counts the distinct pixels covered by CPU invalidation, not individual stores,
compositor work or frame duration.

`ComponentWindow::open(width, height, component, renderer)` presents the first
frame. `update(message)` updates and presents the component; `next_event`
dispatches root keyboard bindings and handles resize before returning the
event. `component`, `view`, `size` and `last_render` provide read-only access.
Close the window on `WindowEvent::CloseRequested`. The existing wait-set and
single-session restrictions still apply; see [window.md](layout-window.md#native-windows).

Damage now reaches the native compositor through the existing gui-v1 ATTACH
rectangles. Presentation keeps two shared backings at most. A backing becomes
writable again only after BUFFER_RELEASE and completed DESTROY_BUFFER. Before
reuse, the adapter repairs the union of the previous published damage and the
current damage from its private canvas. Fresh backings and new geometry need a
full copy. Every published backing therefore contains a complete valid image;
damage is not permission to leave the rest uninitialized. Unchanged views skip
copying, import and commit entirely.

The compositor snapshots only accepted committed images, honors stride/offset,
and copies only declared damage into its CPU texture. Imports and pending
attachments cannot change the visible image. A stable scene with opaque,
unscaled, unobscured content patches updates only the affected desktop pixels
and framebuffer rectangle. Changes to geometry, stacking, theme, overlays or
animations use the full painter, as do transparent, scaled or occluded patches.
Legacy clients with empty damage retain conservative continuous full updates
until they migrate; this preserves their existing in-place writing behaviour.

The counters describe different work: `last_render().damaged_pixels` measures
CPU invalidation; `last_uploaded_pixels()` includes shared-buffer repair;
`[DAMAGE]` reports actual texture copies, desktop composition and presentation.
These counts can differ, especially for a fresh backing or full-scene fallback.

`Window::try_draw` permits a fallible painting callback. Failure happens before
capability transfer/import/publication and poisons the session. Concurrent
configure can retry the callback at new geometry. Component rendering failures
and invalid updates also poison `ComponentWindow`; propagate them and close or
drop the session.

System Monitor uses `ComponentWindow` and a typed `View`. Its DUI string
builder and repeated parse/layout/paint code are removed; counters change only
through a refresh message. It uses `open_themed` and the shared
[SDK token profiles](themes.md#themes), including live reload, without a private DSS
file or font loader. The generated GUI starter uses the same component path.
Controls retain valid interaction state by stable key and kind. Nested component lifecycle remains separate work. See
[buttons](widgets.md#push-buttons) and [text input](input.md#text-input).

## Layout

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

## Native windows

Enable `dunit = { path = "…/runtime/dunit", features = ["gui"] }`, or create a
project with `cargo dunit new demo --kind gui`. The generated component uses
the [configured SDK theme](themes.md#themes), with no application stylesheet. The
lower-level canvas API remains available for custom rendering.

```rust
use dunit::time::Duration;
use dunit::window::{Window, WindowEvent};

// The compositor launches this process from its installed manifest.
let mut window = Window::open(480, 320)?;
window.draw(|frame| {
    // XRGB8888 words. Obtain colors from the application/theme configuration.
    frame.pixels_mut().fill(background);
})?;
loop {
    match window.next_event(Duration::from_secs(60))? {
        Some(WindowEvent::CloseRequested) => break,
        Some(WindowEvent::Resized { .. }) => {
            window.draw(|frame| frame.pixels_mut().fill(background))?;
        }
        _ => {}
    }
}
window.close()?;
```

The visible caption and application identity come from `dunit.toml` through the
compositor's registry. The application does not duplicate them in its source.
The existing live binding does not apply per-surface SET_TITLE/SET_APP_ID to
its desktop model; this initial helper therefore does not expose those as
working dynamic window properties.

`Window` owns bootstrap, HELLO, surface/configure acknowledgments,
shared capabilities, imports, commits and frame callbacks. `next_event` blocks
through the native wait-set; it reports legacy INP1 input as `Input`, protocol
input as `Protocol` (match the exported `ProtocolEvent` enum), resize as
`Resized`, and either close form as
`CloseRequested`. The strict server-event decoder lives in `gui-v1`, beside its
encoder; application code never reads wire offsets.

`draw` obtains an unpublished buffer and lends its pixels to the callback.
The writable borrow ends before a read-only capability is transferred. A
successful return requires the matching PRESENTED frame callback, not just a
successful IPC send. The previous object is destroyed after BUFFER_RELEASE;
only then can its local mapping enter the bounded two-backing reuse pool.
Published buffers are never exposed as writable slices. `close` completes
DISCONNECT; Drop attempts a nonblocking disconnect and releases both local
backings, including on errors.

Publication queues the existing CAP1, IMPORT_BUFFER, ATTACH_BUFFER and COMMIT
messages in their required FIFO order before waiting for the three matching
RESULTs. This removes two application/server round-trip gaps without adding a
wire operation or another frame in flight. `draw` still waits for matching
FrameDone, observes BUFFER_RELEASE and waits for DESTROY_RESULT before reusing
the old backing. A concurrent CONFIGURE is retained while reading replies; a
stale COMMIT follows the same bounded surface-recovery path described below.

`try_draw_damage` accepts a fallible callback returning `Some(DamageRect)` for
changed pixels or `None` for an unchanged frame. The latter skips publication
unless a new surface needs its first full image. Rectangles must be nonempty
and inside the configured size. The whole backing must still be valid: use
`Frame::copy_pixels` with a complete private canvas to repair both the previous
frame's damage and the new region in a retired backing. Fresh storage requires
a full copy. `Frame::requires_full_frame` also forces full damage after surface
recovery or geometry changes. `draw` and `try_draw` publish explicit full damage.
The wire format and existing empty-damage semantics are unchanged.
The compositor completes first-frame notification before sending queued input
or a policy resize (session restore, window rule or desktop self-test). This keeps
legacy clients alive while callbacks follow actual desktop composition.
Callback-free commits also release queued policy after composition.

`Window` supports one window/session per process and owns the
process IPC inbox. It must not be mixed with independent IPC receivers; it is
neither Send nor Sync. Reopening returns EBUSY, including after a failed open.
It uses the compositor's existing 8-byte launch bootstrap: the claimed PID must
match the kernel-authenticated sender, and all later traffic is filtered to
that PID. This preserves the existing launch trust contract; it does not add a
new authenticated service-discovery mechanism.

Dimensions are checked against gui-v1 bounds before allocation. Headers,
versions, serials, payload lengths and reserved fields are checked. Each
transaction has a five-second monotonic deadline;
foreign traffic cannot extend it. Deferred protocol/input/release storage is
bounded. Timeout returns EAGAIN, malformed/protocol failure EIO, and native
errors retain their errno. A draw/event failure poisons the session; close/drop
it rather than continuing with partially accepted state. In particular, a
concurrent CONFIGURE can invalidate an in-flight ACK/COMMIT. For a nonfatal
BAD_SERIAL accompanied by a newer CONFIGURE, the helper uses the existing
DESTROY_SURFACE cleanup barrier, destroys released buffer objects and creates a
fresh surface on the same connection/PID at the latest geometry. The desktop
retains that PID's placement and maximize/restore state. Painting is retried;
**the callback may run more than once**, so keep application state changes
outside it. Frame attempts and surface rebuilds are bounded separately (eight
attempts each, with monotonic retry budgets);
persistent failure is returned, never reported as a presented frame. There is
no wire-format or syscall ABI change. Scale 1 and XRGB8888
are supported initially. Damage and [typed component diffing](layout-window.md#components-and-rendering)
are implemented. The raw Window path supports scale 1 and XRGB8888. Multiwindow sessions and reconnect remain unsupported; interactive controls use ComponentWindow.

System Monitor uses the [typed component adapter](layout-window.md#components-and-rendering) over this
API, including live maximize/restore, timed
refresh and close. Its app code contains no raw IPC, capability management,
pointers, `_start` or panic handler. All seven ordinary GUI window clients use the native SDK.

## Scroll areas

`View::scroll(key, label, children)` lays out children vertically at natural
height inside a bounded viewport. It is a focusable accessibility `region`
with the supplied label. Its default `height: Grow(1)` shares the remaining
column height with other growing children. Fixed and percentage heights are
also supported. The default remains vertical for compatibility;
`.scroll_axes(ScrollAxes::Horizontal)` or `.scroll_axes(ScrollAxes::Both)` enables
other axes explicitly. Children still stack vertically, and can contain wide
rows, grids or other content. The layout engine derives flex weight from typed main-axis
`Sizing::Grow` when there is no explicit `Attrs::grow` weight.

```rust
View::scroll("actions", "Actions", rows).scroll_axes(ScrollAxes::Both)
```

Viewport position is transient SDK state. The application owns child contents
and messages; it does not duplicate offset, thumb geometry or hit testing.
Offsets follow the stable key while the node remains a Scroll. Reorder, resize
and theme reload retain the offset, clamped to the new content extent.
Removal, retype or disabled state discards it. Disabling a scroll viewport
blocks its own scrolling; it does not recursively disable its children.

### Behavior

- Wheel events bubble from the deepest hit node to the nearest viewport that
  can move. At a boundary they continue to an outer viewport. A textarea gets
  first chance to scroll its own document. Negative native deltas move upward.
  Without a pointer target the native adapter uses the focused node.
- Tab traverses the region and its controls. A new focused descendant is
  revealed, from inner viewports outward. Resizing its viewport also reveals
  it. Manual scrolling survives no-op model rebuilds and color-only theme
  reloads. The separate negotiated [window lifecycle](input.md#keyboard-and-focus-lifecycle)
  now cancels scrollbar capture on keyboard focus-out without changing offsets;
  legacy clients retain their existing notification limits.
- Up/Down move vertically by the themed step; Left/Right move horizontally.
  PageUp/PageDown page vertically; Home/End reach the vertical limits. Axis-aware
  pure APIs support line/page/end commands on either axis. Native text editing and radio navigation have priority;
  remaining raw scroll keys bubble through the focused node's ancestors. The
  current native wheel channel reports only vertical deltas; horizontal wheel
  routing is available through the pure axis-aware API. No gui-v1 format changed.
- Clicking a track pages toward the pointer. Dragging the thumb preserves the
  grab point, clamps at both ends and continues outside the track within the
  client. Pointer Leave permanently cancels capture. Text-selection capture
  crossing a scrollbar is preserved. Content removal, disabled state or a
  disappearing scrollbar also cancels thumb capture.
- Paint and hit testing use the same translated child rectangles and ancestor
  viewport clipping. Scrollbars paint after their content; later siblings keep
  normal paint/hit precedence. The last painted snapshot stays immutable while
  interaction updates the controller, so incremental diff sees the change.

`dunit_widgets::ScrollAxis` supplies finite extent/offset handling, minimal
reveal, line/page/end commands, proportional/minimum thumb geometry and
anchored drag. The text renderer uses the same axis for fitting, wheel and scrollbar
clamping; it retains logical LF editing and caret behavior. Clicking or dragging
a text scrollbar, including the reserved corner or hidden-bar gutter, focuses
the field without placing the caret or starting text selection. The current
viewport is pinned before scrollbar focus, so its thumb cannot jump to the
old caret. Undo/redo history is unaffected; the next text caret action reveals
the insertion point. Blink/no-op updates retain manual viewport offsets.

### Appearance and limits

Shared dark/light DSS defines `scroll-step` (content pixels for Scroll),
`scrollbar-width`, `scrollbar-min-thumb`, `scrollbar-radius`, `scrollbar-track`
and `scrollbar-thumb`. `scrollbar-hide-delay` is a duration from 0 to 60000ms;
zero keeps overflowing bars visible. Dark/light profiles use 1200ms.
Dimensions/steps must be finite and within 0..1024;
colors must be valid colors. A zero step disables line/wheel scrolling; zero
width hides the bar. Keyboard paging remains available. The existing textarea
vertical `scroll-step` is in logical lines; its horizontal API step is
`font-size * scroll-step` pixels.

A fixed themed gutter is reserved inside the scroll border and padding even
when content fits, avoiding reflow feedback as the scrollbar appears. Each bar
hides automatically when its axis has no overflow. With a monotonic render
clock, both bars also hide after the configured idle delay. Wheel, keyboard,
caret reveal, pointer activity through the viewport and scrollbar gestures
restart the deadline; a held thumb remains visible. Pointer activity on a child
wakes its scrollable ancestors. Hiding changes painting only: gutter size,
viewport, offsets, clipping and track/thumb hit geometry stay unchanged. A
hidden gutter can therefore reveal and grab its thumb in the same gesture.
There is no fade or repeated animation sampling: the native wait set schedules
a single deadline and redraws once when it expires. Immediate `Renderer::render`
without a clock remains deterministic and keeps overflowing bars visible.
Each enabled axis reserves its own gutter. Both Textarea axes
use the shared theme by default; zero scrollbar-width hides the bars and removes
the gutters. With a collapsed content viewport the bars are suppressed to avoid
painting over the field border. Timed
[selection edge autoscroll](input.md#selection-autoscroll) is available for Input and
Textarea. This container lays out its complete bounded View tree;
it does not claim the 10,000-row virtualization criterion.

`Renderer::scroll_offset`, `scroll_viewport`, `scroll_track`, `scroll_thumb`,
`scroll_by`, `scroll_wheel`, `scroll_key` and `scroll_pointer` expose the pure
path. The `*_x` diagnostics and `scroll_by_x` address the horizontal axis;
`scroll_wheel_axis` and `scroll_key_axis` take `ScrollDirection`. Textarea uses
the same controller API and bar geometry. Text editing keys keep priority over
viewport navigation. `ComponentWindow` exposes offset/viewport/thumb diagnostics and routes
native events automatically. Pure callers use `scroll_activity_at(target, now)`
to announce pointer/keyboard activity and `scroll_hide_deadline_ms()` to schedule
the next render on their own monotonic wait set. `scrollbars_visible(key)`
reports the last painted visibility; geometry diagnostics remain available while
paint is hidden.

## Workspace controls

The typed SDK provides tabs, split panes, toolbars, status bars, modal dialogs
and message boxes. They use the existing component model, DSS, CPU painter,
seat input and wait-set loop. 

### Tabs and command bars

```rust
View::tabs("documents", "Documents", self.active, vec![
    Tab::new("edit", "Editor", Msg::Tab("edit"), editor_view()),
    Tab::new("preview", "Preview", Msg::Tab("preview"), preview_view()),
])
View::toolbar("tools", "Commands", vec![
    View::button("save", "Save", Msg::Save),
    View::toolbar_separator("separator"),
    View::button("open", "Open", Msg::Open),
])
View::status_bar("status", "Operation status", self.status.as_str())
```

Tabs are controlled: selecting a header dispatches its message, and the next
model view accepts or rejects the selected key. Disabled headers are skipped.
Left/Right wrap over enabled tabs; Home/End select endpoints. Ctrl+Tab and
Ctrl+Shift+Tab switch the nearest active tab set, including from page controls.
There is one Tab stop for the tab header group, followed by active page controls.
Already-selected tabs do not dispatch duplicate activations.

Only the selected page is mounted. Its model values survive switching; transient
editor selection/undo/caret state is recreated when an inactive page remounts.
No hidden page participates in input, timers, layout or painting. Page IDs must
be nonempty and unique, and the selected ID must exist. Empty tab sets and bad
inactive-page descriptions fail validation. Generated keys use
`<tabs-key>/tab/<page-id>`, `<tabs-key>/headers` and
`<tabs-key>/panel/<page-id>` and must not collide with application keys.

A toolbar has one remembered Tab stop. Left/Right and Home/End move focus among
enabled commands without activating them; Space/Enter or a click activates the
focused command normally. It accepts direct button/checkable/menu/select
commands and passive labels/separators. Nested focusable controls and editors
are rejected: use a regular Row for a form/search bar that owns its caret keys.
Status bars and separators are passive and never dispatch input messages.

Roles include group, tablist, tab, tabpanel, toolbar, status and separator.
`selected_tab`, `accessibility_selected`, `accessibility_controls` (for the
mounted selected panel), `accessibility_labelled_by` and orientation metadata
are available for a future accessibility bridge.

### Controlled split panes

```rust
View::splitter("panes", "Pane sizes", self.ratio,
    SplitterOptions::new(SplitAxis::Horizontal), left_view(), right_view(),
    Msg::Ratio)
```

The ratio is an integer in permille (0..1000). `SplitterOptions::range` reuses
validated overflow-safe SliderOptions for logical bounds, step and page step;
the default step is 10. Horizontal places panes left/right, Vertical top/bottom.
The generated `<split-key>/handle` is a focusable separator with a supplied label,
orientation metadata and `splitter_value()`.

Dragging preserves the initial pointer offset, continues outside the handle
inside the client, and clamps at endpoints. Left/Up decrease, Right/Down
increase; Home/End and PageUp/PageDown use the configured range. Escape, Leave,
removal, disabling or axis/range changes cancel capture. Resize uses the new
geometry. The model accepts or rejects proposals; unchanged values do not
redispatch. Disabling the splitter disables its generated handle while keeping
pane contents available.

Paint layout, hit testing and pointer mapping share SplitGeometry. Pane primary
min/max layout constraints bound the visual divider; impossible minima shrink
proportionally to fit. The model still owns the requested ratio, so its logical
value may differ from the visually constrained position in a very small client.
If both pane maxima cannot fill the area, remaining space is left unused.
Nested splitters arrange parents before descendants; no application hit-test or
manual painting is needed.

The UI layout layer exposes validated subtree measurement and rearrangement
for these compound controls. Invalid IDs/nonfinite rectangles are rejected
without changing the prior layout. Scroll capture is cancelled when modal/popup
ownership changes or a fresh splitter gesture begins.

### Modal dialogs and message boxes

```rust
let dialog = self.show_dialog.then(|| View::dialog(
    "settings", "Settings", settings_body(),
    vec![DialogAction::new("accept", "Save", Msg::Accept),
         DialogAction::new("cancel", "Cancel", Msg::Cancel)],
    Some("accept"), Some(Msg::Cancel)));
View::modal("window", main_view(), dialog)
```

`View::modal` is a top-level client layer: the base view, a themed scrim and one
centered dialog. `View::dialog` can also be the sole root view of a dedicated
client window. Nested modal hosts, dialogs outside their host and multiple
simultaneously mounted dialogs fail validation. The default action must refer
to an activation-capable descendant.

Opening captures the opener and cancels other gestures/popups/tooltips. Focus
starts at the enabled default action, otherwise the first eligible descendant.
Tab/Shift+Tab stay inside the dialog. Background clicks, menu mnemonics, tab
switching and root shortcuts cannot activate the base UI. Menus and input fields
inside the dialog retain their own routing priority.

Enter activates a focused action, or the default action from a single-line
field; multiline editing and spinbox commit retain their existing Enter rules.
Escape first cancels an open menu or splitter gesture, then sends the dialog's
cancel message. A dialog without a cancel message consumes Escape. Closing only
happens when the model removes the dialog; rejecting a close request leaves the
scope active. The model also owns draft/accept/cancel data semantics.

Closing restores the eligible opener. Replacing a dialog preserves the original
base opener. Resize/theme reload preserves model and focus while recomputing
bounded placement; the body viewport shrinks before actions become unreachable.
Background model updates are allowed while input is scoped to the dialog.

```rust
View::message_box("notice", "Notice", "First line\nSecond line", vec![
    DialogAction::new("ack", "OK", Msg::Acknowledge),
], Some("ack"), Some(Msg::Acknowledge))
```

Message boxes use the same dialog mechanism. Plain LF-delimited message lines
sit in a two-axis scroll viewport; long lines scroll rather than automatically
word-wrap. The viewport label derives from the supplied title. `active_dialog()`
exposes the active scope in Application and ComponentWindow. These are modal to
one client surface, not compositor-managed dialogs spanning multiple windows.

## Protocol source

The control codec is `protocols/gui-v1/src/control.rs`. Clients import its
types and encode/decode methods for CAP1, INP1, RLD1 and NOT1. GUIC stays in
`libdunit`; DGUI framing and the surface protocol live in `gui_protocol_v1`.
M7.7's XFR1 codec and generated transfer values have their own
[contract](desktop-transfers.md#clipboard-and-dragdrop).

`tools/check_gui_protocol.py` derives control tag names and little-endian
values from the canonical declarations. It scans production Rust under
kernel, userspace, runtime and protocols, rejecting local redefinitions and
direct byte/raw-byte or integer copies, including escaped bytes and integer
radices. Comments and diagnostic strings are skipped. External `tests/`
golden fixtures retain independent expected wire bytes. Wire conformance is
also checked by the protocol crate's tests.

```sh
python3 tools/check_gui_protocol.py
python3 tools/test_gui_protocol_source.py
python3 tools/gen_abi.py --check
cargo test --manifest-path protocols/gui-v1/Cargo.toml
python3 tools/test_sdk_gui_fixture.py component --accel tcg
```

Both source checks run in the existing CI protocol-contract step. Mutation
tests insert duplicate byte/numeric constants at each package depth, check
raw/escaped forms, and verify that comments, golden fixtures and ordinary
imports remain valid. Missing or duplicate canonical declarations fail.

## Native verification

`tools/test_dunit_gui_workflow.py`, `test_sdk_catalog.py` and
`test_sdk_window_resize.py` build/boot through `tools/qemu_test.py`. The workflow
checks external-project generation, actual GUI close/exit and 201 buffer commits
with bounded physical-memory and object lifetime. Empty/error/panic suites must
fail with guest exits 1/1/101, including before any window exists.

The resize fixture issues maximize while a painting callback sleeps. It requires
a new paint/geometry, restored dimensions and saved placement, cleared maximize
state and exit 0. The `canvas` fixture delays publication until an overtaken private
document snapshot is replaced at the new geometry. The `component` fixture
requires exactly two partial texture/composition/presentation repairs of 15,360
pixels each, retires/reuses backings and skips an unchanged update. Those counters
measure different work from CPU invalidation and do not imply a timing speedup.

Scroll/workspace/focus/routing host contracts and native fixture names are indexed
in [control verification](widgets.md#verification). Theme/canvas references remain
independent of adapter output. Original GUI smoke, Settings, SVG and PTY contracts
are regression gates for the native application boundaries.

## Raw DUI validation limits

Quoted text preserves UTF-8 and ASCII escapes. Subtree translation rejects
invalid IDs and mismatched layout sizes before changing geometry. Padding accepts
complete 1/2/4-value forms; malformed tokens or extra values preserve existing
edges. Weighted grow(N) sizing uses its positive weight. Raw Attrs::apply still
retains unknown keys and falls back for other malformed numeric values; it is
not a strict configuration validator. Use checked component/configuration APIs
for candidates that must reject invalid fields atomically.
