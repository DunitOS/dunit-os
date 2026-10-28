//! Native window adapter for the pure retained component runtime.
use crate::{
    time::Duration,
    window::{DamageRect, Window, WindowEvent, IN_DOWN, IN_KEY, IN_LEAVE, IN_MOVE, IN_UP},
    Error, Result as NativeResult,
};
pub use dunit_component::*;

/// Load the selected SDK profile and configured font for a custom renderer.
pub fn load_theme(overlay: &str) -> NativeResult<(Theme, Font)> {
    let (_, theme, font) = crate::ui_theme::Watcher::open(overlay)?;
    Ok((theme, font))
}

fn native(error: UiError) -> Error {
    Error(match error {
        UiError::Allocation => libdunit::ENOMEM,
        UiError::InvalidClass
        | UiError::DuplicateKey
        | UiError::EmptyKey
        | UiError::DuplicateBinding
        | UiError::ChildBinding
        | UiError::TreeLimit
        | UiError::InvalidLayout
        | UiError::InvalidSize => libdunit::EINVAL,
    })
}

// Target interval for automatic sampling, not a proven 16 ms rendering budget.
const MOTION_FRAME: Duration = Duration::from_millis(16);

/// One root component in the existing single-window IPC session. Button input
/// and shortcuts dispatch through `update`; resize uses compositor geometry.
/// CPU invalidation drives buffer repair and existing gui-v1 damage metadata.
/// Unchanged views skip publication; the compositor retains unchanged pixels.
pub struct ComponentWindow<C: Component> {
    application: Application<C>,
    renderer: Renderer,
    window: Window,
    last_render: RenderStats,
    uploaded_pixels: u64,
    failed: bool,
    theme: Option<crate::ui_theme::Watcher>,
    theme_error: Option<Error>,
    theme_error_detail: Option<alloc::string::String>,
    theme_checked: Option<crate::time::Instant>,
    motion_clock: crate::time::Instant,
    motion_frame: Option<crate::time::Instant>,
    motion_requested: bool,
}
impl<C: Component> ComponentWindow<C> {
    pub fn open(width: u32, height: u32, component: C, renderer: Renderer) -> NativeResult<Self> {
        let application = Application::new(component).map_err(native)?;
        let mut ui = Self {
            application,
            renderer,
            window: Window::open(width, height)?,
            last_render: RenderStats::default(),
            uploaded_pixels: 0,
            failed: false,
            theme: None,
            theme_error: None,
            theme_error_detail: None,
            theme_checked: None,
            motion_clock: crate::time::Instant::now()?,
            motion_frame: None,
            motion_requested: true,
        };
        ui.redraw()?;
        Ok(ui)
    }
    /// Open with the configured font and SDK profile, without application DSS.
    pub fn open_themed(width: u32, height: u32, component: C) -> NativeResult<Self> {
        Self::open_themed_with_style(width, height, component, "")
    }
    /// An optional application layer can reference the selected profile tokens.
    pub fn open_themed_with_style(
        width: u32,
        height: u32,
        component: C,
        overlay: &str,
    ) -> NativeResult<Self> {
        let (watcher, theme, font) = crate::ui_theme::Watcher::open(overlay)?;
        let mut renderer = Renderer::new(font, theme.cascade, theme.background);
        renderer.set_motion_enabled(watcher.animations_enabled());
        let mut ui = Self::open(width, height, component, renderer)?;
        ui.theme = Some(watcher);
        ui.theme_checked = Some(crate::time::Instant::now()?);
        Ok(ui)
    }
    /// Last rejected theme candidate. Automatic reload keeps the working theme;
    /// a later valid candidate clears this error. Drawing errors still poison.
    pub fn theme_error(&self) -> Option<Error> {
        self.theme_error
    }
    pub fn theme_error_detail(&self) -> Option<&str> {
        self.theme_error_detail.as_deref()
    }
    pub fn reload_theme(&mut self) -> NativeResult<bool> {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        let candidate = match self
            .theme
            .as_ref()
            .map(|theme| theme.candidate())
            .transpose()
        {
            Ok(candidate) => candidate,
            Err(failure) => {
                self.theme_error = Some(failure.error);
                self.theme_error_detail = Some(failure.detail);
                return Err(failure.error);
            }
        };
        let Some(Some((watcher, theme, font))) = candidate else {
            self.theme_error = None;
            self.theme_error_detail = None;
            return Ok(false);
        };
        self.renderer.set_theme(theme, font);
        self.renderer
            .set_motion_enabled(self.motion_requested && watcher.animations_enabled());
        self.redraw()?;
        self.theme = Some(watcher);
        self.theme_error = None;
        self.theme_error_detail = None;
        Ok(true)
    }
    pub fn is_animating(&self) -> bool {
        self.renderer.is_animating()
    }
    /// Applications may reduce motion. Themed windows also honor the desktop's
    /// [effects] anim setting; changing the mode settles the next frame.
    pub fn set_motion_enabled(&mut self, enabled: bool) -> NativeResult<()> {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        self.motion_requested = enabled;
        let global = self
            .theme
            .as_ref()
            .map_or(true, |theme| theme.animations_enabled());
        self.renderer.set_motion_enabled(enabled && global);
        self.redraw()
    }
    pub fn component(&self) -> &C {
        self.application.component()
    }
    pub fn view(&self) -> &View<C::Message> {
        self.application.view()
    }
    pub fn size(&self) -> (u32, u32) {
        self.window.size()
    }
    /// Read-only CPU canvas of the most recent render, useful for snapshots.
    pub fn pixels(&self) -> &[u32] {
        self.renderer.pixels()
    }
    /// Pixels copied into shared storage by the last painting attempt, including
    /// repair of a retired buffer's previous contents. Zero for unchanged views.
    pub fn last_uploaded_pixels(&self) -> u64 {
        self.uploaded_pixels
    }
    pub fn last_render(&self) -> RenderStats {
        self.last_render
    }
    /// A model/view validation or draw failure ends this session; propagate it.
    pub fn update(&mut self, message: C::Message) -> NativeResult<()> {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        if let Err(error) = self.application.update(message) {
            self.failed = true;
            return Err(native(error));
        }
        self.redraw()
    }
    fn redraw(&mut self) -> NativeResult<()> {
        let clock = self.motion_clock;
        let mut sampled = None;
        let renderer = &mut self.renderer;
        let view = self.application.view();
        let last = &mut self.last_render;
        let uploaded = &mut self.uploaded_pixels;
        let result = self.window.try_draw_damage(|frame| {
            let (w, h) = frame.size();
            sampled = Some(crate::time::Instant::now()?);
            let now =
                u64::try_from(clock.elapsed()?.as_millis()).map_err(|_| Error(libdunit::EINVAL))?;
            *last = renderer.render_at(view, w, h, now).map_err(native)?;
            if last.damage.is_none() && !frame.requires_full_frame() {
                *uploaded = 0;
                return Ok(None);
            }
            let damage = if frame.requires_full_frame() {
                DamageRect { x: 0, y: 0, w, h }
            } else {
                let d = last.damage.unwrap();
                DamageRect {
                    x: d.x as i32,
                    y: d.y as i32,
                    w: d.width,
                    h: d.height,
                }
            };
            *uploaded = frame.copy_pixels(renderer.pixels(), Some(damage))?;
            Ok(Some(damage))
        });
        self.failed |= result.is_err();
        if result.is_ok() {
            self.motion_frame = sampled;
        }
        result
    }
    pub fn next_event(&mut self, timeout: Duration) -> NativeResult<Option<WindowEvent>>
    where
        C::Message: Clone,
    {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        let result = self.next_inner(timeout);
        self.failed |= result.is_err();
        result
    }
    fn next_inner(&mut self, timeout: Duration) -> NativeResult<Option<WindowEvent>>
    where
        C::Message: Clone,
    {
        let start = crate::time::Instant::now()?;
        let event = loop {
            let remaining = timeout.saturating_sub(start.elapsed()?);
            let since_check = self
                .theme_checked
                .map(|t| t.elapsed())
                .transpose()?
                .unwrap_or_default();
            let slice = if self.theme.is_some() {
                remaining.min(crate::ui_theme::POLL.saturating_sub(since_check))
            } else {
                remaining
            };
            let slice = if self.renderer.is_animating() {
                let elapsed = self
                    .motion_frame
                    .map(|t| t.elapsed())
                    .transpose()?
                    .unwrap_or_default();
                slice.min(MOTION_FRAME.saturating_sub(elapsed))
            } else {
                slice
            };
            let event = self.window.next_event(slice)?;
            if matches!(event, Some(WindowEvent::CloseRequested)) {
                self.renderer.cancel_motion();
                break event;
            }
            if self.theme.is_some()
                && self
                    .theme_checked
                    .map(|t| t.elapsed())
                    .transpose()?
                    .map_or(true, |elapsed| elapsed >= crate::ui_theme::POLL)
            {
                let previous_error = self.theme_error_detail.clone();
                if let Err(error) = self.reload_theme() {
                    if self.failed {
                        return Err(error);
                    }
                    if previous_error != self.theme_error_detail {
                        crate::println!(
                            "{}",
                            alloc::format!(
                                "[SDKTHEME] reload rejected: {}",
                                self.theme_error_detail.as_deref().unwrap_or(error.name())
                            )
                        )?;
                    }
                    self.theme_error = Some(error);
                }
                self.theme_checked = Some(crate::time::Instant::now()?);
            }
            if self.renderer.is_animating()
                && self
                    .motion_frame
                    .map(|t| t.elapsed())
                    .transpose()?
                    .map_or(true, |elapsed| elapsed >= MOTION_FRAME)
            {
                self.redraw()?;
            }
            if event.is_some() || start.elapsed()? >= timeout {
                break event;
            }
        };
        match &event {
            Some(WindowEvent::Resized { .. }) => self.redraw()?,
            Some(WindowEvent::Input(input))
                if matches!(input.kind, IN_MOVE | IN_DOWN | IN_UP | IN_LEAVE) =>
            {
                if matches!(input.kind, IN_DOWN | IN_UP) && input.value != 0 {
                    return Ok(event);
                }
                let pointer = match input.kind {
                    IN_DOWN => PointerEvent::Down,
                    IN_UP => PointerEvent::Up,
                    IN_LEAVE => PointerEvent::Leave,
                    _ => PointerEvent::Move,
                };
                let target = self.renderer.hit_key(input.x as f32, input.y as f32);
                if self.application.pointer(pointer, target).map_err(native)? {
                    self.redraw()?;
                }
            }
            Some(WindowEvent::Input(input)) if input.kind == IN_KEY => {
                if let (Some(character), Ok(modifiers)) =
                    (char::from_u32(input.value), u8::try_from(input.y))
                {
                    if self
                        .application
                        .key(Key {
                            character,
                            modifiers,
                        })
                        .map_err(native)?
                    {
                        self.redraw()?;
                    }
                }
            }
            _ => {}
        }
        Ok(event)
    }
    pub fn close(self) -> NativeResult<()> {
        self.window.close()
    }
}
