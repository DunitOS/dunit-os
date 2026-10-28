# SDK themes and named values

`ComponentWindow::open_themed(width, height, component)` loads the configured
font and SDK profile. It gives Text and Row/Column/Stack/Grid a shared default
appearance without application styles. System Monitor and the generated GUI
starter use this path. Interactive widget behaviour and animation remain
separate M7 work.

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
embedded kernel assets. Desktop chrome still uses its existing TOML theme;
this step does not unify compositor decoration and SDK token registries.

## Token contract

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
It validates tokens and the supported color, font-size, padding, border-width,
radius, background-gradient, shadow, backdrop-blur and clip declarations before
constructing the cascade. Optional application rules use
`open_themed_with_style(..., overlay)` and can reference profile values. Local
overlay definitions stay local to that layer. Radius and border drive shared
rounded painting. The optional `cards.dss` layer uses `shadow`, `shadow-y` and
`shadow-spread`; `glass.dss` uses `glass-tint`, `glass-radius` and `blur-radius`.
Custom profiles only need these extra names when using those layers. Motion
values remain queryable; widget animations are separate work.

## Rounded fills, gradients and shadows

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

## Backdrop blur and rounded content clips

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
stay in the compositor. SDK blur allocates two bounded region buffers. A scene
with active blur redraws the entire client canvas whenever anything changes,
including a change behind the blurred node. Rebuilding from the scene prevents
blur feedback and stale sampled neighbours. Unchanged scenes still skip painting
and publication. Scenes without blur retain partial damage, including rounded
content masks. A narrower filter dependency planner and the frame-time budget
remain performance work.

`paint`, `paint_clipped`, `Surface::backdrop_blur` and
`Surface::with_rounded_clip` return allocation errors rather than silently
omitting an effect. Raw callers must discard a failed frame. `Renderer` reports
`UiError::Allocation` and invalidates its private baseline so any retry rebuilds
a complete frame; the window adapter does not publish a failed draw.

## Reload and errors

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

## Verification

```sh
cargo test --manifest-path runtime/dunit-style/Cargo.toml
cargo test --manifest-path runtime/dunit-render/Cargo.toml
cargo test --manifest-path runtime/dunit-component/Cargo.toml
python3 tools/test_sdk_gui_fixture.py theme --accel tcg
python3 tools/test_sdk_gui_fixture.py effects --accel tcg
python3 tools/test_sdk_gui_fixture.py backdrop --accel tcg
```

The native fixture opens a component with no application DSS, changes the theme
while retaining its counter, rejects both an unknown token and invalid layout
style, restores dark and closes cleanly. It captures actual QEMU framebuffer
images for dark, light and restored states. Every client pixel must match both
the native renderer's RGB hash and the reviewed baseline in
`tools/fixtures/sdk_theme_golden.json`. The baseline pins font bytes, canvas
size and three full-frame hashes; it is not regenerated by the test. Review the
captures and commit an intentional baseline update when changing default visuals.

Kernel terminal writes, clears and cursor updates now honor the existing
exclusive display lease, so serial output cannot overwrite a retained GUI frame.
Terminal history and serial diagnostics continue; console painting resumes
when the lease ends. The host regression exercises the production console
implementation, and the framebuffer comparison checks the real guest path.

The effects gallery has a separate reviewed baseline in
`tools/fixtures/sdk_effects_golden.json`. Run
`python3 tools/test_sdk_gui_fixture.py effects --accel tcg` to check four actual
framebuffer captures: dark, a partially repainted narrower card, light, and dark
restored. The native fixture also compares the narrowed retained canvas against
a fresh full render and requires less than full-window CPU damage. The original
no-application-style theme golden remains a separate unchanged regression.


The backdrop fixture has a separate reviewed baseline in
`tools/fixtures/sdk_backdrop_golden.json`. Run
`python3 tools/test_sdk_gui_fixture.py backdrop --accel tcg` to capture dark,
changed background, light and restored states. It compares the rebuilt canvas
to a fresh render, verifies zero damage for a no-op, rejects invalid blur/clip
styles without changing pixels, and requires exact initial pixels after restore.
All four QEMU captures must match both native RGB hashes and the reviewed golden.


Verified on 28 October 2026: 184 host checks passed (19 render, 18 component,
17 style, 13 SDK, 101 desktop/config/console, 8 CLI and 8 image-profile). The
backdrop fixture passed on TCG and KVM with exact four-state framebuffer/golden
comparisons. The original theme and effect goldens passed unchanged on TCG.
The component damage/resize fixture retained its two 15,360-pixel partial frames
on TCG. Full desktop smoke passed on KVM with all 30 required markers and no
forbidden markers. ABI generation remained at 89 syscalls, 25 errno and 7 rights.
This establishes rendering correctness; the whole-scene 16 ms budget remains open.
The warnings check remained at 13 against the existing budget of 99.
