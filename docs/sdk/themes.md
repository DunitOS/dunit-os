# SDK themes and named values

`ComponentWindow::open_themed(width, height, component)` loads the configured
font and SDK profile. It gives Text and Row/Column/Stack/Grid a shared default
appearance without application styles. System Monitor and the generated GUI
starter use this path. Interactive widget behaviour, backdrop blur and animation
remain separate M7 work.

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
It validates tokens and the supported color, font-size, padding, border-width
radius, background-gradient and shadow declarations before constructing the cascade. Optional application
rules use `open_themed_with_style(..., overlay)`; they can reference profile
values. Local overlay definitions stay local to that layer. Radius and border now drive shared rounded painting. The packaged optional
`cards.dss` layer also uses `shadow`, `shadow-y` and `shadow-spread` profile
values; custom profiles only need these additional names when using that layer.
Motion values remain queryable; widget animations are separate work.

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
text and descendants retain rectangular layout clips, rather than acquiring a
rounded overflow mask. Radius is clamped to half the smaller box dimension.

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
These effects add no syscall or GUI protocol changes. Backdrop blur, rounded
window shapes, widget animations and the whole-scene 16 ms budget remain open.

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
cargo test --manifest-path runtime/dunit-component/Cargo.toml
python3 tools/test_sdk_gui_fixture.py theme --accel tcg
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

Verified on 27 October 2026: 17 style, 11 component, 13 native SDK and 101
desktop/console/config host checks passed, plus 8 CLI-tooling and 8 image-profile
checks. The theme fixture and exact framebuffer/golden comparison passed on
TCG and KVM. The generated GUI workflow passed 201 real frames and expected
failure exits 1/1/101. Catalog/monitor refresh, maximize/restore and close passed
on TCG; boot/runtime stress/IPC and full desktop smoke passed on KVM. The prior
component-damage contract passed on TCG. ABI generation stayed in sync at 89
syscalls, 25 errno and 7 rights.
Compiler warnings remained 13 within the existing budget of 99.


The effects gallery has a separate reviewed baseline in
`tools/fixtures/sdk_effects_golden.json`. Run
`python3 tools/test_sdk_gui_fixture.py effects --accel tcg` to check four actual
framebuffer captures: dark, a partially repainted narrower card, light, and dark
restored. The native fixture also compares the narrowed retained canvas against
a fresh full render and requires less than full-window CPU damage. The original
no-application-style theme golden remains a separate unchanged regression.


Verified effects on 28 October 2026: 174 host checks passed (13 render, 14
component, 17 style, 13 SDK, 101 desktop/config/console, 8 CLI and 8 image-profile).
The effects gallery passed on TCG and KVM, with all four actual framebuffer
captures matching the reviewed baseline. The original theme golden and component
damage/resize fixture passed on TCG, including two 15,360-pixel partial frames.
Full desktop smoke passed on KVM with all 30 required markers and no forbidden
markers. The ABI remained at 89 syscalls, 25 errno and 7 rights. This verifies
correctness; it does not establish the remaining whole-scene 16 ms frame budget.
The warnings check remained at 13 against the existing budget of 99.
