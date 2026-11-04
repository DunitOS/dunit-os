# M7.9: shared style, retained in-process shell

The compositor keeps its software composition, damage, blur, input routing and
transfer broker. Panels, dock, launcher and menus stay in the same process.
They use the same validated DSS registry and selected theme as SDK applications.
This is the retained-shell option explicitly allowed by M7.9, not a claim that
an external shell has been implemented or accepted.

## Appearance boundary

All shell palette, cursor, decoration/control geometry, gradient deltas,
shadow color/offset, title placement, component intrinsic padding/widths,
text baseline/padding, icon spacing/raster minimums, running markers, group
previews, clock marks/hand proportions and initial window cascade metrics
come from `assets/dwm/sdk/shell.dss`. Selected profiles override them. Explicit
legacy TOML fields retain precedence; sparse scalar saves retain inheritance.
Numeric types/bounds and geometry algorithms stay in Rust, without fallback
appearance values. Configuration/reload prepares resources before committing a
scene; frame painting uses cached typed values. Invalid candidates retain the
previous scene. Control painting and pointer hit tests use one clipped rectangle.

Fixed arithmetic remains where it describes a mechanism: ARGB channel masks and
255 coverage; one raster pixel and inclusive endpoints; division by two for
centering; twelve clock-hour samples and clock periods; font width estimation;
SVG/BMP format dimensions; resource budgets, protocol IDs, queue bounds,
validation bounds and self-test deadlines. These are not private theme defaults.
The source guard checks private color literals, direct paint geometry/style
arguments and appearance fallbacks/assignments. Its mutation tests are a
regression guard, not a Rust type checker or a replacement for review.

Startup capability proofs, window state/session persistence, keyboard navigation
and asset loading were separated from `main.rs` into focused production modules.
Composition still runs on the lower SDK layers (`libdunit`, shared protocol,
`dunit_style`, `dunit_ui`, text/image/render primitives); it does not become a
component widget tree. The remaining event loop is an orchestration boundary;
this milestone does not claim all repository file-size debt is resolved.

## Why extraction is not enabled

The current versioned surface protocol supports ordinary client windows and
input/transfer/lifecycle transport. It does not implement panel layers, reserved
work areas/struts, shell roles or external DWM-policy operations. The
`0x1000–0x1fff` policy range is reserved, not a usable implementation. A plain
SDK window cannot take ownership of the current panel/dock/menu behavior.
Adding privileged policy messages would require a separately reviewed protocol
contract. M7.9 does not need that change to retain the shell.

A bounded native experiment (`sdk_shellprobe`) measures the existing SDK route:
a themed 640×28 panel-like client, a separate surviving client, 24 keyboard-driven
render/commit/FrameDone transactions, then an intentional invalid-opcode fault
without graceful close. Only after observing the exact compositor reap marker
does the harness arm the survivor to start its new publications. The compositor must reap that exact process with
`kind=3 code=-4`; the surviving client must perform 24 more commits and exit
successfully. The verifier rejects another post-ready client fault, lost
publication/completion or clock failure. Boot-smoke faults before the probe
identity are outside this experiment. No production shell or protocol is
replaced by the fixture.

This establishes client-process isolation and the measured cost of the available
publication route. It cannot establish full-shell equivalence or prove that a
panel will share correct z-order, reservations, focus and menu routing. That
acceptance gate is therefore not met: keep the shell in-process. The experiment
is not evidence that moving *all* shell behavior would be equally fast.

Boot currently needs `init` plus `gui_server` to construct its shell scene.
Extraction would add shell executable(s), manifests, spawn/readiness ordering,
policy transport and restart/reservation recovery. The experiment exercises two
normal startup clients through existing manifest discovery, with no new init
service. Successful ordinary-client restart/isolation is insufficient to adopt
the additional shell bootstrap and recovery contract.

## Reproduce

Run OS scenes through the existing wrapper and harness:

```sh
python3 tools/test_sdk_gui_fixture.py latency --accel tcg
python3 tools/test_sdk_gui_fixture.py shellprobe --accel tcg
python3 tools/sdk_shellprobe_report.py timing.log serial.log --output probe.json
python3 tools/test_sdk_gui_fixture.py theme --accel tcg
python3 tools/check_shell_style.py
python3 tools/test_shell_style_source.py
python3 tools/test_shellprobe_report.py
```

The performance acceptance record and raw trial summaries are in
[gui-server acceptance](../performance/gui-server-acceptance.md). Target ≤16 ms
is separate performance work; retaining a measured non-regressing shared-style
shell does not imply that target has been reached.
