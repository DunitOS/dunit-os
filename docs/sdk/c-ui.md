# C UI v1: the first native component bridge

`<dunit.h>` now includes generated `<dunit/ui.h>`. Link a native C program with
`-ldunit_ui` and call `dunit_ui_run_v1(argc, argv, envp, width, height, &callbacks)`.
The static library is the existing Rust `ComponentWindow`, component model,
DSS theme, renderer and window transport behind a synchronous C adapter.
It implements no second layout engine, widget painter or GUI handshake.

This subset describes Text, Button, Column, Row, Stack, Grid, Checkbox, Switch, ToggleButton
and grouped Radio nodes. It opens one native themed window and dispatches
activation messages back to the C-owned model.
Other widgets and C transfer/file-capability
interfaces remain later M7.10 increments. This is not the complete C SDK.

## Frozen v1 contract

`runtime/dunit-c/api.json` owns the library constants, field order, callbacks,
record sizes/alignment and exported function prototype. `tools/gen_abi.py`
generates both C and Rust declarations. Rust compilation asserts the layouts
and actual function signature; the native C example also asserts record sizes.
This library ABI version is separate from kernel ABI v0 and GUI wire versions.
No syscall or wire message changed. Existing v1 values/layouts must not be
renumbered or repurposed; incompatible additions require a new versioned API.

On native x86_64, `DunitUiNodeV1` is 48 bytes/alignment 8 and
`DunitUiCallbacksV1` is 32 bytes/alignment 8. Initialize `abi_version` with
`DUNIT_UI_ABI_VERSION`, `struct_size` with `sizeof callbacks`, and provide both
callbacks. C owns `user` and its model; Rust owns the converted Views and window.

- `build(user, nodes, capacity)` fully initializes nodes `[0,count)` and returns
  that positive count. Zero, over-capacity counts, unsupported kinds, malformed
  UTF-8 and invalid trees fail. A negative callback result propagates unchanged.
- Node 0 has parent `DUNIT_UI_ROOT`. Every other parent index precedes the child
  and identifies a Column/Row/Stack/Grid or radio group container. Input order defines
  sibling order. The original SDK validator checks keys and shared tree/depth bounds.
- Key/text spans are UTF-8 bytes with explicit lengths. They need no terminator.
  Each span is bounded by `DUNIT_UI_MAX_TEXT_BYTES`. Rust copies them into owned
  strings before retaining the tree. Null spans are allowed only at length zero.
  Container text/message payloads are unused. Buttons deliver their `uint64_t`
  message; other payloads do not create new input behavior.
- `update(user, message)` returns 0 to rebuild/publish, 1 to close, or a negative
  error to terminate. A failed candidate never replaces the last valid View.
  Rust cannot roll back side effects in the C-owned model.
- The run function returns 0 after normal closure, or a negative error status
  (native errors are negative `DUNIT_E*`; negative callback results propagate).
  It closes the SDK window on loop termination. Invalid preflight configuration
  fails before transport. Reentrant/concurrent runs return `-DUNIT_EBUSY`.
- Current gui-v1 supplies one bootstrap/window session per process. After a valid
  initial tree begins its window-open attempt, another run returns `-DUNIT_EPIPE`,
  including after closure or an open failure. Invalid preflight/tree attempts
  do not consume the session. Multiwindow/reopen behavior is not fabricated.

Checkbox, Switch and ToggleButton are controlled widgets. Build their unchecked
state with `DUNIT_UI_CHECKBOX`, `DUNIT_UI_SWITCH` or `DUNIT_UI_TOGGLE_BUTTON`;
use the corresponding `_CHECKED` kind for checked state. These are additive node
kinds; no existing kind, field, layout or export changes. Both states use `text`
as their accessible label and deliver the full `uint64_t message` on activation.
The C update callback decides whether to change state, then build supplies the
next kind. The adapter does not toggle state independently of that model.
SDK focus, pointer release, Space/Enter handling, accessibility and theme styles
apply unchanged. Checkables are leaves, so they cannot be used as parents.

Use `DUNIT_UI_RADIO_GROUP_COLUMN` or `DUNIT_UI_RADIO_GROUP_ROW` as a normal
Column/Row container with its own unique key. Direct child `DUNIT_UI_RADIO` and
`DUNIT_UI_RADIO_CHECKED` nodes use that key as their SDK radio-group identity.
Group names, keys and labels are bounded UTF-8 copied into owned SDK data.
A radio outside a direct group container is invalid. A group cannot have two
checked radios; the ordinary SDK validator enforces selection validity.
Group containers share Column/Row rendering; they add no new widget painter.

Radio `text` is its accessible label and `message` retains all 64 bits. The
C callback owns selection, including whether to accept activation. Each group
has one tab stop; SDK arrows select/wrap within that group. Activating an
already selected radio does not dispatch another message. Groups are independent.
These additional kinds preserve every existing ABI v1 layout/value/export.

