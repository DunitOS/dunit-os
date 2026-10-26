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

Views contain text and Row/Column/Stack/Grid containers. Text is stored directly;
quotes, braces and Unicode do not need markup escaping. Every node has a
nonempty unique key, also used as its DSS ID. The current adapter hosts one root
component. This is not yet a nested component/widget lifecycle system.

`Application` exposes immutable component/view access. Dispatching a message
calls `update`, builds and validates the next view, then replaces the previous
one. Duplicate keys, duplicate shortcuts, nested shortcuts, invalid dimensions
and excessive depth/node count are errors. Propagate validation failures: a
model can already have consumed the message, while its previous valid view is
retained for diagnostics. There is no model rollback.

`on_key` binds application shortcuts on the root view. A binding matches the
cooked character and native modifier mask exactly. `Key::plain` uses no
modifiers. These shortcuts are not focused-widget editing or button behaviour;
those remain part of M7.6. Child bindings are rejected instead of being ignored.

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

Presentation copies the complete private CPU canvas into a **fresh** shared
buffer. Published shared buffers are never mutated. The live compositor still
receives a complete frame and empty wire damage; partial composition, partial
buffer copies and the protocol damage path remain open. This CPU renderer does
not complete M7.4's compositor-damage requirement.

`Window::try_draw` permits a fallible painting callback. Failure happens before
capability transfer/import/publication and poisons the session. Concurrent
configure can retry the callback at new geometry. Component rendering failures
and invalid updates also poison `ComponentWindow`; propagate them and close or
drop the session.

System Monitor now uses `ComponentWindow` and a typed `View`. Its DUI string
builder and repeated parse/layout/paint code are removed; counters change only
through a refresh message. Its style is in
`assets/dwm/apps/gui_stat.dss`, loaded once on startup. Theme tokens, automatic
live theme reload, interactive widgets and widget-state reconciliation remain
separate work. The generated GUI starter remains the lower-level themed canvas.

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
maximizes and restores while retaining state, then closes with child exit 0.
Its `[SDKUI] ok` marker is required alongside all intermediate assertions.
The existing System Monitor refresh/close and catalog resize tests retain their
previous requirements and additionally require typed-component presentation.
All guest launches use `qemu_test.py`; the fixture builder is shared with the
existing resize-during-paint test.

Verified on 26 October 2026: 70 host tests passed across components, rendering,
styles, widget measurement/events, the native SDK and Python tooling/image
profiles. ABI generation remained in sync at 89 syscalls, 25 errno and 7 rights.
The native component fixture passed in KVM and TCG; System Monitor passed the
same refresh/close contract before and after migration, and catalog discovery
plus maximize/restore passed in TCG. The existing resize-during-paint TCG and
full desktop KVM smoke also passed. Every positive guest contract completed
with its marker budget satisfied. Compiler warnings remained 13/99.
