# Saving scalar settings without pinning DSS inheritance

A Settings value includes fallback values for omitted TOML fields. Serializing
the whole value on every save used to write these fallbacks as explicit
`[theme]`, `[layout]` and `[effects]` overrides. Changing a notification setting
could therefore detach the desktop from its selected DSS profile.

Scalar writes now own only their selected declarations:

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

## Validation checkpoint — 4 November 2026

Host checks cover sparse updates, explicit assignments equal to defaults,
partial imports, coalescing, source comments/order, unrelated concurrent edits,
metadata restrictions and rejected updates. The actual packaged GUI-smoke
configuration retains identical scalar values except the selected display size.
The shared SDK/compositor token test confirms that an unrelated save retains a
DSS title height/text color while an explicit TOML default overrides it.

The native theme fixture runs terminal setup's setter and partial import, then
checks actual stored source after normal, no-op, rejected and compatibility
saves. No inherited palette/title/shadow fields are added. Its existing
three captures still verify exact SDK client, panel and cursor pixels, including
invalid-theme rollback. CLI setup also runs its full existing menu/import/error
contract; GUI smoke checks the real resolution save and client recycling.

Initial GUI runs missed the recycle marker because boot-scene `run.actions`
were incorrectly capped by the 15-second default command budget, despite the
marker contract requesting a 120-second wait. The runner now uses each step's
timeout and the overall run deadline for boot actions; command actions keep
their command budget. A simulated 20-second late boot marker reproduces the
old failure. Four host regressions cover success, per-step/overall expiration
and preservation of command deadlines. No marker or model assertion was relaxed,
and no production compositor lifecycle logic was changed for this correction.

A second deadline issue affected the config-gated resize proof: its intended
four-second grace for clients allowed to keep their size was measured as 240
rendered frames. A 509 ms loaded frame stretched that to roughly two minutes.
The proof now uses the existing monotonic FrameDeadline helper, with a fresh
deadline after session re-entry. Host samples at 16/500/2000 ms frame intervals
all reach the same four-second deadline. Maximize/restore calls, actual resize
markers and client-recycle assertions are retained; this is self-test timing,
not a rendering speedup.

M7.9 remains open for remaining paint/layout literals, controlled performance
acceptance and shell extraction evaluation. This increment adds no input/render
throughput claim.
