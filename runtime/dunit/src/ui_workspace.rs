use super::*;
impl<C: Component> ComponentWindow<C>
where
    C::Message: Clone,
{
    pub fn active_dialog(&self) -> Option<&str> {
        self.application.active_dialog()
    }
    pub fn workspace_command(&mut self, command: WorkspaceCommand) -> NativeResult<bool> {
        let consumed = self
            .application
            .workspace_command(command)
            .map_err(native)?;
        if consumed {
            self.redraw()?;
        }
        Ok(consumed)
    }
    pub(super) fn workspace_event(&mut self, event: &Option<WindowEvent>) -> NativeResult<bool> {
        if let Some(WindowEvent::Input(input)) = event {
            if input.kind == IN_KEY {
                if input.y == 0 && input.value == 0 {
                    let command = match input.x {
                        0x4b => Some(SliderCommand::Decrease),
                        0x4d => Some(SliderCommand::Increase),
                        0x48 => Some(SliderCommand::Decrease),
                        0x50 => Some(SliderCommand::Increase),
                        0x47 => Some(SliderCommand::Home),
                        0x4f => Some(SliderCommand::End),
                        0x49 => Some(SliderCommand::PageUp),
                        0x51 => Some(SliderCommand::PageDown),
                        _ => None,
                    };
                    if let Some(command) = command {
                        if self.application.splitter_command(command).map_err(native)? {
                            self.redraw()?;
                            return Ok(true);
                        }
                    }
                    let tab = match input.x {
                        0x47 => Some(WorkspaceCommand::First),
                        0x4f => Some(WorkspaceCommand::Last),
                        _ => None,
                    };
                    if let Some(tab) = tab {
                        if self.workspace_command(tab)? {
                            return Ok(true);
                        }
                    }
                }
            }
            if matches!(input.kind, IN_MOVE | IN_DOWN | IN_UP | IN_LEAVE)
                && (!matches!(input.kind, IN_DOWN | IN_UP) || input.value == 0)
            {
                let event = match input.kind {
                    IN_DOWN => PointerEvent::Down,
                    IN_UP => PointerEvent::Up,
                    IN_LEAVE => PointerEvent::Leave,
                    _ => PointerEvent::Move,
                };
                let target = self
                    .renderer
                    .hit_key(input.x as f32, input.y as f32)
                    .map(alloc::string::String::from);
                let capture = if event == PointerEvent::Down {
                    target.clone()
                } else {
                    self.application
                        .dragging_splitter_key()
                        .map(alloc::string::String::from)
                        .or_else(|| target.clone())
                };
                let geometry = capture.as_deref().and_then(|key| {
                    self.renderer
                        .splitter_geometry(key)
                        .map(|g| (key, g.coordinate(input.x as f32, input.y as f32), g))
                });
                if event == PointerEvent::Down && geometry.is_some() {
                    self.renderer.cancel_scroll_capture();
                }
                if self
                    .application
                    .splitter_pointer(event, target.as_deref(), geometry)
                    .map_err(native)?
                {
                    self.redraw()?;
                    return Ok(true);
                }
                if self.application.blocked_pointer(target.as_deref()) {
                    self.redraw()?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
