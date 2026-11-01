# Focus ordering and mnemonics

All enabled focusable controls participate in stable document order by default.
`.tab_index(n)` orders positive values first, then zero-valued controls; ties retain
document order. Negative values skip Tab, while allowing pointer/programmatic
focus. Disabled controls, inactive tab pages and controls outside the active modal
scope are excluded. Radio groups and toolbars keep their roving single Tab stop.
Shift+Tab reverses the same order with wrap.

`.mnemonic('f')` binds Alt+F to focus a control and invoke its action if present.
Inputs receive focus without inserting text. Disabled controls are skipped.
Duplicate mnemonics within a focus scope are validation errors; a dialog can reuse
a background mnemonic because its scope is exclusive. Popup menus retain their
existing mnemonic behavior. Focus/disabled appearance and layout changes resolve
through DSS state selectors (`:focus`, `:disabled`).

Committing a focused spinbox may rebuild the entire view or open a modal. Pointer
and mnemonic dispatch recheck scope and availability after that commit, preventing
an action on the background with the same event. Host tests in `focus_order.rs`
cover ordering, reverse traversal, negative indices, disabled/field mnemonics,
modal restoration and that reentrancy case.
