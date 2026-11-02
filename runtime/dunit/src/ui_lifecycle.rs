//! Negotiated make/break and keyboard-focus lifecycle in the native adapter.
use super::*;
impl<C: Component> ComponentWindow<C> {
    pub fn input_lifecycle(&self) -> bool {
        self.window.input_lifecycle()
    }
    pub(super) fn focus_lifecycle(&mut self, focused: bool) -> NativeResult<()> {
        let changed = self.application.window_focus(focused);
        if !focused {
            self.renderer.cancel_scroll_capture();
            self.text_clicks.cancel();
            self.selection_frame = None;
            self.pointer_position = None;
            self.caret_frame = None;
        } else {
            self.caret_frame = Some(crate::time::Instant::now()?);
        }
        if changed {
            self.redraw()?;
        }
        Ok(())
    }
}
impl<C: Component> ComponentWindow<C>
where
    C::Message: Clone,
{
    pub(super) fn key_lifecycle(&mut self, input: &crate::window::Input) -> NativeResult<bool> {
        if input.kind == IN_KEY_UP {
            let consumed = self
                .application
                .key_release(input.x, input.y as u8)
                .map_err(native)?;
            if consumed {
                self.redraw()?;
            }
            return Ok(true);
        }
        if input.kind == IN_KEY && self.window.input_lifecycle() {
            if let Some(key) = Key::from_native(input.x, input.value, input.y as u8) {
                // Escape also falls through to dialog/menu defaults. Publish
                // cancellation even when those defaults have nothing to handle.
                if key.character == '\u{1b}' && self.application.cancel_keyboard_activation() {
                    self.redraw()?;
                }
                if self.application.key_press(input.x, key).map_err(native)? {
                    self.redraw()?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
