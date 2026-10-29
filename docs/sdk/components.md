# Typed components

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

Views contain text, [push buttons](buttons.md), [checkable controls](checkable-controls.md),
[single-line fields](text-input.md), [multiline fields](multiline-input.md), [scroll regions](scroll-areas.md) and Row/Column/Stack/Grid
containers. Text is stored directly;
quotes, braces and Unicode do not need markup escaping. Every node has a
nonempty unique key, also used as its DSS ID. The current adapter hosts one root
component. This is not yet a nested component/widget lifecycle system.

`View::class(name)` selects a DSS appearance class from the model. The renderer
uses these classes during cascade resolution. Supported appearance changes can
use [timed DSS transitions](motion.md). Buttons and fields supply automatic interaction
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

## Rendering and native windows

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
single-session restrictions still apply; see [window.md](window.md).

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

System Monitor now uses `ComponentWindow` and a typed `View`. Its DUI string
builder and repeated parse/layout/paint code are removed; counters change only
through a refresh message. It now uses `open_themed` and the shared
[SDK token profiles](themes.md), including live reload, without a private DSS
file or font loader. The generated GUI starter uses the same component path.
Buttons, fields and scroll regions retain interaction state by stable key and kind. Other
widget families and nested component lifecycle remain separate work. See
[buttons](buttons.md) and [text input](text-input.md).

## Verification

```sh
cargo test --manifest-path runtime/dunit-component/Cargo.toml
cargo test --manifest-path runtime/dunit-render/Cargo.toml
cargo test --manifest-path runtime/dunit-style/Cargo.toml
cargo test --manifest-path runtime/dunit-widgets/Cargo.toml
python3 tools/test_sdk_gui_fixture.py component --accel tcg
python3 tools/test_sdk_catalog.py --accel tcg
```

The native fixture opens a real component window, receives Space from QMP,
checks the changed model/view and a 480×32 dirty row inside a 480×240 frame,
skips an unchanged update, then reuses a retired backing with a 480×32 upload.
It requires exactly two partial presentations, each copying, composing and
presenting 15,360 pixels on a larger screen. It maximizes and restores while
retaining state, then closes with child exit 0.
Its `[SDKUI] ok` marker is required alongside all intermediate assertions.
The existing System Monitor refresh/close and catalog resize tests retain their
previous requirements and additionally require typed-component presentation.
All guest launches use `qemu_test.py`; the fixture builder is shared with the
existing resize-during-paint test.

Damage validation on 26 October 2026: 75 gui-v1 tests, 13 native SDK tests,
9 component tests, 99 desktop host checks (including 4 damage planner tests),
and 16 Python tooling/image-profile tests passed. ABI generation remained in
sync at 89 syscalls, 25 errno and 7 rights. The native component fixture passed
in KVM and TCG. The TCG GUI workflow passed 201 frames with bounded memory and
buffer objects, plus expected guest failure exits 1/1/101. Catalog discovery,
System Monitor refresh/maximize/restore and resize during painting passed in
TCG. The counters distinguish the first full backing upload from subsequent
15,360-pixel repairs; an unchanged update produces no extra presentation.
The KVM desktop smoke satisfied all 30 required markers, with no forbidden
markers, including the first-frame ordering regression for legacy clients.
The existing Lua/Genie engine contract also passed all 20 required markers.
Compiler warnings stayed at 13 within the existing budget of 99.
