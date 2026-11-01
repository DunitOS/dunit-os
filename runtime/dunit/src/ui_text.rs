//! Text pointer gestures share pure selection with host callers. The native
//! adapter supplies time; no input-wire extension or busy timer is involved.
use super::*;
impl<C: Component> ComponentWindow<C>
where
    C::Message: Clone,
{
    pub(super) fn text_pointer(
        &mut self,
        pointer: PointerEvent,
        target: Option<&str>,
        position: Option<(&str, usize)>,
        x: f32,
        y: f32,
    ) -> NativeResult<bool> {
        let config = position
            .and_then(|(key, _)| self.renderer.input_click_config(key))
            .unwrap_or_default();
        let time_ms = self
            .motion_clock
            .elapsed()?
            .as_millis()
            .min(u64::MAX as u128) as u64;
        self.application
            .pointer_text_at(
                pointer,
                target,
                position,
                &mut self.text_clicks,
                TextClickSample {
                    time_ms,
                    x,
                    y,
                    config,
                },
            )
            .map_err(native)
    }
}
