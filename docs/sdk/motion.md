# SDK appearance transitions

A component can select a DSS class from its model. The SDK animates supported
appearance changes while preserving the model and stable node keys.

```rust
let label = View::text("status", "Ready");
let label = if model.active { label.class("active") } else { label };
```

```css
Text#status {
    background: $surface;
    color: $text;
    radius: $radius;
    transition: background $motion-fast ease-out,
                color $motion-fast linear,
                radius $motion-fast ease-in-out;
}
Text#status.active {
    background: $accent;
    color: $window-bg;
    radius: 24;
}
```

Classes follow the DSS identifier grammar: an ASCII letter or underscore first,
then letters, digits, underscores or hyphens. `View::classes()` exposes them for
inspection. Invalid names fail view validation with `UiError::InvalidClass`.
Classes describe appearance chosen by application state; they do not implement
hover, clicking, focus or other widget behaviour. Those belong to M7.6.

## Supported properties and timing

The initial continuous properties are `background`, `color`, `border-color`,
`radius`, `border-width` and `backdrop-blur`. Both endpoints must explicitly
provide a compatible color/number value. Newly introduced or removed values
snap. Other style properties continue to apply immediately; naming one in an
SDK transition declaration is rejected rather than pretending to animate it.
Layout transforms, opacity, gradient/shadow lists and widget animation behaviour
remain separate work.

Each descriptor has a property, mandatory duration and optional easing. Durations
are milliseconds (`160ms`), seconds (`0.16s`) or bare milliseconds, finite and
within 0–60,000 ms. Easing is `linear`, `ease-in`, `ease-out`, `ease-in-out` or
`ease` (the latter two use smoothstep). Missing easing means linear. Repeated
properties and malformed/unsupported descriptors reject the SDK theme candidate.
`transition: none` or a zero duration applies values immediately. The original
DSS motion decoder remains permissive for existing desktop consumers.

`dunit_style::motion` remains the interpolation source of truth. Transitions
match stable keys and element kinds, not arena indices. Reordering preserves
an active transition; deletion or a changed kind removes it. A new target during
an active transition samples the old path at that timestamp and starts from
there. Removing a descriptor or changing its timing without a new property target
settles to the target. A new target can use new timing while retargeting from the
current sample.

## Pure rendering

```rust
renderer.render_at(&initial_view, width, height, 0)?;
renderer.render_at(&active_view, width, height, 10)?;
renderer.render_at(&active_view, width, height, 90)?;
```

`render_at` accepts caller-owned monotonic milliseconds. This makes intermediate
frames reproducible in host tests. `render` stays an immediate operation, settles
target values and cancels transitions. Initial appearance, resize, changed
style/theme/font, disabled motion or a backwards clock also settle immediately.
Re-enabling motion allows future changes without replaying the current state.

`is_animating()` reports active tracks. `set_motion_enabled(false)` cancels
sampling; the next render reaches the target. `cancel_motion()` cancels the
current tracks without changing the policy. `RenderStats::active_transitions`
and `animation_time_ms` expose the current sample; they are not frame-cost or
performance-budget measurements. Damage includes old/new sampled geometry and
shadow bounds. Changes outside every old/new blur rectangle and its sampled
halo retain partial damage. Changes touching these filter dependencies rebuild
the client canvas to avoid blur feedback; see [the blur contract](themes.md#backdrop-blur-and-rounded-content-clips).

## Native windows and reduced motion

`ComponentWindow` supplies a monotonic clock and samples through its existing
wait-set. No second thread, syscall or protocol packet is added. Its nominal
sampling interval is 16 ms, with missed frames coalesced by elapsed time and
presentation backpressure. It never queues an unbounded series of old frames.
Rendering/transport work can take longer than that interval; this does not prove
the remaining whole-scene 16 ms budget.

`next_event(timeout)` keeps the original caller deadline while serving animation
and theme checks. Internal timers do not return an early None or restart that
deadline. Rendering and presentation work can extend the call beyond its waiting
timeout. Input, resize and close still use the same window session. Resize
settles the target; close requests cancel sampling before any further timer draw.

Themed windows honor the live desktop document's `[effects] anim` flag.
`ComponentWindow::set_motion_enabled(false)` also lets an application reduce
motion and immediately present the target. An application request to enable it
cannot override a disabled desktop flag. Changing that flag through theme reload
settles the frame and preserves model state. Explicitly managed
`ComponentWindow::open(..., renderer)` uses its renderer/application policy and
does not automatically watch desktop settings. Theme/profile/font reloads apply
instantly; appearance-class changes are the animated path in this slice.

## Verification

```sh
cargo test --manifest-path runtime/dunit-style/Cargo.toml
cargo test --manifest-path runtime/dunit-component/Cargo.toml
python3 tools/test_sdk_gui_fixture.py motion --accel tcg
```

Host tests check exact midpoint/end pixels, interrupted paths, stable-key reorder,
removal/retyping, resize, reduced motion, backwards time and strict declarations.
The native fixture receives Space, observes multiple distinct uploaded timed
frames without further input, checks caller timeout semantics and exact endpoints,
rejects an unsupported transition without replacing the frame, disables desktop
motion, and closes cleanly while another transition is active.

Its idle, active, restored and reduced-motion endpoints are captured from the
actual QEMU framebuffer and compared to both native CPU pixels and the reviewed
`tools/fixtures/sdk_motion_golden.json`. The font and canvas dimensions are pinned.
Intermediate values have deterministic host pixel tests; the QEMU golden covers
stable endpoints. The fixture alone extends `$motion-fast` via
`tools/fixtures/sdk_motion_profile.dss` to make observation reliable under load.
The packaged `assets/dwm/sdk/motion.dss` example uses the configured profile's
normal `$motion-fast` value.


Verified on 28 October 2026: 190 host checks passed (18 style, 23 component,
19 render, 13 SDK, 101 desktop/config/console, 8 CLI and 8 image-profile). The
motion fixture passed on TCG and KVM, including the four actual framebuffer
endpoint goldens. Existing theme/backdrop goldens and component input/damage/
resize passed on TCG; the latter retained two 15,360-pixel partial frames.
Full desktop smoke passed on KVM with all 30 required markers and no forbidden
markers. ABI generation remained at 89 syscalls, 25 errno and 7 rights.
The whole-scene 16 ms budget and full widget/geometry animations remain open.
The warnings check remained at 13 against the existing budget of 99.
