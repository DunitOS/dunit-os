# Contributing to Dunit OS

Thanks for taking an interest in Dunit. Start with the
[roadmap](../DUNIT_OS_TECHNICAL_ROADMAP.md) to see what works today and what is
planned. For a larger change, open an issue first so we can agree on the scope.

## Working on the code

Keep kernel mechanisms separate from desktop policy. Window management, themes,
panels and application behavior belong in userspace. Preserve the native ABI
and GUI protocol; discuss changes to these contracts before implementing them.

Use `tools/qemu_test.py` to build, boot and test the system. Test the behavior
you changed, check the serial log, and include the command and result with your
contribution. A successful build alone does not show that a feature works.

For example, run the desktop smoke test from the repository root:

```bash
python3 tools/qemu_test.py --build --config limine_test_gui.conf \
  --markers-file tools/gui_smoke_markers.json \
  --ready-marker 'gui_server: desktop ready' --timeout 300 --linger 8
```

Keep changes focused. Avoid unrelated formatting, generated build files and
changes to submodule pointers unless they are part of the work. Describe what
changed, why, how you tested it, and any remaining limitations.

## Using coding agents

The agent instructions are kept here with contributor documentation:

- [AGENTS.md](contributors/AGENTS.md): entry point for agents.
- [CLAUDE.md](contributors/CLAUDE.md): architecture, testing, safety and Git rules.

These files apply to work across the repository. When using an agent, explicitly
ask it to read `.github/contributors/AGENTS.md` and the linked rules before
editing. Tools that only discover instructions at the repository root may not
load this directory automatically.

All paths in the rules refer to the repository root unless stated otherwise.
