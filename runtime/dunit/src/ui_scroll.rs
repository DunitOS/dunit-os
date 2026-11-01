//! One-shot scrollbar idle deadlines share the native wait set.
use super::*;
impl<C: Component> ComponentWindow<C> {
    /// Last painted idle visibility; geometry remains available when hidden.
    pub fn scrollbars_visible(&self, key: &str) -> Option<bool> {
        self.renderer.scrollbars_visible(key)
    }
    fn scrollbar_now(&self) -> NativeResult<u64> {
        u64::try_from(self.motion_clock.elapsed()?.as_millis()).map_err(|_| Error(libdunit::EINVAL))
    }
    pub(super) fn scrollbar_wait(&self) -> NativeResult<Option<Duration>> {
        let Some(deadline) = self.renderer.scroll_hide_deadline_ms() else {
            return Ok(None);
        };
        Ok(Some(Duration::from_millis(
            deadline.saturating_sub(self.scrollbar_now()?),
        )))
    }
    pub(super) fn tick_scrollbars(&mut self) -> NativeResult<()> {
        let now = self.scrollbar_now()?;
        if self
            .renderer
            .scroll_hide_deadline_ms()
            .is_some_and(|deadline| now >= deadline)
        {
            self.redraw()?;
        }
        Ok(())
    }
    pub(super) fn wake_scrollbars(&mut self, event: &Option<WindowEvent>) -> NativeResult<()> {
        let Some(WindowEvent::Input(input)) = event else {
            return Ok(());
        };
        let target = match input.kind {
            IN_MOVE | IN_DOWN | IN_UP => self.renderer.hit_key(input.x as f32, input.y as f32),
            IN_SCROLL => self
                .pointer_position
                .and_then(|(x, y)| self.renderer.hit_key(x as f32, y as f32))
                .or_else(|| self.application.focused_key()),
            IN_KEY => self.application.focused_key(),
            _ => None,
        }
        .map(alloc::string::String::from);
        let now = self.scrollbar_now()?;
        if self.renderer.scroll_activity_at(target.as_deref(), now) {
            self.redraw()?;
        }
        Ok(())
    }
}
