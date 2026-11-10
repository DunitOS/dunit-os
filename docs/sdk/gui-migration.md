# M7.11: native GUI clients

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

## Shared mechanisms

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
layer composition now have separate call frames; SVG depth, pixel and feature
limits and rendering algorithms are unchanged. Stock SVG icons run on the same
64 KiB user stack; no kernel stack or privilege change was used.

Short formatted SDK print records are buffered before native writing. The serial
console prefixes each native write, so emitting individual formatter fragments
would split existing complete-line diagnostics and marker records.

The calculator captures arithmetic keys before focused Button defaults: Enter
continues to mean Equals after a mouse click has focused a digit. Other clients
keep ordinary SDK keyboard activation.

## Packaging

The seven GUI clients use size-oriented whole-program LTO. The initrd packer
continues stripping executable copies while retaining host build symbols. The
installer ESP size, archive format, syscall ABI, gui-v1 wire format and privilege
model are unchanged.

## Validation

Before migration, the existing GUI smoke contract passed all 30 required markers.
The original production Settings, theme-import and SVG/icon mouse contracts also
passed. After migration the same required budgets passed. Mouse coordinates and
scroll gestures follow the retained layout; readiness waits follow completed
publication. Production contracts additionally forbid user faults. No content,
inode or required marker check was removed.

| Native QEMU TCG contract | Result |
| --- | --- |
| GUI smoke, original Full image | 30 required markers; startup, shared-buffer isolation, ARGB, resize/restore, notifications, resolution change and reaper |
| Settings production mouse | 8 required markers; wheel scrolling, fixed navigation, saved Lua toggle, columns/monocle and live resize |
| SVG/theme production mouse | 10 required markers; three saved/applied themes, SVG wallpaper, seven compositor icons and six Files icons |
| Theme budget on the identical SVG action replay | all 7 required markers also satisfied |
| Selected transfer matrix, one VM | all 7 pairs below; source data and receiver content/files checked by the native observer |
| Console/PTY, Full image | 15 required markers; argv with a spaced executable path, CLI/TUI stdin/stdout, natural completion, live-child cleanup and legacy shell |
| SDK canvas resize | 8 required markers; configure overtakes an old snapshot, zero completed frames before replacement, exactly one completed frame for maximize and one for restore |

The seven matrix pairs are `gui_demo→gui_stat`, `gui_calc→gui_settings`,
`gui_stat→gui_client`, `gui_files→gui_files`, `gui_files→gui_terminal`,
`gui_terminal→gui_settings`, and `gui_settings→gui_files`. They cover the active
source/receiver roles and unsupported roles without repeating all 49 pairs.
Calculator preparation focuses a key, clears it and enters `12+3` plus Enter;
the receiving field must contain **15**, rather than merely the initial zero.
Files receivers are verified through actual filesystem data; text receivers are
verified through the system clipboard or a real shell redirection.

Host checks: **109** SDK/render/terminal-model tests, **13** SVG tests and **60**
C adapter tests, all passing. They include cached-mask identity and exact pixel
parity, clipped premultiplied composition, buffer repair, bounded/partial PTY
input, source-preserving document proposals and existing ABI/callback checks.
The deleted filename-helper test belonged to the deleted manual editor; SDK
field/clipboard coverage and the native filename receiver replace that path.

Representative commands (all OS builds and boots use `qemu_test.py`):

```sh
python3 tools/qemu_test.py --build --markers-file tools/gui_smoke_markers.json \
  --accel tcg --ready-marker 'gui_server: desktop ready' \
  --boot-timeout 240 --timeout 360 --linger 25
python3 tools/qemu_test.py --build --image-profile dwm \
  --markers-file tools/settings_ui_markers.json --accel tcg \
  --ready-marker 'gui_server: desktop ready'
python3 tools/qemu_test.py --markers-file tools/svg_ui_markers.json \
  --accel tcg --ready-marker 'gui_server: desktop ready'
DUNIT_MATRIX_CASES='gui_calc->gui_settings,gui_files->gui_files,gui_files->gui_terminal,gui_terminal->gui_settings,gui_settings->gui_files,gui_demo->gui_stat,gui_stat->gui_client' \
  python3 tools/test_sdk_gui_fixture.py matrix --accel tcg
python3 tools/test_sdk_gui_fixture.py console --accel tcg
python3 tools/test_sdk_gui_fixture.py canvas --accel tcg
```

The last validated Full initrd size is recorded below. Generated serial logs, images and
screenshots remain build artifacts, not repository payload.

Last validated Full initrd: **124040636 bytes**; installer ESP remains **128 MiB**.
