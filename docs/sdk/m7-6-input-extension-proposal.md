# M7.6 input lifecycle extension — proposal awaiting approval

The current INP1 channel delivers key presses but no releases or keyboard focus
changes. Existing widget implementations cannot honestly claim held-key pressed
states or close menus reliably on another window taking focus. Completing those
parts of M7.6 requires an explicit protocol decision under CLAUDE.md §1.

Proposed compatible extension:

- Add negotiated `FEATURE_INPUT_LIFECYCLE` (feature bit 1) to HELLO/WELCOME.
  Existing clients offering no bit keep their exact current events/bytes.
- Keep the 20-byte INP1 envelope and all current kind values. Add KEY_UP=7,
  FOCUS_IN=8, FOCUS_OUT=10 only for negotiated clients. Key-up carries scancode,
  modifiers and no cooked text. A focus-out cancels capture/pressed state,
  transient menus/tooltips and pending keyboard activation.
- For negotiated pointer events only, retain button in low 8 bits of value and
  put Shift/Ctrl/Alt/Super in bits 8..11. Central codec helpers decode the two
  fields; old clients continue receiving the old button value. Native SDK uses
  these modifiers for range selection and text selection.
- Preserve original kernel key-event ABI: make/break and modifiers are already
  available. No new syscall is proposed. Pointer modifier ordering must be
  tested against the kernel pointer queue; any necessary change to that queue's
  public contract will be described and approved before implementation.
- New native clients negotiate the feature; unsupported servers preserve the
  documented legacy behavior. Malformed kinds/modifiers fail codec validation.
- Verify both negotiated and legacy clients, focus changes between two clients,
  held Space/Enter/release/Escape, popup dismissal and pointer selection with
  real input through qemu_test. Existing GUI smoke remains a regression gate.

This proposal does not authorize a popup surface role, global grabs, clipboard,
IME wire messages or privilege changes. Those are separate contracts if needed.
Independent table/file-dialog/focus/layout work can proceed while this decision
is pending. No protocol implementation changes have been made by this proposal.
