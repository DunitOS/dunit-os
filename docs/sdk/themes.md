# SDK themes and effects

SDK applications and compositor chrome load shared DSS profiles from the VFS.
The same pure validators, raster primitives and motion interpolation are used
in host tests and native applications.

## Themes

`ComponentWindow::open_themed(width, height, component)` loads the configured
font and SDK profile. It gives Text and Row/Column/Stack/Grid a shared default
appearance without application styles. System Monitor and the generated GUI
starter use this path. Appearance classes support [timed DSS transitions](themes.md#appearance-transitions);
interactive behavior is documented in the controls and input manuals.

The desktop document selects the profile:

```toml
[desktop]
sdk_theme = "/assets/dwm/sdk/light.dss"
font = "/assets/fonts/DejaVuSans.ttf"
```

The packaged dark profile is the default. macOS Light selects the light profile;
Mojave Dark, Nord and Green Tea select dark. Custom profiles can live under
`/persist`; paths must be absolute without `..`. The common structural rules are
in `/assets/dwm/sdk/base.dss`. The SDK reads these as ordinary VFS files, not
embedded kernel assets. SDK clients and compositor chrome share the profile,
`/assets/dwm/sdk/shell.dss` token defaults and the same pure theme validator.
Shell stylesheets can reference both core SDK tokens and `shell-*` colors.

Explicit legacy `[theme]` fields are converted to `shell-*` overrides at the
configuration boundary (TOML ARGB is converted to DSS RGBA). Omitted fields
inherit the selected profile; a profile may override individual `shell-*` values.
These defaults never replace an explicit value. App colors keep their normal
core tokens, while the complete registry exposes intentional chrome variations.
Existing legacy presets therefore retain their colors.

The compositor prepares the palette and stylesheet before accepting a desktop
configuration. Invalid candidates retain the previous shell. Native SDK clients
poll their theme resources; after saving a desktop/profile change, call
`ComponentWindow::reload_desktop()` to request the compositor's existing atomic
reload. In-place file edits alone do not notify the compositor.

[Shared shell metrics](desktop-transfers.md#shell-appearance) cover decoration/strip geometry,
popup hit cells, quick settings, switcher, desktop cards, notifications and
cursor resources. Explicit numeric `[layout]`/`[effects]` fields become DSS
overrides just like legacy colors. SDK clients and the compositor call
`Theme::parse_shell`, which always validates the complete shell schema.

[Scalar saves now preserve inheritance](desktop-transfers.md#settings-persistence): snapshot edits write
changed fields only; explicit setters and partial imports retain supplied fields,
including values equal to defaults. Complete document replacement is separate.

The accepted retained-shell design uses shared DSS: window controls and remaining
paint/layout details use shared DSS. The [architecture decision](desktop-transfers.md#retained-shell-decision)
and [acceptance measurements](../performance/gui-performance.md#retained-shell) document
why the shell stays in `gui_server`; external extraction and ≤16 ms are not claimed.

### Token contract

DSS definitions remain queryable after parsing. Names omit the `$` in the API:

```rust
let (theme, font) = dunit::ui::load_theme("")?;
let accent = theme.tokens.color("accent");
let spacing = theme.tokens.number("space");
let duration_ms = theme.tokens.duration("motion-fast");
// Use theme.cascade, theme.background and font for a custom Renderer.
```

`Tokens::get` returns a typed Value; `iter` enumerates names and values. Missing
names and type mismatches return None. Definitions and aliases resolve in source
order. Rebinding a name does not retroactively change an earlier alias or rule.
`parse_with_tokens` seeds an additional layer and rejects unknown references,
including a missing name inside a multi-value declaration. It does not mutate
its input registry. The original `parse` entry retains its legacy expansion
behaviour, while also retaining token metadata.

A complete SDK profile defines:

| Names | Type and accepted range |
| --- | --- |
| `window-bg`, `surface`, `text`, `muted`, `accent`, `line` | Color; window background must be opaque |
| `font-size`, `font-title` | Number, 1–512 |
| `space`, `space-small`, `radius` | Number, 0–1024 |
| `border` | Number, 0–64 |
| `motion-fast` | Duration, 0–60,000 ms |

`Theme::parse(profile, base, overlay)` is pure and shared by host and guest.
`Theme::parse_with_defaults(profile, defaults, base, overlay)` additionally loads
a token-only defaults layer; it rejects rules there and invalid color overrides.
It validates tokens and the supported color, font-size, padding, border-width,
radius, background-gradient, shadow, backdrop-blur and clip declarations before
constructing the cascade. Optional application rules use
`open_themed_with_style(..., overlay)` and can reference profile values. Local
overlay definitions stay local to that layer. Radius and border drive shared
rounded painting. The optional `cards.dss` layer uses `shadow`, `shadow-y` and
`shadow-spread`; `glass.dss` uses `glass-tint`, `glass-radius` and `blur-radius`.
Custom profiles only need these extra names when using those layers. Motion
values drive SDK paint-property transitions; full widget animations remain separate work.

### Rounded fills, gradients and shadows

```css
Text#card {
    radius: $radius;
    border-width: $border;
    border-color: $line;
    background-gradient: vertical $surface $line;
    shadow: 0 $shadow-y $shadow-spread $shadow;
}
```

`background-gradient` accepts `vertical` or `horizontal` followed by exactly two
colors. It replaces `background`; `none` restores the flat fill. Endpoints stay
anchored to the original layout box during clipped repaint. RGBA values are
interpolated linearly per channel. `radius` rounds the fill and inside border;
text and descendants retain rectangular layout clips unless `clip: rounded`
is selected. Radius is clamped to half the smaller box dimension.

`shadow` accepts x/y offset, spread and color, or `none`. Offsets range from
−1024 to 1024 pixels and spread from 0 to 64. Spread is a bounded nested-layer
falloff, not a Gaussian blur. The shadow excludes the node silhouette and may
extend beyond its box, but remains inside its ancestor layout clip and the
window canvas. The painter skips the covered centre of each layer. Transparent
or zero-spread shadows paint nothing.

Retained rendering invalidates both old and new visual bounds, including shadow
extent and pixel rounding. It then restores the background and repaints all
intersections in stacking order. This removes old shadows after shrinking or
moving a node and preserves no-op frame skipping. The desktop decoration painter
and SDK use the same integer rounded-corner coverage and RGB interpolation
kernels from `dunit_render::raster`; the desktop keeps its existing shadow policy.
These effects add no syscall or GUI protocol changes. Rounded window shapes,
blur of the desktop behind an application window, widget animations and the
whole-scene 16 ms budget remain open.

### Backdrop blur and rounded content clips

```css
Column#glass {
    background: $glass-tint;
    radius: $glass-radius;
    clip: rounded;
    backdrop-blur: $blur-radius;
}
```

`clip: rounded` clips the completed group: its background, inside border, text,
descendants and their shadows all share the parent's rounded boundary.
The node's own shadow remains outside that boundary. Coverage is applied once
after the group paints, so overlapping children do not compound its antialiased
edge. Scratch storage saves only affected corner pixels. Nested masks compose;
`clip: rect` explicitly selects rectangular clipping. The retained component
renderer already contains every node in its rectangular layout box by default.

`backdrop-blur` is a number from 0 to 64 pixels, rounded to the nearest integer;
0 disables it. It filters the opaque canvas already painted behind this node,
before adding its background and foreground. Two horizontal/vertical box passes
produce a bounded soft blur. Output is contained in the node's shape and current
clip, while sampling includes a `2 * radius` halo beyond the output rectangle
(up to the window edge). This matches filtering the complete canvas. SDK themes
require an opaque window background; direct `Surface::backdrop_blur` calls also
expect an opaque backdrop. This samples the application's own canvas and does
not read the desktop behind the application window.

The desktop and SDK share the production RGB box-filter kernels in
`dunit_render::blur`. Existing desktop blur configuration and buffer ownership
stay in the compositor. SDK blur allocates two bounded region buffers when its
output is repainted. The retained renderer checks damage against every old and
new blur rectangle expanded by the shared filter support (`2 * rounded radius`)
and a conservative pixel-rounding guard. Changes disjoint from all of these
regions retain partial painting and avoid blur scratch allocation. This includes
an independently changing label beside a glass panel.

Damage touching any filter output or sampled halo still rebuilds the complete
client canvas. Replaying from the scene prevents feedback from the previous
frame. Both snapshots matter when a filter moves, changes radius or disappears.
Ancestor clips and paint order can reduce dependencies; this check deliberately
overencloses them, including nested and overlapping filters. Unchanged scenes
still skip painting and publication. Filter-local reconstruction and the
whole-scene frame-time budget remain performance work. Pixel counts describe
invalidated canvas area, not paint stores or elapsed time.

`paint`, `paint_clipped`, `Surface::backdrop_blur` and
`Surface::with_rounded_clip` return allocation errors rather than silently
omitting an effect. Raw callers must discard a failed frame. `Renderer` reports
`UiError::Allocation` and invalidates its private baseline so any retry rebuilds
a complete frame; the window adapter does not publish a failed draw.

### Reload and errors

Themed windows check sources at most once per 500 ms while `next_event` waits.
The original caller timeout is preserved: internal checks do not produce early
None events. Input and resize continue through the existing window adapter.
Applications which do not wait in `next_event` can call `reload_theme()`.

Changing the profile path or profile/base contents invalidates the full canvas
and presents the new appearance. Changing the font path loads the new font too;
editing a font file at the same path requires restarting the application. Model/view
state is retained. Unchanged sources do not redraw. Custom `open(..., renderer)`
windows remain explicitly managed and do not acquire automatic theme watching.

Unreadable, oversized, malformed or wrong-type candidates leave the working
renderer and frame intact. `theme_error()` reports its errno and
`theme_error_detail()` retains a diagnostic with parser/token/property details;
a later valid candidate clears it. Explicit reload returns the error, automatic
reload emits a diagnostic once per changed error and keeps the window usable.
Native drawing/transport failures still poison the window session. DSS reads
are bounded to 64 KiB and font loads to 4 MiB. This uses VFS reads and the
existing wait-set; there is no new syscall, GUI packet or background thread.

## Appearance transitions

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
hover, clicking, focus or other widget behaviour. Those belong to the controls and input controllers.

### Supported properties and timing

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

### Pure rendering

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
the client canvas to avoid blur feedback; see [the blur contract](themes.md#themes).

### Native windows and reduced motion

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

Host tests in `runtime/dunit-style`, `dunit-render` and `dunit-component` cover
strict theme validation, token precedence, retained/fresh pixel equality, clipped
effects, allocation errors, interrupted transitions and reduced motion. Native
fixtures use `python3 tools/test_sdk_gui_fixture.py NAME --accel tcg` with NAME
`theme`, `effects`, `backdrop` or `motion`; all guest runs use `tools/qemu_test.py`.

The reviewed references in `tools/fixtures/sdk_theme_golden.json`,
`sdk_effects_golden.json`, `sdk_backdrop_golden.json` and `sdk_motion_golden.json`
pin actual full-client endpoints. References are not regenerated by acceptance.
Theme compares dark/light/restored states while retaining model state and rejecting
bad candidates. Effects checks a partial narrower-card update; backdrop checks
damage outside the blur halo, full repair when the halo changes and no-op skipping.
Motion compares endpoints, exact timed host intermediates, caller timeout handling
and closure during an active transition. Pixel counts do not measure frame time.

The shared shell fixture independently verifies panel and actual cursor pixels
alongside unchanged client hashes; scalar saves must preserve inherited DSS fields.
`tools/check_shell_style.py`, `test_shell_style_source.py` and the `shellprobe`
fixture guard appearance sources and client isolation. Performance measurements
remain in [gui-server acceptance](../performance/gui-performance.md#retained-shell).

The 28 October 2026 backdrop record also observed one recycling instruction-fetch
fault at RIP/address 0. An identical-image retry and two control runs passed; no
cause was established for that run. Later collection workloads found and fixed a
DF-on-kernel-entry fault with an explicit DF regression, but the record does not
prove it was the same failure. Successful theme/effect tests alone do not resolve
that historical stability finding or establish the whole-scene 16 ms budget.
