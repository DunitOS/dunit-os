# Dunit SDK

The SDK is a native `no_std + alloc` runtime over `libdunit`, with typed Rust
components and a generated C bridge to the same widget runtime. Ordinary GUI
applications are independent ELF processes; the compositor owns display/input.

- [Native API](native-api.md): entry, streams, files, time, child processes and wait-set.
- [Tooling](tooling.md): cargo dunit, manifests, native tests and desktop discovery.
- [Components and windows](layout-window.md): rendering, layout, scrolling, workspace and buffer lifetime.
- [Controls](widgets.md): buttons, choices, numbers, menus, collections and file dialogs.
- [Input](input.md): routing, focus, text editing and compatible device/pointer extensions.
- [Themes](themes.md#themes): DSS tokens, effects, reload and appearance transitions.
- [Desktop and transfers](desktop-transfers.md): shell decision, configuration saves, clipboard and file authority.
- [C API](c-ui.md): generated frozen v1 declarations, callbacks, transfers and packaging.
- [gui-v1 protocol](../../protocols/gui-v1/README.md): wire requirements and native binding.

The manuals describe implemented contracts and their limits. The
[technical roadmap](../../DUNIT_OS_TECHNICAL_ROADMAP.md) owns remaining work.
Existing per-call wait-set registrations are level-triggered; persistent/edge
registrations, thread/sync wrappers and standalone SDK distribution remain open.
GUI sessions currently own one process inbox and support one window attempt.
A future std port, IME, broader POSIX profile or privileged shell protocol needs
its own contract; this SDK does not supply those facilities.