OR `DUNIT_UI_DISABLED` into an activation node's `kind`, for example
`DUNIT_UI_CHECKBOX_CHECKED | DUNIT_UI_DISABLED`. It applies to Button, Checkbox,
Switch, ToggleButton and Radio, in either checked state. It maps directly to
the SDK's `View::disabled(true)`: disabled controls reject pointer activation,
are skipped by focus navigation and retain their C-owned checked/selected state.
Disabling a focused or pressed control clears that transient interaction;
removing the flag in a subsequent build enables it again. Theme `:disabled`
styles remain those of the Rust SDK. The flag cannot be used on Text or
containers, including radio groups; it does not disable a subtree. Unsupported
kind bits still return `-DUNIT_EINVAL`. No record field, existing kind value or
export changes; unflagged v1 clients retain their behavior.

## Application layout and styles

`DUNIT_UI_STACK` and `DUNIT_UI_GRID` create ordinary SDK containers, with the
same child ordering, unique-key and depth limits as Column/Row. All four use
the Rust layout engine; the C adapter adds no layout algorithm.

`dunit_ui_run_styled_v1` takes the same arguments as `dunit_ui_run_v1`, followed
by a pointer to `DunitUiStyleV1`. Set its `abi_version` to `DUNIT_UI_ABI_VERSION`
and `struct_size` to `sizeof style`. Its `stylesheet`/`stylesheet_len` describe
bounded UTF-8 DSS (at most `DUNIT_UI_MAX_TEXT_BYTES`), copied at entry. Null at
zero length is valid. The SDK's application layer resolves that sheet against
the installed profile tokens and keeps it during ordinary theme reloads.
Selectors, colors, state rules and layout properties use the exact same DSS
parser, validation, cascade and renderer as Rust applications.

The optional `decorate(user, index, appearance)` callback attaches classes to
the node at that build's array index, using the same `user` as build/update.
It runs once per node on every successful description build; invocation order
is unspecified. `DunitUiAppearanceV1` starts as `{NULL,0}`. Return 0 with an
optional ASCII-whitespace-separated UTF-8 `classes` span, or a negative error.
Positive results are invalid. The same bounded-span checks and SDK identifier
grammar apply. Classes are copied before the next callback; the returned span
must remain readable until that copy. No Rust buffer or borrowed span may be
retained or unwound across the FFI boundary. A null callback means no classes;
id/element selectors in DSS still work.

Width/height (pixels, percentages, fill/auto), min/max constraints, flex weights,
gaps, padding, alignment and Grid column counts can be set through application
DSS. Use classes from the C model to change layout or appearance on update;
the stylesheet itself is fixed for the window session. The example
`/assets/tcc/layout.c` loads `/assets/tcc/layout.dss`, stacks a caption over a
panel and switches a card Grid between two/four columns and two palettes.
Its stylesheet is application data, not a second widget painter.

On native x86_64, appearance is 16 bytes and style is 32 bytes, both aligned to
8. These new declarations and the styled export are generated from the common
schema. Existing v1 structures, callbacks, values and export remain unchanged.
Both run functions share the same busy/session guard. Invalid initial classes
or decoration errors fail before opening; failed update candidates keep the
last valid tree and terminate with an error. DSS/theme failure during opening
consumes the single window attempt, consistent with the original v1 contract.

Call from the C main thread before starting threads that use Rust SDK runtime
arguments/environment. Loader argv/envp must be the valid null-terminated vectors
received by main and remain mapped. The adapter initializes the existing Rust
loader context after musl CRT, without replacing CRT, libc or either allocator.
All callbacks run synchronously on that thread. Their function pointers and
userdata must remain valid throughout the call. They may call C libc normally.

Foreign callers must respect buffer capacities and provide readable byte spans
until Rust copies them after build returns. Do not return pointers to expired
callback stack data, retain/mutate the Rust node buffer, unwind/throw or longjmp
across the bridge. Pointer validity is a C caller obligation, not a new kernel
memory-safety boundary. Rust's reusable node scratch buffer is allocated once
per window attempt; it is not zeroed/reallocated on every message.

## Static packaging and size

`tools/ensure_c_ui.py` uses Cargo's native target and installs `/system/lib/`'s
`libdunit_ui.a` through the existing sysroot. Release LTO/size optimization is
followed by a relocatable link rooted at the schema's C exports. Unreachable
Rust code, LLVM bitcode and unneeded symbols are excluded from the shipped C
archive. The linker script coalesces sections; it fixes no code addresses.
Real undefined libc references remain linkable. C linkers need no LLVM plugin.

The first unpruned archive was 8,094,406 bytes; LTO intermediate archives carry
additional bitcode and are not shipped. The final machine-code archive is
2,057,768 bytes in this checkpoint. The full initrd is about 123 MB and still
fits the existing 128 MiB installer ESP; no partition size/format change was
needed. Minimal images omit the library with the rest of the toolchain.
The checkable-widget extension brings the archive to 2,059,096 bytes (+1,328);
the existing installer ESP still fits without changes.

## Validation

```sh
python3 tools/gen_abi.py --check
cargo test --manifest-path runtime/dunit-c/Cargo.toml --lib
python3 tools/test_c_ui_archive.py  # needs the normal build tools, including lld
python3 tools/test_c_ui.py --accel tcg
python3 tools/test_c_ui_checkables.py --accel tcg
python3 tools/test_c_ui_radio.py --accel tcg
python3 tools/test_c_ui_disabled.py --accel tcg
python3 tools/test_c_ui_style.py --accel tcg
python3 tools/test_c_ui_compat_host.py
python3 tools/test_c_ui_compat.py --accel tcg
```

