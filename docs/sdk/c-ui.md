# C UI v1: the first native component bridge

`<dunit.h>` now includes generated `<dunit/ui.h>`. Link a native C program with
`-ldunit_ui` and call `dunit_ui_run_v1(argc, argv, envp, width, height, &callbacks)`.
The static library is the existing Rust `ComponentWindow`, component model,
DSS theme, renderer and window transport behind a synchronous C adapter.
It implements no second layout engine, widget painter or GUI handshake.

This first subset describes Text, Button, Column and Row nodes. It opens one
native themed window and dispatches button messages back to the C-owned model.
Other widgets, application stylesheet parameters and C transfer/file-capability
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
  and identifies a Column/Row. Input order defines sibling order. The original
  SDK validator checks keys and the shared tree/depth bounds.
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

## Validation

```sh
python3 tools/gen_abi.py --check
cargo test --manifest-path runtime/dunit-c/Cargo.toml --lib
python3 tools/test_c_ui_archive.py  # needs the normal build tools, including lld
python3 tools/test_c_ui.py --accel tcg
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

Cross-rebuild compatibility using a retained older C object and the remaining
widget/transfer matrix are still separate acceptance work for all of M7.10.
