# Native window API

Enable `dunit = { path = "…/runtime/dunit", features = ["gui"] }`, or create a
project with `cargo dunit new demo --kind gui`. The generated canvas uses the
configured desktop palette; it is a starting point for content, not the future
Component/widget framework.

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

`draw` allocates an unpublished buffer and lends its pixels to the callback.
The writable borrow ends before a read-only capability is transferred. A
successful return requires the matching PRESENTED frame callback, not just a
successful IPC send. The previous object is destroyed after BUFFER_RELEASE;
its local mapping and capability are released together. Published buffers are
never exposed as writable slices. `close` completes DISCONNECT; Drop attempts a
nonblocking disconnect and releases local storage, including on errors.

This first helper supports **one window/session per process** and owns the
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
are supported initially. Transparency, multiwindow sessions, reconnect, damage,
Component diffing and widgets remain separate work.

System Monitor now uses the [typed component adapter](components.md) over this
API, including live maximize/restore, timed
refresh and close. Its app code contains no raw IPC, capability management,
pointers, `_start` or panic handler. Other legacy clients remain to be migrated.

```sh
python3 tools/test_dunit_gui_workflow.py --accel tcg
python3 tools/test_sdk_catalog.py --accel tcg
python3 tools/test_sdk_window_resize.py --accel tcg
```

The GUI workflow creates an external project, builds and opens its real window,
closes it through the configured desktop shortcut, then runs 201 native frames.
The frame test exceeds the server's live-buffer count limit and bounds physical
memory growth below the size of 200 leaked backings. Empty/error/panic suites
must fail on the host and expose guest exit codes 1/1/101, even before a window
exists. The compositor now reaps those pending clients too, checking child-exit
readiness before consuming their status. The catalog test additionally
maximizes and restores the migrated monitor, refreshes it and closes it.

The forced-resize test waits for a drawing callback to begin, then sends the
real maximize shortcut while that callback sleeps. It requires a second paint,
a new window size, restoration to the original dimensions and persisted x/y
placement with maximize state cleared, and clean close with
exit code 0. This covers the stale-commit race rather than relying on timing a
resize between frames.

Keyboard events retain the modifier snapshot from scancode arrival, as required
by the existing KeyEvent contract. A shortcut queued during a slow frame keeps
its Super/Ctrl/Alt/Shift flags even if the keys are released before the desktop
reads it. Host queue tests cover delayed reads, split extended scancodes,
overflow and ring wrap; the forced-resize test exercises real guest shortcuts.

Verified on 26 October 2026: 131 host tests passed (72 gui-v1, 11 runtime,
28 desktop configuration, 16 Python tooling/image profiles and 4 keyboard
queue tests). ABI generation stayed in sync. The complete GUI workflow passed
under TCG, including expected rejection of empty/error/panic suites with guest
exit codes 1/1/101. Forced resize, persisted placement restoration and clean
exit passed under both TCG and KVM. The native API/PTY calculator, monitor
refresh/close and full desktop resize/reload smoke also passed under KVM on the
updated kernel. The compiler warning count remained 13 within the budget of 99.