The wrapper generates direct-Rust reference canvases, performs the normal kernel
quick build, and runs OS execution only through `qemu_test.py`. TCC compiles and
links `/assets/tcc/gui.c` inside Dunit against the installed archive. The actual
program rejects bad versions/trees, detects nested/repeated runs, opens its window,
activates Increment twice with keyboard input and exits through Quit with code 0.
Both full 400×180 client captures (Count 0 and Count 2 with focus) exactly match
the ordinary Rust SDK. C progress output explicitly flushes stdio so harness
waits observe callback progress while the window's synchronous loop is running.

Eight Rust adapter checks, seven existing button regressions, the actual
archive-link test, five header/sysroot checks and nine existing image-profile
checks passed (30 total). Native GUI: ten required
and six forbidden markers; both framebuffer comparisons passed. The prior C
file/errno/spawn/wait contract is also rerun on the same final image.

The compatibility runner freezes the original v1 source and UI header from
`9df23f6` under `tools/fixtures/c_ui_v1/`, with SHA-256 pins. Host TCC compiles
that client before a scoped clean and native Rust runtime rebuild. The retained
x86_64 ELF object enters the initrd unchanged; on-device TCC only links it
against the rebuilt archive. All ten required/six forbidden markers and both
exact Rust framebuffer comparisons remain in force. The runner also verifies
the object and library hashes in the actual initrd and writes its receipt to
`build/c-ui-compat-result.json`. No compiled fixture is committed.

On 7 November 2026, this proof passed under QEMU TCG, together with six fixture
integrity/contract checks and eight Rust adapter tests. This covers the frozen
four-kind v1 client with the current compiler and architecture; it does not
establish compatibility for every historical compiler or the complete C SDK.
The remaining widget/style/transfer matrix keeps M7.10 open.

The checkables scenario compiles `/assets/tcc/checkables.c` inside Dunit, toggles
each control on and off with Tab/Space/Enter and closes via Quit. On 7 November
2026, QEMU TCG passed eleven required/six forbidden markers and exact 400×240
Rust-reference comparisons for initial, all-checked and all-unchecked states.
The original frozen-client compatibility scenario also passed against this
extended library, including both original framebuffer comparisons. Host checks:
eleven adapter tests, fifteen existing button/checkable regressions, six frozen
fixture checks, five header/sysroot checks, nine image-profile checks and one
archive-link test (47 total). Adapter tests cover pointer release, declined
C-owned changes, full-width message IDs, accessibility and malformed leaves.

The radio scenario compiles `/assets/tcc/radio.c` inside Dunit and verifies
rejection of a doubly checked group before opening the window. It then exercises
two independent groups (Column and Row), six arrow selections with wraparound,
one tab stop per group, suppressed reactivation and a normal Quit exit. Three
400×240 screenshots match direct Rust SDK pixels exactly. A message using
`UINT64_MAX` crosses the native C callback unchanged. This adds four node kinds
and 744 bytes to the shipped archive (2,059,840 bytes); the ESP size is unchanged.
The radio gate keeps twelve required/six forbidden markers. Host checks total
52: sixteen adapter, fifteen button/checkable, six fixture, five header/sysroot,
nine image-profile and one archive-link test. The frozen v1 object also passes
its separate native gate with both original pixel comparisons against this
library. Other widget/style/transfer bindings remain outside this increment.

The disabled-controls scenario compiles `/assets/tcc/disabled.c` inside Dunit,
tabs past disabled controls, enables them, activates a button and checkbox, then
disables them again while retaining checked state. Its three 400×240 captures
match direct Rust SDK pixels exactly (nine required/six forbidden markers).
Host tests also exercise all nine activation kinds, disabled radio tab stops,
rejection of invalid flag combinations and cancellation when a press is disabled
before release. Checks total 56: twenty adapter, fifteen button/checkable, six
fixture, five header/sysroot, nine image-profile and one archive-link test.
The shipped archive is 2,060,368 bytes (+528); the installer ESP is unchanged.

The layout/style block passes 64 host/Rust checks: 28 adapter, 15 existing
button/checkable, six frozen fixture, five header/sysroot, nine image-profile
and one archive-link test (both public entry points are retained and linked).
The native layout lab passes ten required/six forbidden markers and three
exact 560×360 Rust-reference comparisons. Hit rectangles independently confirm
that the Grid reflows from two rows to one. The lab also rejects invalid style
configuration/classes before opening and proves that the styled and original
entry points share the one-session guard. The frozen older C object passes
its own QEMU TCG run after a forced rebuild, including both original pixel
comparisons. Archive: 2,062,380 bytes (+2,012), still in the original ESP.
Input widgets, other advanced widget bindings and transfer/file capabilities
remain later M7.10 work; this completes the application DSS/classes and core
Column/Row/Stack/Grid layout surface, not the full milestone.
