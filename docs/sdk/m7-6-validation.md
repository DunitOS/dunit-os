# M7.6 widget integration validation — 2026-11-01

This increment adds controlled virtual tables and sorting, shared in-place rename,
composable file dialogs, complete child-message mapping, explicit focus order and
mnemonics, capture/bubble dispatch, timed scrollbar hiding, grapheme editing,
soft wrapping and state-aware flex/grid sizing. Contracts and limits:

- [Tables and rename](tables.md), [file dialogs](file-dialogs.md).
- [Focus](focus-order.md), [event routing](event-routing.md).
- [Text gestures](click-selection.md), [grapheme editing](text-input.md),
  [soft wrap](soft-wrap.md).
- [Layout](layout.md), [scrollbars](scroll-areas.md).

## Host gates

All 385 tests passed, with no compiler warnings in these runs:

| Crate | Tests |
| --- | ---: |
| dunit-ui | 22 |
| dunit-widgets | 40 |
| dunit-render | 61 |
| dunit-component | 249 |
| dunit with gui | 13 |

Commands: `cargo test --manifest-path runtime/<crate>/Cargo.toml`, adding
`--features gui` for dunit. Test/dev debug info and incremental compilation were
disabled to keep the execution workspace within its disk budget. These flags do
not disable assertions or skip tests. ABI generation check also passed:
90 syscalls, 25 errno, 7 rights. No ABI, kernel, HAL or gui-v1 source was changed.

## Native gates

All launches use `tools/test_sdk_gui_fixture.py <fixture> --accel tcg`, through
`tools/qemu_test.py`. Required markers were preserved; each fixture also rejects
kernel/application failures. There are 135 required marker checks across the
five contracts:

| Fixture | Required markers | Behavior |
| --- | ---: | --- |
| collections | 40 | 10,000-row table, idle timer/focus wake, raw F4 capture+bubble exactly once, real header sorting clicks, F2/edit/commit, virtual End, resize, real filesystem picker, overwrite/decline/cancel without writes, single/double/triple click, replacement/undo, wrap navigation and resize, `[WIDGETS] ok`, exit 0 |
| multiline | 19 | Existing multiline editing, selection/history, wheel and resize |
| menus | 31 | Existing select/menu/submenu/context/tooltip interactions and timers |
| workspace | 38 | Existing tabs, splitter, modal/default/cancel and opener restoration |
| theme | 7 | Dark/light/restore and rejected candidate retaining the working frame |

Theme screenshots additionally matched all three existing framebuffer pixel
references exactly. Host wrapped paint/caret tests match an explicit-LF reference;
retained table/editor pixels match fresh renders after resizing in both themes.
The new collections contract scopes USER-FAULT rejection after terminal readiness,
like the existing scroll fixtures: thread/VM boot negative tests intentionally fault
isolated children. It additionally requires their successful completion markers.
The initial overbroad global ban rejected those expected boot faults even though
all GUI steps and exit 0 completed; the corrected contract was rerun.
The new collections fixture requires `[DF-TEST] syscall+irq flags preserved` and
captures rename, table, picker, overwrite, selection and wrapped-text screenshots.

## Milestone gates still open

This is not a declaration that the entire M7.6 milestone is closed. Held-key
pressed states, keyboard-focus loss and native pointer modifiers require the
[gui-v1 lifecycle decision](m7-6-input-extension-proposal.md). Cross-surface popup
roles and an actual system IME are separate contracts. The roadmap already places
system clipboard in M7.7 and full client migration/removal of manual client hit
tests in M7.11; neither is simulated here. File chooser native directory snapshots
explicitly fail with ENOBUFS at the existing maximum-buffer limit rather than
silently claiming a complete truncated listing. No wire or privilege change has
been made while the lifecycle proposal awaits approval.
