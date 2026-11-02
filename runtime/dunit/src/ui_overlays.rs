//! Native seat routing and tooltip deadline integration for SDK overlays.
use super::*;
use crate::window::Input;
impl<C: Component> ComponentWindow<C> {
    pub fn popup_state(&self) -> Option<&OpenPopup> {
        self.application.popup_state()
    }
    pub fn tooltip_candidate(&self) -> Option<(&str, u64)> {
        self.application.tooltip_candidate()
    }
    pub fn tooltip_delay_ms(&self, key: &str) -> Option<u64> {
        self.renderer.tooltip_delay_ms(key)
    }
    pub fn visible_tooltip(&self) -> Option<&str> {
        self.application.visible_tooltip()
    }
    fn tooltip_now(&self) -> NativeResult<u64> {
        u64::try_from(self.motion_clock.elapsed()?.as_millis()).map_err(|_| Error(libdunit::EINVAL))
    }
    pub(super) fn tooltip_wait(&self) -> NativeResult<Option<Duration>> {
        let Some((key, since)) = self.application.tooltip_candidate() else {
            return Ok(None);
        };
        let Some(delay) = self.renderer.tooltip_delay_ms(key) else {
            return Ok(None);
        };
        let now = self.tooltip_now()?;
        Ok(Some(Duration::from_millis(
            since.saturating_add(delay).saturating_sub(now),
        )))
    }
    pub(super) fn tick_tooltip(&mut self) -> NativeResult<()> {
        let delay = self
            .application
            .tooltip_candidate()
            .and_then(|(key, _)| self.renderer.tooltip_delay_ms(key));
        if let Some(delay) = delay {
            let now = self.tooltip_now()?;
            if self.application.sample_tooltip(now, delay) {
                self.redraw()?;
            }
        }
        Ok(())
    }
}
impl<C: Component> ComponentWindow<C>
where
    C::Message: Clone,
{
    pub(super) fn overlay_event(&mut self, event: &Option<WindowEvent>) -> NativeResult<bool> {
        match event {
            Some(WindowEvent::Resized { .. }) => {
                self.application.dismiss_tooltip();
                self.application.cancel_popup_press();
                Ok(false)
            }
            Some(WindowEvent::CloseRequested) => {
                self.application.close_popup();
                self.application.dismiss_tooltip();
                Ok(false)
            }
            Some(WindowEvent::Input(input)) => self.overlay_input(input),
            _ => Ok(false),
        }
    }
    fn overlay_input(&mut self, input: &Input) -> NativeResult<bool> {
        if input.kind == IN_KEY {
            let changed = self.application.dismiss_tooltip();
            if (input.x == 0x5d && input.y == 0) || (input.x == 0x44 && input.y == 1) {
                let target = self
                    .application
                    .focused_key()
                    .map(alloc::string::String::from);
                if let Some(target) = target {
                    let point = self.renderer.widget_rect(&target).map(|r| (r.x, r.y + r.h));
                    if self.application.open_context_menu(&target, point) {
                        self.redraw()?;
                        return Ok(true);
                    }
                }
            }
            let command = if input.y == 0 {
                match input.x {
                    0x48 => Some(PopupCommand::Up),
                    0x50 => Some(PopupCommand::Down),
                    0x4b => Some(PopupCommand::Left),
                    0x4d => Some(PopupCommand::Right),
                    0x47 => Some(PopupCommand::Home),
                    0x4f => Some(PopupCommand::End),
                    0x44 => Some(PopupCommand::MenuBar),
                    0x3e => Some(PopupCommand::Activate),
                    _ => None,
                }
            } else if input.y == 4 && input.x == 0x50 {
                Some(PopupCommand::Down)
            } else {
                None
            };
            if let Some(command) = command {
                if self.application.popup_command(command).map_err(native)? {
                    self.redraw()?;
                    return Ok(true);
                }
            }
            if self.application.popup_state().is_some() || input.y == 4 {
                if let Some(key) = u8::try_from(input.y)
                    .ok()
                    .and_then(|m| Key::from_native(input.x, input.value, m))
                {
                    if self.application.key(key).map_err(native)? {
                        self.redraw()?;
                        return Ok(true);
                    }
                }
            }
            if changed {
                self.redraw()?;
            }
            return Ok(false);
        }
        if input.kind == IN_SCROLL {
            let changed = self.application.dismiss_tooltip();
            if self.application.popup_state().is_some() {
                for _ in 0..input.x.unsigned_abs().min(32) {
                    self.application
                        .popup_command(if input.x > 0 {
                            PopupCommand::Down
                        } else {
                            PopupCommand::Up
                        })
                        .map_err(native)?;
                }
                self.redraw()?;
                return Ok(true);
            }
            if changed {
                self.redraw()?;
            }
            return Ok(false);
        }
        if !matches!(input.kind, IN_DOWN | IN_UP | IN_MOVE | IN_LEAVE) {
            return Ok(false);
        }
        self.pointer_position = if input.kind == IN_LEAVE {
            None
        } else {
            Some((input.x, input.y))
        };
        let target = self
            .renderer
            .hit_key(input.x as f32, input.y as f32)
            .map(alloc::string::String::from);
        let mut changed = false;
        if input.kind == IN_MOVE {
            if self.application.dragging_input_key().is_some()
                || self.application.dragging_slider_key().is_some()
            {
                changed |= self.application.dismiss_tooltip();
            } else {
                let now = self.tooltip_now()?;
                changed |= self.application.tooltip_pointer(target.as_deref(), now);
            }
        } else {
            changed |= self.application.dismiss_tooltip();
        }
        if input.kind == IN_DOWN && input.pointer_button() == 1 {
            let opened = target.as_deref().is_some_and(|key| {
                self.application
                    .open_context_menu(key, Some((input.x as f32, input.y as f32)))
            });
            changed |= opened;
            if !opened {
                changed |= self.application.close_popup();
            }
            if changed {
                self.redraw()?;
            }
            return Ok(true);
        }
        if matches!(input.kind, IN_DOWN | IN_UP) && input.pointer_button() != 0 {
            if changed {
                self.redraw()?;
            }
            return Ok(true);
        }
        let pointer = match input.kind {
            IN_DOWN => PointerEvent::Down,
            IN_UP => PointerEvent::Up,
            IN_LEAVE => PointerEvent::Leave,
            _ => PointerEvent::Move,
        };
        let hit = self.renderer.popup_hit(input.x as f32, input.y as f32);
        if self
            .application
            .popup_pointer(pointer, hit, target.as_deref())
            .map_err(native)?
        {
            self.redraw()?;
            return Ok(true);
        }
        if changed {
            self.redraw()?;
        }
        Ok(false)
    }
}
