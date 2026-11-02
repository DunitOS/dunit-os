//! Capture and bubble dispatch over stable view keys, before control defaults.
use crate::{Application, Component, Key, PointerEvent, Result, View};
use alloc::{string::String, vec::Vec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    Capture,
    Bubble,
}
#[derive(Debug, Clone, PartialEq)]
pub enum UiEventKind {
    Key(Key),
    /// The native keyboard packet, including keys with no cooked character.
    NativeKey {
        scancode: i32,
        character: u32,
        modifiers: u8,
    },
    NativeKeyUp {
        scancode: i32,
        modifiers: u8,
    },
    Pointer {
        event: PointerEvent,
        position: Option<(f32, f32)>,
        button: i32,
        modifiers: u8,
    },
    Wheel {
        delta: i32,
        horizontal: bool,
        position: Option<(f32, f32)>,
    },
}
impl UiEventKind {
    fn keyboard(&self) -> bool {
        matches!(
            self,
            Self::Key(_) | Self::NativeKey { .. } | Self::NativeKeyUp { .. }
        )
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct UiEvent {
    pub kind: UiEventKind,
    pub phase: EventPhase,
    pub target: String,
    pub current_target: String,
    pub default_prevented: bool,
}
/// Propagation and the control's default action are independent decisions.
#[derive(Debug, Clone, PartialEq)]
pub struct EventResponse<M> {
    pub message: Option<M>,
    pub stop_propagation: bool,
    pub prevent_default: bool,
}
impl<M> Default for EventResponse<M> {
    fn default() -> Self {
        Self {
            message: None,
            stop_propagation: false,
            prevent_default: false,
        }
    }
}
impl<M> EventResponse<M> {
    pub fn message(message: M) -> Self {
        Self {
            message: Some(message),
            ..Self::default()
        }
    }
    pub fn stop(mut self) -> Self {
        self.stop_propagation = true;
        self
    }
    pub fn prevent(mut self) -> Self {
        self.prevent_default = true;
        self
    }
    pub(crate) fn map<N>(self, mapper: &impl Fn(M) -> N) -> EventResponse<N> {
        EventResponse {
            message: self.message.map(mapper),
            stop_propagation: self.stop_propagation,
            prevent_default: self.prevent_default,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RouteOutcome {
    pub default_prevented: bool,
    pub changed: bool,
}
impl<M> View<M> {
    pub fn on_capture(mut self, handler: fn(UiEvent) -> EventResponse<M>) -> Self {
        self.capture = Some(crate::callback::Callback::new(handler));
        self
    }
    pub fn on_bubble(mut self, handler: fn(UiEvent) -> EventResponse<M>) -> Self {
        self.bubble = Some(crate::callback::Callback::new(handler));
        self
    }
    fn event_path(&self, target: &str, output: &mut Vec<String>) -> bool {
        output.push(self.key.clone());
        if self.key == target {
            return true;
        }
        for child in &self.children {
            if child.event_path(target, output) {
                return true;
            }
        }
        output.pop();
        false
    }
}
impl<C: Component> Application<C> {
    /// Dispatch handlers only. The caller performs a default action only when
    /// default_prevented is false. Normal key/pointer entry points do this.
    pub fn route_event(&mut self, target: Option<&str>, kind: UiEventKind) -> Result<RouteOutcome> {
        if self.event_dispatch_depth > 0 {
            return Ok(RouteOutcome::default());
        }
        let target: String = target
            .unwrap_or_else(|| self.active_dialog().unwrap_or(self.view.key()))
            .into();
        let mut path = Vec::new();
        if !self.view.event_path(&target, &mut path)
            || !self.in_scope(&target)
            || self
                .view
                .find(&target)
                .is_none_or(|v| v.interaction.disabled)
        {
            return Ok(RouteOutcome {
                default_prevented: true,
                changed: false,
            });
        }
        let modal = self.active_dialog().map(String::from);
        let focus = self.focused_key().map(String::from);
        let target_kind = self.view.find(&target).unwrap().kind.clone();
        let mut route = path
            .iter()
            .filter(|k| self.in_scope(k))
            .map(|k| (k.clone(), EventPhase::Capture))
            .collect::<Vec<_>>();
        route.extend(
            path.iter()
                .rev()
                .filter(|k| self.in_scope(k))
                .map(|k| (k.clone(), EventPhase::Bubble)),
        );
        let valid = |app: &Self| {
            let mut now = Vec::new();
            app.view.event_path(&target, &mut now)
                && now == path
                && app.active_dialog() == modal.as_deref()
                && app.in_scope(&target)
                && app
                    .view
                    .find(&target)
                    .is_some_and(|v| !v.interaction.disabled && v.kind == target_kind)
                && (!kind.keyboard() || app.focused_key() == focus.as_deref())
        };
        let mut outcome = RouteOutcome::default();
        for (current, phase) in route {
            if !valid(self) {
                outcome.default_prevented = true;
                break;
            }
            let handler = self
                .view
                .find(&current)
                .filter(|v| !v.interaction.disabled)
                .and_then(|v| match phase {
                    EventPhase::Capture => v.capture.clone(),
                    EventPhase::Bubble => v.bubble.clone(),
                });
            let Some(handler) = handler else {
                continue;
            };
            let response = handler.call(UiEvent {
                kind: kind.clone(),
                phase,
                target: target.clone(),
                current_target: current,
                default_prevented: outcome.default_prevented,
            });
            outcome.default_prevented |= response.prevent_default;
            if let Some(message) = response.message {
                self.update(message)?;
                outcome.changed = true;
            }
            // Always revalidate once more after the final/stopping handler.
            if !valid(self) {
                outcome.default_prevented = true;
                break;
            }
            if response.stop_propagation {
                break;
            }
        }
        Ok(outcome)
    }
    /// Stable owner of an active pointer gesture, including ordinary buttons.
    pub fn pointer_capture_key(&self) -> Option<&str> {
        let mut nodes = Vec::new();
        self.view.flatten(&mut nodes);
        nodes
            .into_iter()
            .find(|v| {
                (v.interaction.pressed && self.keyboard_activation_key() != Some(v.key.as_str()))
                    || v.input.as_ref().is_some_and(|i| i.dragging)
            })
            .map(|v| v.key.as_str())
    }
    /// End a cancelled gesture without committing a click or editing a model.
    pub fn cancel_pointer_interaction(&mut self) -> bool {
        let mut changed = false;
        self.view.visit_mut(&mut |v| {
            changed |= v.interaction.pressed;
            v.interaction.pressed = false;
            if let Some(i) = &mut v.input {
                changed |= i.dragging;
                i.dragging = false;
            }
            if let Some(s) = &mut v.slider {
                s.grab = 0.;
            }
            if let Some(s) = &mut v.split {
                s.grab = 0.;
            }
            if let Some(s) = &mut v.spinbox {
                changed |= s.pressed.take().is_some();
            }
        });
        changed |= self
            .overlays
            .open
            .as_ref()
            .is_some_and(|p| p.pressed.is_some());
        self.cancel_popup_press();
        changed
    }
    /// Native adapters bracket their complete event default dispatch so nested
    /// key/pointer helpers do not route the same hardware event twice.
    #[doc(hidden)]
    pub fn begin_event_dispatch(
        &mut self,
        target: Option<&str>,
        kind: UiEventKind,
    ) -> Result<RouteOutcome> {
        if matches!(
            kind,
            UiEventKind::Pointer {
                event: PointerEvent::Down,
                ..
            }
        ) {
            self.cancel_keyboard_activation();
        }
        let leaving = matches!(
            kind,
            UiEventKind::Pointer {
                event: PointerEvent::Leave,
                ..
            }
        );
        let cancel_pointer = matches!(
            kind,
            UiEventKind::Pointer {
                event: PointerEvent::Down | PointerEvent::Up | PointerEvent::Leave,
                ..
            }
        );
        let mut outcome = self.route_event(target, kind)?;
        if outcome.default_prevented && cancel_pointer {
            outcome.changed |= self.cancel_pointer_interaction();
            if leaving {
                self.view.visit_mut(&mut |v| {
                    outcome.changed |= v.interaction.hovered;
                    v.interaction.hovered = false;
                });
            }
        }
        self.event_dispatch_depth += 1;
        Ok(outcome)
    }
    #[doc(hidden)]
    pub fn end_event_dispatch(&mut self) {
        self.event_dispatch_depth = self.event_dispatch_depth.saturating_sub(1);
    }
    pub(crate) fn dispatch_input(
        &mut self,
        target: Option<&str>,
        kind: UiEventKind,
        default: impl FnOnce(&mut Self) -> Result<bool>,
    ) -> Result<bool> {
        let capture = if matches!(
            kind,
            UiEventKind::Pointer {
                event: PointerEvent::Move | PointerEvent::Up | PointerEvent::Leave,
                ..
            }
        ) {
            self.pointer_capture_key().map(String::from)
        } else {
            None
        };
        let outcome = self.begin_event_dispatch(capture.as_deref().or(target), kind)?;
        let result = if outcome.default_prevented {
            Ok(true)
        } else {
            default(self)
        };
        self.end_event_dispatch();
        result.map(|consumed| consumed || outcome.changed)
    }
}
