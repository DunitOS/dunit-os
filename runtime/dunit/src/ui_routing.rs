//! One capture/bubble pass for each native packet, before all SDK defaults.
use super::*;
use alloc::string::String;
impl<C: Component> ComponentWindow<C>
where
    C::Message: Clone,
{
    pub(super) fn dispatch_event(
        &mut self,
        event: Option<WindowEvent>,
    ) -> NativeResult<Option<WindowEvent>> {
        self.wake_scrollbars(&event)?;
        let Some(WindowEvent::Input(input)) = &event else {
            return self.dispatch_event_default(event);
        };
        if input.kind == IN_KEY || input.kind == IN_SCROLL {
            self.text_clicks.cancel();
        }
        let pointer = match input.kind {
            IN_DOWN => Some(PointerEvent::Down),
            IN_UP => Some(PointerEvent::Up),
            IN_MOVE => Some(PointerEvent::Move),
            IN_LEAVE => Some(PointerEvent::Leave),
            _ => None,
        };
        let (target, kind) = if let Some(pointer) = pointer {
            let capture = if pointer == PointerEvent::Down {
                None
            } else {
                self.renderer
                    .scroll_capture_key()
                    .or_else(|| self.application.pointer_capture_key())
            };
            let target = self
                .application
                .popup_state()
                .map(|p| p.owner.as_str())
                .or(capture)
                .or_else(|| self.renderer.hit_key(input.x as f32, input.y as f32))
                .map(String::from);
            (
                target,
                UiEventKind::Pointer {
                    event: pointer,
                    position: (pointer != PointerEvent::Leave)
                        .then_some((input.x as f32, input.y as f32)),
                    button: input.value as i32,
                    modifiers: 0,
                },
            )
        } else if input.kind == IN_KEY {
            (
                self.application
                    .popup_state()
                    .map(|p| p.owner.as_str())
                    .or_else(|| self.application.focused_key())
                    .map(String::from),
                UiEventKind::NativeKey {
                    scancode: input.x,
                    character: input.value,
                    modifiers: input.y as u8,
                },
            )
        } else if input.kind == IN_SCROLL {
            (
                self.pointer_position
                    .and_then(|(x, y)| self.renderer.hit_key(x as f32, y as f32))
                    .or_else(|| self.application.focused_key())
                    .map(String::from),
                UiEventKind::Wheel {
                    delta: input.x,
                    horizontal: false,
                    position: self.pointer_position.map(|(x, y)| (x as f32, y as f32)),
                },
            )
        } else {
            return self.dispatch_event_default(event);
        };
        let outcome = self
            .application
            .begin_event_dispatch(target.as_deref(), kind)
            .map_err(native)?;
        let result = self.dispatch_routed_default(event, outcome);
        // The helper owns every early return and ?, so errors cannot leave the
        // next hardware event (or a later host API call) with routing disabled.
        self.application.end_event_dispatch();
        result
    }
    fn dispatch_routed_default(
        &mut self,
        event: Option<WindowEvent>,
        outcome: RouteOutcome,
    ) -> NativeResult<Option<WindowEvent>> {
        let mut changed = outcome.changed;
        if let Some(WindowEvent::Input(input)) = &event {
            if matches!(input.kind, IN_DOWN | IN_UP | IN_MOVE | IN_LEAVE) {
                self.pointer_position = (input.kind != IN_LEAVE).then_some((input.x, input.y));
                if outcome.default_prevented && matches!(input.kind, IN_DOWN | IN_UP | IN_LEAVE) {
                    changed |= self.renderer.cancel_scroll_capture();
                    self.text_clicks.cancel();
                    self.selection_frame = None;
                }
            }
        }
        if changed {
            self.redraw()?;
        }
        if outcome.default_prevented {
            return Ok(event);
        }
        self.dispatch_event_default(event)
    }
}
