# M7.8: live protocol sources and damage acceptance

The control codec is `protocols/gui-v1/src/control.rs`. Clients import its
types and encode/decode methods for CAP1, INP1, RLD1 and NOT1. GUIC stays in
`libdunit`; DGUI framing and the surface protocol live in `gui_protocol_v1`.
M7.7's XFR1 codec and generated transfer values have their own
[contract](m7-7-transfers.md).

`tools/check_gui_protocol.py` derives control tag names and little-endian
values from the canonical declarations. It scans production Rust under
kernel, userspace, runtime and protocols, rejecting local redefinitions and
direct byte/raw-byte or integer copies, including escaped bytes and integer
radices. Comments and diagnostic strings are skipped. External `tests/`
golden fixtures retain independent expected wire bytes. Wire conformance is
also checked by the protocol crate's tests.

```sh
python3 tools/check_gui_protocol.py
python3 tools/test_gui_protocol_source.py
python3 tools/gen_abi.py --check
cargo test --manifest-path protocols/gui-v1/Cargo.toml
python3 tools/test_sdk_gui_fixture.py component --accel tcg
```

Both source checks run in the existing CI protocol-contract step. Mutation
tests insert duplicate byte/numeric constants at each package depth, check
raw/escaped forms, and verify that comments, golden fixtures and ordinary
imports remain valid. Missing or duplicate canonical declarations fail.

## Measured live path

The SDK commits buffer damage through DGUI. The compositor adopts only
committed pixels, updates its CPU texture, plans safe content copies and
presents the affected framebuffer rectangles. This path was already
implemented before M7.8's final acceptance; no new wire layout was introduced
by the source gate.

On 4 November 2026, the native component fixture passed in QEMU TCG with
512 MiB and one vCPU. It verified a real model/view update, an unchanged
update, backing reuse, maximize/restore and clean child exit. Both partial
presentations reported:

```text
[DAMAGE] partial composed=15360 presented=15360 texture=15360 screen=2073600
[DAMAGE] partial frame OK
```

The updated area is 0.74% of the screen, 135 times fewer pixels than a full
presentation. These are pixel-work counters, not a frame-time speedup claim.
The fixture enforces exactly two such presentations. All 13 component
markers and all 30 full GUI smoke markers passed, with no forbidden failures.
Host checks passed: 100 protocol tests, 5 damage planner/cursor tests and
6 Python source-gate tests. ABI generation remained in sync.

The retained fast path requires stable scene geometry, focus, pointer and
shell state, with opaque, unscaled and unobscured content. Geometry changes,
moving cursors, active DnD, menus, notifications, animation, transparency and
unsafe occlusion use the existing full painter. Legacy clients with empty
damage retain continuous updates. Those bounds preserve rendering correctness;
M7.8 acceptance does not assert partial composition for every desktop state.
