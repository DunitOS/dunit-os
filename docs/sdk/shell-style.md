# Shared shell metrics and cursor

The shell's appearance data is in `/assets/dwm/sdk/shell.dss`, shared with native
SDK theme loading. The registry now adds 96 numeric metrics, three opaque paint
colors and a cursor asset path to the existing shell palette. Rust stores types,
validation bounds and geometry algorithms; it provides no fallback metric values.

`Theme::parse_shell(profile, defaults, base, overlay)` validates the entire shell
schema. Removing a token cannot disable validation. The generic `Theme::parse`
and `parse_with_defaults` constructors remain available for pure SDK renderers
that do not load shell resources.

| Token group | Consumers |
| --- | --- |
| `shell-layout-*`, `shell-effects-*` | Existing Layout/Effects, including the compatibility strip tree |
| `shell-group-*` | Popup size, arrangement and the same cells used by pointer hit tests |
| `shell-qs-*`, `shell-toggle-knob` | Quick-settings paint and pointer geometry |
| `shell-switcher-*`, `shell-hover-*`, `shell-menu-*` | Window switcher and launcher paint |
| `shell-widget-*`, `shell-notification-*` | Desktop cards and notification paint |
| `shell-cursor-outline`, `shell-cursor-fill`, `shell-cursor-source` | Full and partial software cursor paint |

Explicit numeric TOML `[layout]` and `[effects]` fields override matching tokens;
omitted fields inherit DSS. Boolean policy and placement choices remain in TOML.
The legacy tree is rebuilt with the resolved geometry before layout, so a DSS
dock size changes both its painted area and hit targets. Explicit shell trees
retain their own panel/component dimensions and DSS selectors.

Metrics are finite integers within declared bounds. Validation also rejects
inconsistent toggle/knob sizes, inverted icon limits and unusable padding.
Compositor resources are prepared at configuration/reload boundaries. Painting
uses cached typed values, with no token lookup, parsing or resource reads in a
frame. A rejected candidate keeps the previous scene.

A custom profile can override individual values:

```css
$shell-layout-title-h: 34;
$shell-layout-dock-w: 72;
$shell-qs-width: 196;
$shell-qs-padding: 10;
$shell-cursor-fill: #ff00ff;
$shell-cursor-source: /assets/dwm/cursor.txt;
```

Cursor files contain rectangular rows: `X` is outline, `.` is fill, `_` is
transparent. The shape owns its dimensions; the top-left pixel is the hotspot.
Paths must be normalized absolute paths. The compositor reads a bounded UTF-8
file and rejects malformed/ragged/oversized shapes. Cursor colors must be opaque.
The same decoded mask and selected colors are used after partial content damage
and after full composition; the packaged shape preserves the previous arrow.

The native theme fixture selects a magenta cursor fill only in its temporary
light profile. Three QEMU captures independently verify dark/light/restored
panel and cursor pixels while the SDK client canvas retains its existing pixel
hashes. Host checks cover custom popup cells, clipped layouts, numeric precedence,
invalid metrics, missing schema fields, cursor data and colored partial repair.

This checkpoint does not complete M7.9. Private window-control offsets and other
shell/window paint literals remain. Scalar Settings saves still serialize all
legacy appearance fields, which can pin previously inherited values. Extraction
has not been prototyped or measured; no new compositor protocol is introduced.

## Measurement checkpoint, 4 November 2026

QEMU TCG, one vCPU, 512 MiB, 1920×1080 software framebuffer. Native budgets:
theme 8 required/4 forbidden plus exact dark/light/restored panel, cursor and
client pixels; component 12 required/5 forbidden plus two exact
`composed=15360 presented=15360 texture=15360 screen=2073600` repairs; GUI smoke
30/13. Invalid quick-settings geometry is rejected by both SDK and compositor,
and the actual framebuffer retains the previous light panel/cursor/client.

The final input run also includes the independently documented
[IntelliMouse correction](../performance/continuous-input.md#intellimouse-id-3-correction--4-november-2026).
The whole-loop frame marker stayed ~21 ms versus the previous checkpoint. The
loaded GUI smoke markers were ~83 ms and ~114 ms in successive runs, versus
~81 ms at the previous checkpoint; ≤16 ms is not achieved. That self-test has
variable window/recycle timing (the final run reopened seven clients, the
previous run four), so these are not matched steady-load samples. Host stream
totals also vary and some increased. No general speedup, absence of performance
regressions or completed M7.9 frame-time acceptance is claimed. The additional atomic wheel
burst is excluded from the original three-trial medians. Raw trial summaries
are in [shell-style-results.json](../performance/shell-style-results.json).

Host validation: 163 existing DWM/geometry/render/editor checks, two new PS/2
regressions and eight shared-theme checks passed. The colored-cursor partial
repair tests were rerun after their color matrix changed. These are scoped
checks; the complete OS test matrix was not rerun for this styling increment.
