# SDK event routing

`View::on_capture` and `View::on_bubble` register typed handlers returning
`EventResponse<Message>`. Each `UiEvent` carries the original stable `target`,
`current_target`, phase, payload and accumulated `default_prevented` flag.

The route runs from root to target in capture, then target to root in bubble.
Both phases precede the widget default action. `Response::message(message)`
updates the component immediately. `.stop()` stops later handlers but permits
the default; `.prevent()` suppresses the default without stopping propagation.
They may be combined. `Response::default()` observes without changing the model.
Message composition maps the entire response, preserving both flags.

After every handler, routing checks that the target still exists, has the same
kind and ancestor path, remains enabled and within the same modal scope. A
keyboard event additionally requires unchanged focus. If any condition fails,
the stale route and default are cancelled. A modal dialog limits handlers to its
own subtree; background ancestors do not receive dialog events.

`Application::key`, `pointer`, `pointer_at`, `scrollbar_pointer` and
`pointer_text_at` route their input before performing defaults. Captured pointer
Move/Up/Leave routes to the gesture owner, while activation still requires a
release over the original control. A prevented Down/Up/Leave cancels transient
press/drag state without emitting a click or modifying text selection.

`ComponentWindow` routes each native packet once before all SDK defaults,
including raw arrow/function keys, collection navigation, text editing,
scrollbars, wheel, menus, sliders and splitters. Nested application helpers are
suppressed only for that dispatch; errors restore routing for the next event.
Native packets use `UiEventKind::NativeKey`; direct cooked key calls use `Key`.
Pointer coordinates are available for native events and `pointer_text_at`;
plain host `pointer` calls supply no coordinates. The current gui-v1 channel
has no pointer modifiers or key-up delivery: routing does not synthesize them.

Programmatic commands such as `edit`, `table_command` or `update` do not invent
hardware events. Timers and view reconciliation are also separate operations.
For a custom adapter, `route_event` performs handlers only and returns whether
the adapter must suppress its default. Native bracket helpers are adapter
plumbing, not application commands.
