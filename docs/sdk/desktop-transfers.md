# Desktop, configuration and transfers

Desktop policy lives in userspace. Configuration, the shared theme registry and
the authenticated transfer broker are the sources used by applications and shell.

## Shell appearance

The shell's appearance data is in `/assets/dwm/sdk/shell.dss`, shared with native
SDK theme loading. The registry adds 147 numeric metrics, four opaque paint
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

Window controls, title/gradient placement, component intrinsic geometry,
icon/text spacing, clock/running markers and shadow paint use the same
registry. [Scalar saves](desktop-transfers.md#settings-persistence) preserve inherited fields and explicit
import intent. The [completed M7.9 decision](desktop-transfers.md#retained-shell-decision) retains the
shell in-process after a bounded SDK publication/isolation experiment.
[Final measurements](../performance/gui-performance.md#retained-shell) retain all trials.

## Settings persistence

A Settings value includes fallback values for omitted TOML fields. Serializing
the whole value on every save used to write these fallbacks as explicit
`[theme]`, `[layout]` and `[effects]` overrides. Changing a notification setting
could therefore detach the desktop from its selected DSS profile.

Scalar writes own only their selected declarations:

| Operation | API | Intent |
| --- | --- | --- |
| Edit a staged snapshot | `save_changes(before, after)` | Fields whose serialized values changed |
| Explicit setter or partial theme import | `save_overlay(text)` | All supplied scalar fields, including values equal to defaults |
| Read a partial import while retaining intent | `load_overlay_patch(current, path)` | Candidate Settings plus canonical explicit declarations |
| Compatibility save | `save(settings)` | Differences from the current stored scalar values |
| Complete document replacement | `save_document(text)` | The complete validated source, as before |

The pure helpers `scalar_changes` and `apply_scalar_overlay` let callers stage
and inspect a patch. Overlay field ownership comes from the existing serializer,
not a second setting-name schema. Partial import validation uses the same edit
rules as before; it does not require an application registry. Complete document
imports retain their separate registry-aware validation.

The source patcher keeps table order, leading comments, unrelated assignments,
application registry, panels, components and other independent tables. It
replaces edited declarations in their existing table and appends new fields or
tables only when necessary. Changed declarations use canonical formatting;
untouched text stays intact. Multiple staged edits of a field coalesce into one
declaration. A no-op snapshot save validates the active document without
rewriting it. Invalid candidates leave the prior source untouched.

Snapshot saves preserve unrelated source changes made since the snapshot.
Explicitly edited keys keep existing last-writer-wins behavior. This is a source
patch operation, not a new filesystem transaction or cross-process lock.
Source and result are validated against installed manifests before writing;
the bounded complete-document writer remains the final write path.

GUI scalar actions and the resolution-picker test use `save_changes`. GUI
partial-theme imports retain their explicit patch. Terminal setup stages an
explicit patch for `set`, menu actions and `import`; `save` verifies only those
fields on read-back and then clears the staged intent. Full-document import
and reload discard staged changes as before.

Legacy scalar forms still display declared/fallback Settings values; they are
not an inspector of computed DSS values. The writer now preserves inheritance
even when a form holds fallback or older unedited values. Fully inspecting
resolved style remains separate work.

## Retained shell decision

The compositor keeps its software composition, damage, blur, input routing and
transfer broker. Panels, dock, launcher and menus stay in the same process.
They use the same validated DSS registry and selected theme as SDK applications.
This is the retained-shell option explicitly allowed by M7.9, not a claim that
an external shell has been implemented or accepted.

### Appearance boundary

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

### Why extraction is not enabled

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

### Reproduce

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
[gui-server acceptance](../performance/gui-performance.md#retained-shell). Target ≤16 ms
is separate performance work; retaining a measured non-regressing shared-style
shell does not imply that target has been reached.

## GUI application boundaries

The window-client inventory is `gui_client`, `gui_demo`, `gui_calc`, `gui_stat`,
`gui_settings`, `gui_files` and `gui_terminal`. They use generated SDK entry,
SDK I/O and owned window resources. Applications no longer import `libdunit` or
encode gui-v1 window packets. The source guard scans every Rust module of each
client, including helper modules.

| Client | Application responsibility | SDK responsibility |
| --- | --- | --- |
| Client | independent click counter and launch title | retained Button, input states, layout, buffer/resize lifecycle |
| Demo | notification labels and app palette | retained Buttons, bounded notification request, native presentation |
| Calculator | arithmetic, display export and drag gesture | retained key grid, capture routing, fonts, layout and presentation |
| Stat | sampled system facts | retained view, timed native event wait and presentation |
| Settings | source-preserving document edits, validation and persistence | Buttons, Switches, Inputs, scrolling, selection/history, clipboard/DnD |
| Files | icon grid, preview, selected inode custody and filesystem operations | canvas snapshot, cached fonts, context menu, filename and confirmation dialogs, text clipboard/DnD |
| Terminal | ANSI cell grid, scrollback and cell selection | window/resize protocol, cached glyph masks, owned PTY and child process, bounded pending input |

`gui_server` is the compositor service, with privileged display/input and
protocol-mechanism responsibilities. Its separate lower-layer migration remains
an explicit roadmap item. `gui_shbuf_peer` is a capability-mechanism probe, not a
window application: it deliberately tests mapping/attestation and denied direct
framebuffer access. Neither is disguised as a retained GUI component.

### Shared mechanisms

`canvas::Font` owns the existing bounded `dunit_text::GlyphCache`; it does not
introduce another rasterizer. Configured candidates are read through bounded SDK
file I/O, and malformed candidates advance to the caller-specified fallback.
Terminal cells and Files labels reuse coverage between frames.

`ComponentWindow::set_canvas` retains private application pixels beneath ordinary
SDK controls. Menus, focus, modal defaults, text editing and clipboard routing
still use the existing component runtime. Published shared buffers are never
used as mutable application state. The overlay is premultiplied and composited
through the shared clipped raster kernel; damage repair still belongs to the
native window adapter. The caller redraws custom document content after resize.
A geometry-bound canvas defers publication when a configure overtakes its
snapshot; it returns the new geometry to the application instead of publishing
a frame without its document pixels.

An explicit external transfer offer supports Files' capability receiver. SDK
Inputs take precedence while a filename dialog is open; removing the Input
restores the file offer. There are no competing SDK/application registrations
or duplicate text-drop decisions.

`pty::Pty` owns the native PTY master. `Child` owns its completion status; the
terminal reaps a completed child once and preserves final output. Closing a live
command terminates and waits for it. `PendingInput` preserves short-write suffixes
and enforces the existing bounded queue/service budget.

Settings queues persistence actions until the input dispatch returns. Startup
configuration/constructor temporaries are released before its event loop. Its
document model contains source lines and selection identity, while caret,
selection and history belong exclusively to SDK Inputs. Theme import explicitly
reloads the SDK profile and acknowledges it after presentation; screenshot
contracts wait for that publication, not just the earlier settings-file save.

The native SVG path exposed another stack-budget problem: raster paint/pipeline
temporaries remained live in recursive traversal frames. Shape painting and
layer composition have separate call frames; SVG depth, pixel and feature
limits and rendering algorithms are unchanged. Stock SVG icons run on the same
64 KiB user stack; no kernel stack or privilege change was used.

Short formatted SDK print records are buffered before native writing. The serial
console prefixes each native write, so emitting individual formatter fragments
would split existing complete-line diagnostics and marker records.

The calculator captures arithmetic keys before focused Button defaults: Enter
continues to mean Equals after a mouse click has focused a digit. Other clients
keep ordinary SDK keyboard activation.

### Packaging

The seven GUI clients use size-oriented whole-program LTO. The initrd packer
continues stripping executable copies while retaining host build symbols. The
installer ESP size, archive format, syscall ABI, gui-v1 wire format and privilege
model are unchanged.

## CLI migration

`dufetch` uses `#[dunit::main]`, checked SDK output and the existing
`system`, `process` and `fs` adapters. Its logo, logical ANSI colors, labels,
spacing, number formatting and conditional facts remain application presentation.
The pure `report.rs` accepts a fact snapshot and an SDK `Write` implementation;
it performs no syscalls. There is one formatter in production.

The application no longer owns a loader entry/panic handler, raw stdout writes
or a second cwd reader. Partial writes are completed and errors propagate to the
SDK entry diagnostic/exit status. A failed stats/cwd query no longer silently
pretends zero stats or `/`. Display absence still omits the display line.

`system::display_info()` wraps the existing framebuffer query and returns only
width, height and pitch, without exposing the mapping address. It introduces no
syscall, kernel field, wire format or privilege change. This is not a new uname
contract: unavailable OS/kernel/platform identity is still not fabricated.

`wc_demo` already uses native musl stdio, which is the C SDK's file/stream API.
Its migration consists of consuming the generated `<dunit.h>` umbrella; the
portable counting/file logic and native libc implementation are unchanged.
There is no parallel C stream wrapper and no rewrite merely to increase scope.

### Acceptance

Before removing the old `dufetch` entry/I/O code, both original applications
passed `sdk_cli_migration_markers.json` in QEMU TCG. The C test parent, itself
compiled by TCC inside Dunit, redirects real stdout/stderr using native spawn
file actions, waits for and validates each child status, and reads back files:

- `dufetch` exits 0 with nonempty stdout and empty stderr. Its actual UTF-8 bytes
  are exported as bounded hex lines, avoiding serial tagging/chunking ambiguity.
- `wc_demo` with its default path produces exactly
  `wc demo.txt -> 3 lines, 13 words, 61 bytes\n` and exits 0.
- An explicit `/tmp/m711-wc.txt` produces the corresponding exact line and exits 0.
- A missing parent produces empty stdout, exact
  `fopen(w): No such file or directory\n` on stderr and exits 1.

The identical native contract passes after migration. The existing, unchanged
`tools/minimal_markers.json` also passes on actual rebuilt Minimal release ISOs
before and after the application change; image composition is checked separately.
Minimal retains six files and no GUI runtime dependency.

`tools/fixtures/dufetch_stdout.json` records the old source commit/hash and five
complete stdout snapshots produced by that old renderer before migration. It
covers unavailable facts, seconds/minutes/hours, missing/invalid display geometry,
unknown tick frequency, large counters, Unicode cwd, optional IPC/NIC lines and
ANSI swatches. The fixture contains expected bytes, not a retained legacy
implementation. The current production formatter must match every byte for the
same input. A writer limited to three bytes verifies complete writes; injected EIO
at byte 17 verifies propagation and the exact written prefix for every snapshot.
The unchanged C program also compiles/runs against the umbrella header on Linux
with identical output and file bytes.

Live kernel counters, uptime and PID cannot be byte-identical across processes
and reboots. Their native format is validated and only those numeric values are
masked before comparing with the pre-migration report. All units, punctuation,
labels, ANSI, Unicode logo, spacing, display geometry and cwd remain exact. The
fixed-snapshot comparison additionally checks every numeric value without masking.
Do not replace the frozen baseline merely to hide a regression.

## Clipboard and drag/drop

The accepted M7.7 model uses `gui_server` as an authenticated broker. It checks
kernel-reported sender PID, clipboard focus and actual source pointer capture.
`FEATURE_DATA_TRANSFER` (`1 << 2`) enables the sideband only after negotiation;
legacy clients receive their established stream.

`XFR1` is a 32-byte little-endian envelope: magic, kind u16, reserved-zero u16,
request u64, transfer u64, arg0 u32, arg1 u32. `abi/transfers.abi` owns kinds,
actions, statuses and file operations; `tools/gen_abi.py` generates Rust/C values.
The codec and broker state machine live in `gui_protocol_v1`. Large data uses
kernel-attested handles copied into bounded private receiver memory. A handle
number written in a payload supplies no authority.

Representations include UTF-8 text, normalized URI lists, Files, RGBA and custom
MIME. Limits are 16 formats, 256 files, 4096 path bytes, 16 MiB per blob and
32 MiB for payload copies of active clipboard reads. Invalid lengths, UTF-8, RGBA
dimensions, duplicate IDs and illegal states fail. Per-recipient delivery order
is preserved; one full IPC queue cannot block other recipients.

Drag/drop requests data only after an accepted drop. Crossing workspaces retains
source capture. Escape, peer loss, an unsuitable target or deadline expiration
ends the operation and releases escrow. After pointer release the selected target
may reply until the deadline; that transfer cannot move to another target.
Clipboard materializes every representation before ACK, so Copy survives source
exit. File Cut keeps the source until a successful Move.

### File authority and lifetime

The approved hybrid delivers quoted paths to text receivers and real attenuated
file/directory capabilities to file receivers. Syscalls 91–94 are FileCapOpen,
FileCapOp, IpcSendHandle and IpcReceiveHandle. Receivers enable attached IPC first;
ordinary receive does not consume a message carrying an attachment. Delivery is
atomic and failed sends roll back the duplicate. TRANSFER is required and rights
can only narrow.

A capability names the opened backend object rather than a path string. Relative
lookup rejects `..` traversal and cannot escape its subtree. MemFS keeps an opened
unlinked file alive; DunitFS invalidates a removed inode rather than retargeting a
descriptor to a reused slot. Files receivers obtain READ|TRANSFER; WRITE needed
for Move remains in broker escrow. Move is an atomic no-overwrite rename within
one backend. Cross-backend Move returns EOPNOTSUPP. The virtual VFS root `/` is
not one backend; use an actual directory such as `/tmp` or `/persist`.

Directory reads retain the existing maximum of 1024 entries and 64 UTF-8 bytes
per name. An overlong name fails rather than becoming a truncated capability.
Copy uses a bounded task: one 4 KiB block or one node per poll, at most 64 levels,
16384 nodes and 256 MiB. Cancellation removes only objects created by that task,
matched by identity, one per poll.

Copy/Move/Link are negotiated actions. Built-in file receivers support Copy/Move;
the filesystem has no link-creation operation. Custom receivers may implement
Link. Capabilities use ordinary RAII handle lifetime: transfer completion releases
broker escrow but cannot revoke a received handle. A client retains only the
rights actually delivered. Existing ambient path syscalls remain available; this
is not a new process filesystem sandbox. File receivers perform transferred
operations through capabilities rather than restoring authority from display paths.

### SDK integration

`Window` has one authenticated IPC pump, including while awaiting FrameDone.
Transfer packets are dispatched through the shared typed client/session API.
Input paste proposals bind to exact editor/focus/revision and form one undo action.
Read-only fields can export text; password fields cannot. Collections export
selected labels or TSV. Starting a text drag inside an existing selection differs
from beginning a new pointer selection.

Files uses system clipboard, multiple selection, a capability for its selected
destination and cancelable background Copy/Move. Received directories provide
no ambient path fallback. Terminal inserts text and properly quoted paths through
the shared shell grammar. Settings opens a transferred TOML as a draft requiring
Apply; Calculator exports its read-only result. Dock chooses an opener from
`open_formats`, spawns only after drop and delivers authority to that exact PID
after registration. The policy does not dispatch by application name.

### Verification references

`tools/dnd_matrix.json` declares all production GUI pairs; `test_dnd_matrix.py`
rejects desktop entries absent from it. `tools/test_sdk_gui_fixture.py matrix`
runs real binaries under distinct identities and observes actual clipboard/file/
shell receiver content. The `transfer` fixture covers lazy delivery, Escape, bad
targets, RGBA/custom MIME, Link, workspace changes, file identity and dock. The
`clipboard` fixture covers controlled text paste/cut/drop/undo, UTF-8, read-only,
password and source-exit lifetime. `tools/m6_handle_markers.json` verifies rights,
IPC rollback, real Copy and bounded cancellation. DunitFS prepare/reboot/verify
contracts operate on private QEMU disk copies and check capabilities plus Sync.

```sh
python3 tools/test_sdk_gui_fixture.py transfer --accel tcg
python3 tools/test_sdk_gui_fixture.py clipboard --accel tcg
DUNIT_MATRIX_CASES='gui_files->gui_files' DUNIT_MATRIX_MOVE=1 \
  python3 tools/test_sdk_gui_fixture.py matrix --accel tcg
DUNIT_MATRIX_CASES='gui_files->gui_terminal' DUNIT_MATRIX_QUOTED=1 \
  python3 tools/test_sdk_gui_fixture.py matrix --accel tcg
```

The Move case asserts both source removals; the quoted case passes a filename
containing Unicode, spaces and a quote to a real shell. Changed overlays require
a fresh image. Harness readiness waits for clipboard ACK and committed geometry;
QEMU virtual key-release timing is not a guest-processing barrier. All OS builds
and boots use `tools/qemu_test.py`.

The 3 November 2026 record reports 449 Rust tests, 33 Python tests, all 49 production
GUI pairs and the transfer/clipboard/handle/DunitFS/GUI regressions passing on TCG.
The later GUI migration rechecked seven source/receiver role pairs with actual
content/files, plus original smoke, Settings, SVG, console and canvas contracts.
These historical checks do not replace verification after new source changes.
