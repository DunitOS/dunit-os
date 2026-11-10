//! Native window adapter for the pure retained component runtime.
use crate::{
    time::Duration,
    window::{
        DamageRect, Window, WindowEvent, IN_DOWN, IN_FOCUS_IN, IN_FOCUS_OUT, IN_KEY, IN_KEY_UP,
        IN_LEAVE, IN_MOVE, IN_SCROLL, IN_UP,
    },
    Error, Result as NativeResult,
};
pub use dunit_component::*;
#[path = "ui_metrics.rs"]
pub(crate) mod metrics;
pub use metrics::{ComponentMetrics, MetricTime, WindowMetrics};
#[path = "ui_files.rs"]
mod files;
pub use files::{choose_file, NativeDirectorySource};
#[path = "ui_lifecycle.rs"]
mod lifecycle;
#[path = "ui_list.rs"]
mod list;
#[path = "ui_overlays.rs"]
mod overlays;
#[path = "ui_routing.rs"]
mod routing;
#[path = "ui_scroll.rs"]
mod scroll;
#[path = "ui_text.rs"]
mod text;
#[path = "ui_transfer.rs"]
mod transfer;
#[path = "ui_workspace.rs"]
mod workspace;

/// Load the selected SDK profile and configured font for a custom renderer.
pub fn load_theme(overlay: &str) -> NativeResult<(Theme, Font)> {
    let (_, theme, font) = crate::ui_theme::Watcher::open(overlay)?;
    Ok((theme, font))
}

fn native(error: UiError) -> Error {
    Error(match error {
        UiError::Allocation => libdunit::ENOMEM,
        UiError::InvalidInput
        | UiError::InvalidRename
        | UiError::InvalidTable
        | UiError::InvalidTree
        | UiError::InvalidList
        | UiError::InvalidWorkspace
        | UiError::InvalidPopup
        | UiError::InvalidProgress
        | UiError::InvalidSpinbox
        | UiError::InvalidSlider
        | UiError::InvalidRadioGroup
        | UiError::InvalidClass
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

struct CanvasLayer {
    pixels: alloc::vec::Vec<u32>,
    output: alloc::vec::Vec<u32>,
    size: (u32, u32),
    dirty: bool,
}
/// One root component in the existing single-window IPC session. Button input
/// and shortcuts dispatch through `update`; resize uses compositor geometry.
/// CPU invalidation drives buffer repair and existing gui-v1 damage metadata.
/// Unchanged views skip publication; the compositor retains unchanged pixels.
pub struct ComponentWindow<C: Component> {
    canvas: Option<CanvasLayer>,
    metrics_enabled: bool,
    metrics: ComponentMetrics,
    application: alloc::boxed::Box<Application<C>>,
    renderer: alloc::boxed::Box<Renderer>,
    window: Window,
    last_render: RenderStats,
    uploaded_pixels: u64,
    failed: bool,
    theme: Option<alloc::boxed::Box<crate::ui_theme::Watcher>>,
    theme_error: Option<Error>,
    theme_error_detail: Option<alloc::string::String>,
    theme_checked: Option<crate::time::Instant>,
    motion_clock: crate::time::Instant,
    motion_frame: Option<crate::time::Instant>,
    motion_requested: bool,
    caret_frame: Option<crate::time::Instant>,
    selection_frame: Option<crate::time::Instant>,
    pointer_position: Option<(i32, i32)>,
    text_clicks: TextClickState,
    transfers: alloc::boxed::Box<transfer::Transfers>,
}
impl<C: Component> ComponentWindow<C> {
    pub fn open(width: u32, height: u32, component: C, renderer: Renderer) -> NativeResult<Self> {
        Self::open_format(
            width,
            height,
            component,
            renderer,
            crate::window::PixelFormat::Xrgb8888,
        )
    }
    pub fn open_format(
        width: u32,
        height: u32,
        component: C,
        renderer: Renderer,
        format: crate::window::PixelFormat,
    ) -> NativeResult<Self> {
        // Retained trees/theme/transfer snapshots live beyond a frame. Keep
        // their ownership on the heap so construction does not multiply large
        // return slots on the native 64 KiB user stack.
        let application = alloc::boxed::Box::new(Application::new(component).map_err(native)?);
        let mut ui = Self {
            canvas: None,
            metrics_enabled: false,
            metrics: Default::default(),
            application,
            renderer: alloc::boxed::Box::new(renderer),
            window: Window::open_format(width, height, format)?,
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
            caret_frame: None,
            selection_frame: None,
            pointer_position: None,
            text_clicks: TextClickState::default(),
            transfers: alloc::boxed::Box::default(),
        };
        ui.redraw()?;
        Ok(ui)
    }
    /// Supply custom document pixels beneath retained SDK controls. The SDK
    /// owns a private snapshot; published buffers remain immutable. The caller
    /// repaints domain content on resize, while menus/input keep SDK routing.
    pub fn set_canvas(
        &mut self,
        pixels: alloc::vec::Vec<u32>,
        width: u32,
        height: u32,
    ) -> NativeResult<()> {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or(Error(libdunit::EINVAL))?;
        if width == 0
            || height == 0
            || count > 16 * 1024 * 1024
            || pixels.len() != count
            || self.size() != (width, height)
        {
            return Err(Error(libdunit::EINVAL));
        }
        self.canvas = Some(CanvasLayer {
            output: pixels.clone(),
            pixels,
            size: (width, height),
            dirty: true,
        });
        self.redraw()
    }
    /// Ask the compositor to atomically reload saved desktop/theme resources.
    pub fn reload_desktop(&mut self) -> NativeResult<()> {
        self.window.reload_desktop()
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
        ui.theme = Some(alloc::boxed::Box::new(watcher));
        ui.theme_checked = Some(crate::time::Instant::now()?);
        Ok(ui)
    }
    pub fn launch_id(&self) -> u32 {
        self.window.launch_id()
    }
    pub fn widget_rect(&self, key: &str) -> Option<Rect> {
        self.renderer.widget_rect(key)
    }
    pub fn notify(&mut self, text: &str) -> NativeResult<()> {
        self.window.notify(text)
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
        let (width, height) = self.window.size();
        if let Err(error) = self.renderer.validate_theme_application(
            &self.application,
            &theme,
            font.as_ref(),
            width,
            height,
        ) {
            let error = native(error);
            self.theme_error = Some(error);
            self.theme_error_detail =
                Some(alloc::format!("theme layout rejected: {}", error.name()));
            return Err(error);
        }
        self.renderer.set_theme(theme, font);
        self.renderer
            .set_motion_enabled(self.motion_requested && watcher.animations_enabled());
        self.redraw()?;
        self.theme = Some(alloc::boxed::Box::new(watcher));
        self.theme_error = None;
        self.theme_error_detail = None;
        Ok(true)
    }
    pub fn is_animating(&self) -> bool {
        self.animation_interval().is_some()
    }
    fn animation_interval(&self) -> Option<Duration> {
        let transition = self.renderer.is_animating().then_some(MOTION_FRAME);
        let busy = self.renderer.busy_interval_ms().map(Duration::from_millis);
        match (transition, busy) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }
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
    pub fn scroll_offset_x(&self, key: &str) -> Option<f32> {
        self.renderer.scroll_offset_x(key)
    }
    pub fn scroll_thumb_x(&self, key: &str) -> Option<Rect> {
        self.renderer.scroll_thumb_x(key)
    }
    pub fn scroll_offset(&self, key: &str) -> Option<f32> {
        self.renderer.scroll_offset(key)
    }
    pub fn scroll_thumb(&self, key: &str) -> Option<Rect> {
        self.renderer.scroll_thumb(key)
    }
    pub fn scroll_viewport(&self, key: &str) -> Option<Rect> {
        self.renderer.scroll_viewport(key)
    }
    pub fn input_scroll_y(&self, key: &str) -> Option<f32> {
        self.renderer.input_scroll_y(key)
    }
    pub fn input_scroll_x(&self, key: &str) -> Option<f32> {
        self.renderer.input_scroll_x(key)
    }
    /// Opt-in aggregate diagnostics. Enabling does not reset existing samples.
    pub fn enable_metrics(&mut self, enabled: bool) {
        self.metrics_enabled = enabled;
        self.window.enable_metrics(enabled);
    }
    /// Reset counters, not event queues or application state. Already queued
    /// input may therefore be delivered without a receive in the new interval.
    pub fn reset_metrics(&mut self) {
        self.metrics = Default::default();
        self.window.reset_metrics();
    }
    pub fn metrics(&self) -> ComponentMetrics {
        let mut metrics = self.metrics;
        metrics.window = self.window.metrics();
        metrics
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
        self.caret_frame = Some(crate::time::Instant::now()?);
        self.redraw()
    }
    fn redraw(&mut self) -> NativeResult<()> {
        if self
            .canvas
            .as_ref()
            .is_some_and(|c| c.size != self.window.size())
        {
            self.uploaded_pixels = 0;
            return Ok(());
        }
        self.sync_transfer_target()?;
        let metrics_enabled = self.metrics_enabled;
        let redraw_started = metrics::start(metrics_enabled);
        if metrics_enabled {
            self.metrics.redraw_calls = self.metrics.redraw_calls.saturating_add(1);
        }
        let clock = self.motion_clock;
        let mut sampled = None;
        let renderer = &mut self.renderer;
        let application = &self.application;
        let last = &mut self.last_render;
        let uploaded = &mut self.uploaded_pixels;
        let metrics = &mut self.metrics;
        let canvas = &mut self.canvas;
        let result = self.window.try_draw_damage(|frame| {
            let (w, h) = frame.size();
            if canvas.as_ref().is_some_and(|c| c.size != (w, h)) {
                frame.defer();
                *uploaded = 0;
                return Ok(None);
            }
            sampled = Some(crate::time::Instant::now()?);
            let now =
                u64::try_from(clock.elapsed()?.as_millis()).map_err(|_| Error(libdunit::EINVAL))?;
            let render_started = metrics::start(metrics_enabled);
            if metrics_enabled {
                metrics.render_calls = metrics.render_calls.saturating_add(1);
            }
            let rendered = renderer.render_application_at(application, w, h, now);
            metrics.render.finish(metrics_enabled, render_started);
            *last = rendered.map_err(native)?;
            let canvas_dirty = canvas.as_ref().is_some_and(|c| c.dirty);
            if last.damage.is_none() && !frame.requires_full_frame() && !canvas_dirty {
                *uploaded = 0;
                return Ok(None);
            }
            let damage = if frame.requires_full_frame() || canvas_dirty {
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
            let copy_started = metrics::start(metrics_enabled);
            let copied = if let Some(canvas) = canvas.as_mut().filter(|c| c.size == (w, h)) {
                let x = damage.x as usize;
                let y = damage.y as usize;
                for row in y..y + damage.h as usize {
                    let start = row * w as usize + x;
                    let end = start + damage.w as usize;
                    canvas.output[start..end].copy_from_slice(&canvas.pixels[start..end]);
                }
                {
                    let mut surface =
                        dunit_render::Surface::new(&mut canvas.output, w as usize, h as usize);
                    surface.with_clip(
                        Rect {
                            x: x as f32,
                            y: y as f32,
                            w: damage.w as f32,
                            h: damage.h as f32,
                        },
                        |surface| {
                            surface.blit_premultiplied(renderer.pixels(), w as usize, h as usize);
                        },
                    );
                }
                frame.copy_pixels(&canvas.output, Some(damage))
            } else {
                frame.copy_pixels(renderer.pixels(), Some(damage))
            };
            metrics.copy.finish(metrics_enabled, copy_started);
            *uploaded = copied?;
            Ok(Some(damage))
        });
        self.metrics
            .redraw_total
            .finish(metrics_enabled, redraw_started);
        self.failed |= result.is_err();
        if result.is_ok() {
            if let Some(canvas) = self.canvas.as_mut() {
                canvas.dirty = false;
            }
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
            let selection = self.selection_autoscroll();
            if selection.is_none() {
                self.selection_frame = None;
            } else if self.selection_frame.is_none() {
                self.selection_frame = Some(crate::time::Instant::now()?);
            }
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
            let slice = if let Some(interval) = self.animation_interval() {
                let elapsed = self
                    .motion_frame
                    .map(|t| t.elapsed())
                    .transpose()?
                    .unwrap_or_default();
                slice.min(interval.saturating_sub(elapsed))
            } else {
                slice
            };
            let slice = if let Some(ms) = self.renderer.caret_blink_ms() {
                let interval = Duration::from_millis(ms);
                let elapsed = self
                    .caret_frame
                    .map(|t| t.elapsed())
                    .transpose()?
                    .unwrap_or_default();
                slice.min(interval.saturating_sub(elapsed))
            } else {
                slice
            };
            let slice = if let Some((_, _, _, ms)) = &selection {
                let elapsed = self.selection_frame.unwrap().elapsed()?;
                slice.min(Duration::from_millis(*ms).saturating_sub(elapsed))
            } else {
                slice
            };
            let slice = if let Some(wait) = self.scrollbar_wait()? {
                slice.min(wait)
            } else {
                slice
            };
            let slice = if let Some(tooltip) = self.tooltip_wait()? {
                slice.min(tooltip)
            } else {
                slice
            };
            let event = self.window.next_event(slice)?;
            // Deliver input first: a pending Up/Leave/Close must cancel capture
            // before a due selection timer can move the viewport again.
            if event.is_none() {
                self.sample_selection_autoscroll()?;
                self.tick_tooltip()?;
                self.tick_scrollbars()?;
            }
            if let Some(ms) = self.renderer.caret_blink_ms() {
                if self
                    .caret_frame
                    .map(|t| t.elapsed())
                    .transpose()?
                    .is_some_and(|d| d >= Duration::from_millis(ms))
                {
                    if self.application.blink_caret() {
                        self.redraw()?;
                    }
                    self.caret_frame = Some(crate::time::Instant::now()?);
                }
            }
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
            if let Some(interval) = self.animation_interval() {
                if self
                    .motion_frame
                    .map(|t| t.elapsed())
                    .transpose()?
                    .map_or(true, |elapsed| elapsed >= interval)
                {
                    self.redraw()?;
                }
            }
            if event.is_some() || start.elapsed()? >= timeout {
                break event;
            }
        };
        self.dispatch_event(event)
    }
    fn dispatch_event_default(
        &mut self,
        event: Option<WindowEvent>,
    ) -> NativeResult<Option<WindowEvent>>
    where
        C::Message: Clone,
    {
        if matches!(
            &event,
            Some(WindowEvent::Resized { .. }) | Some(WindowEvent::CloseRequested)
        ) || matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_KEY || i.kind == IN_SCROLL)
        {
            self.text_clicks.cancel();
        }
        if self.overlay_event(&event)? {
            if matches!(&event,Some(WindowEvent::Input(i)) if i.kind == IN_DOWN || i.kind == IN_LEAVE)
            {
                self.text_clicks.cancel();
            }
            return Ok(event);
        }
        if self.list_event(&event)? {
            return Ok(event);
        }
        if self.workspace_event(&event)? {
            return Ok(event);
        }
        match &event {
            Some(WindowEvent::Resized { .. }) => self.redraw()?,
            Some(WindowEvent::Input(input))
                if matches!(input.kind, IN_MOVE | IN_DOWN | IN_UP | IN_LEAVE) =>
            {
                if matches!(input.kind, IN_DOWN | IN_UP) && input.pointer_button() != 0 {
                    return Ok(event);
                }
                self.pointer_position = if input.kind == IN_LEAVE {
                    None
                } else {
                    Some((input.x, input.y))
                };
                if input.kind != IN_MOVE {
                    self.selection_frame = Some(crate::time::Instant::now()?);
                }
                let pointer = match input.kind {
                    IN_DOWN => PointerEvent::Down,
                    IN_UP => PointerEvent::Up,
                    IN_LEAVE => PointerEvent::Leave,
                    _ => PointerEvent::Move,
                };
                if let Some(key) =
                    self.renderer
                        .scroll_pointer(pointer, input.x as f32, input.y as f32)
                {
                    self.application
                        .scrollbar_pointer(pointer, &key)
                        .map_err(native)?;
                    self.text_clicks.cancel();
                    self.redraw()?;
                    return Ok(event);
                }
                let target = self
                    .renderer
                    .hit_key(input.x as f32, input.y as f32)
                    .map(alloc::string::String::from);
                if pointer == PointerEvent::Down {
                    if let Some((key, column)) = target.as_deref().and_then(|k| {
                        self.renderer
                            .table_header_at(k, input.x as f32, input.y as f32)
                            .map(|c| (k, c))
                    }) {
                        if self.application.table_header(key, column).map_err(native)? {
                            self.text_clicks.cancel();
                            self.redraw()?;
                            return Ok(event);
                        }
                    }
                    if let Some((key, row, column)) = target.as_deref().and_then(|k| {
                        self.renderer
                            .table_cell_at(k, input.x as f32, input.y as f32)
                            .map(|(r, c)| (k, r, c))
                    }) {
                        if self
                            .application
                            .table_pointer(key, row, column, input.pointer_modifiers() & 3)
                            .map_err(native)?
                        {
                            self.text_clicks.cancel();
                            self.redraw()?;
                            return Ok(event);
                        }
                    }
                    if let Some((key, row)) = target.as_deref().and_then(|k| {
                        self.renderer
                            .tree_toggle_at(k, input.x as f32, input.y as f32)
                            .map(|row| (k, row))
                    }) {
                        if self.application.tree_toggle_row(key, row).map_err(native)? {
                            self.text_clicks.cancel();
                            self.redraw()?;
                            return Ok(event);
                        }
                    }
                    let row = target
                        .as_deref()
                        .and_then(|k| self.renderer.list_geometry(k))
                        .and_then(|g| g.row_at(input.x as f32, input.y as f32));
                    if self
                        .application
                        .list_pointer(
                            pointer,
                            target.as_deref(),
                            row,
                            input.pointer_modifiers() & 3,
                        )
                        .map_err(native)?
                    {
                        self.text_clicks.cancel();
                        self.redraw()?;
                        return Ok(event);
                    }
                }
                let spin_key = if pointer == PointerEvent::Down {
                    target.clone()
                } else {
                    self.application
                        .pressed_spinbox_key()
                        .map(alloc::string::String::from)
                        .or_else(|| target.clone())
                };
                let spin_button = spin_key.as_deref().and_then(|key| {
                    self.renderer
                        .spinbox_button_at(key, input.x as f32, input.y as f32)
                });
                if self
                    .application
                    .spinbox_pointer(pointer, target.as_deref(), spin_button)
                    .map_err(native)?
                {
                    self.text_clicks.cancel();
                    self.redraw()?;
                    return Ok(event);
                }
                let slider_key = if pointer == PointerEvent::Down {
                    target.clone()
                } else {
                    self.application
                        .dragging_slider_key()
                        .map(alloc::string::String::from)
                        .or_else(|| target.clone())
                };
                let slider_position = slider_key.as_deref().and_then(|key| {
                    self.renderer
                        .slider_geometry(key)
                        .map(|g| (key, g.coordinate(input.x as f32, input.y as f32), g))
                });
                if self
                    .application
                    .slider_pointer(pointer, target.as_deref(), slider_position)
                    .map_err(native)?
                {
                    self.text_clicks.cancel();
                    self.redraw()?;
                    return Ok(event);
                }
                let capture = if pointer == PointerEvent::Down {
                    target.clone()
                } else {
                    self.application
                        .dragging_input_key()
                        .map(alloc::string::String::from)
                };
                let text_position = capture.as_deref().and_then(|key| {
                    if pointer == PointerEvent::Down {
                        self.renderer
                            .input_position_at(key, input.x as f32, input.y as f32)
                    } else {
                        self.renderer
                            .input_drag_position_at(key, input.x as f32, input.y as f32)
                    }
                });
                let position = capture.as_deref().zip(text_position.map(|p| p.index));
                if self.text_pointer(
                    pointer,
                    target.as_deref(),
                    position,
                    input.x as f32,
                    input.y as f32,
                    input.pointer_modifiers(),
                )? {
                    if let Some((key, position)) = capture.as_deref().zip(text_position) {
                        self.application.set_input_affinity(key, position);
                    }
                    self.caret_frame = Some(crate::time::Instant::now()?);
                    self.redraw()?;
                }
            }
            Some(WindowEvent::Input(input)) if input.kind == IN_SCROLL => {
                let hovered = self
                    .pointer_position
                    .and_then(|(x, y)| self.renderer.hit_key(x as f32, y as f32))
                    .map(alloc::string::String::from);
                let target = hovered.or_else(|| {
                    self.application
                        .focused_key()
                        .map(alloc::string::String::from)
                });
                if self.renderer.scroll_wheel(target.as_deref(), input.x) {
                    self.redraw()?;
                }
            }
            Some(WindowEvent::Input(input)) if input.kind == IN_KEY => {
                if input.value == 0 && input.y & !3 == 0 {
                    let extend = input.y & 1 != 0;
                    let word = input.y & 2 != 0;
                    let command = match input.x {
                        0x4b => Some(EditCommand::Left { extend, word }),
                        0x4d => Some(EditCommand::Right { extend, word }),
                        0x47 if word => Some(EditCommand::DocumentHome { extend }),
                        0x4f if word => Some(EditCommand::DocumentEnd { extend }),
                        0x47 => Some(EditCommand::Home { extend }),
                        0x4f => Some(EditCommand::End { extend }),
                        0x48 => Some(EditCommand::Up { extend }),
                        0x50 => Some(EditCommand::Down { extend }),
                        0x53 => Some(EditCommand::Delete { word }),
                        0x0e => Some(EditCommand::Backspace { word }),
                        _ => None,
                    };
                    if let Some(command) = command {
                        if self
                            .renderer
                            .edit_input(&mut self.application, command)
                            .map_err(native)?
                        {
                            self.caret_frame = Some(crate::time::Instant::now()?);
                            self.redraw()?;
                            return Ok(event);
                        }
                    }
                }
                if input.value == 0 && input.y == 0 {
                    let command = match input.x {
                        0x47 => Some(SliderCommand::Home),
                        0x4f => Some(SliderCommand::End),
                        0x49 => Some(SliderCommand::PageUp),
                        0x51 => Some(SliderCommand::PageDown),
                        _ => None,
                    };
                    if let Some(command) = command {
                        if matches!(command, SliderCommand::PageUp | SliderCommand::PageDown)
                            && self.application.spinbox_step(command).map_err(native)?
                        {
                            self.redraw()?;
                            return Ok(event);
                        }
                        if self.application.slider_command(command).map_err(native)? {
                            self.redraw()?;
                            return Ok(event);
                        }
                    }
                }
                // Existing IN_KEY carries PS/2 set-1 scancode and modifiers;
                // extended arrow keys have no cooked character.
                let navigation = if input.value == 0 && input.y == 0 {
                    match input.x {
                        0x4b => Some(Navigation::Left),
                        0x4d => Some(Navigation::Right),
                        0x48 => Some(Navigation::Up),
                        0x50 => Some(Navigation::Down),
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some(direction) = navigation {
                    if self.application.navigate(direction).map_err(native)? {
                        self.redraw()?;
                        return Ok(event);
                    }
                }
                if input.value == 0 && input.y == 0 {
                    if let Some(command) = match input.x {
                        0x4b => Some(ScrollCommand::LineUp),
                        0x4d => Some(ScrollCommand::LineDown),
                        _ => None,
                    } {
                        if self.renderer.scroll_key_axis(
                            self.application.view(),
                            command,
                            ScrollDirection::Horizontal,
                        ) {
                            self.redraw()?;
                            return Ok(event);
                        }
                    }
                    let command = match input.x {
                        0x48 => Some(ScrollCommand::LineUp),
                        0x50 => Some(ScrollCommand::LineDown),
                        0x49 => Some(ScrollCommand::PageUp),
                        0x51 => Some(ScrollCommand::PageDown),
                        0x47 => Some(ScrollCommand::Home),
                        0x4f => Some(ScrollCommand::End),
                        _ => None,
                    };
                    if command
                        .is_some_and(|cmd| self.renderer.scroll_key(self.application.view(), cmd))
                    {
                        self.redraw()?;
                        return Ok(event);
                    }
                }
                if let Some(key) = u8::try_from(input.y)
                    .ok()
                    .and_then(|modifiers| Key::from_native(input.x, input.value, modifiers))
                {
                    if self.application.key(key).map_err(native)? {
                        self.caret_frame = Some(crate::time::Instant::now()?);
                        self.redraw()?;
                    }
                }
            }
            _ => {}
        }
        if matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_MOVE) {
            self.sample_selection_autoscroll()?;
        }
        Ok(event)
    }
    fn sample_selection_autoscroll(&mut self) -> NativeResult<()> {
        let Some((key, x, y, ms)) = self.selection_autoscroll() else {
            self.selection_frame = None;
            return Ok(());
        };
        let elapsed = self.selection_frame.map(|t| t.elapsed()).transpose()?;
        if elapsed.is_some_and(|d| d >= Duration::from_millis(ms)) {
            if let Some(position) = self.renderer.input_autoscroll_position_step(&key, x, y) {
                self.application
                    .drag_text_selection(&key, position.index, &mut self.text_clicks);
                self.application.set_input_affinity(&key, position);
                self.caret_frame = Some(crate::time::Instant::now()?);
                self.redraw()?;
            }
            self.selection_frame = Some(crate::time::Instant::now()?);
        }
        Ok(())
    }
    fn selection_autoscroll(&self) -> Option<(alloc::string::String, f32, f32, u64)> {
        let key = self.application.dragging_input_key()?;
        let (x, y) = self.pointer_position?;
        let (x, y) = (x as f32, y as f32);
        let ms = self.renderer.input_autoscroll_ms(key, x, y)?;
        Some((key.into(), x, y, ms))
    }
    pub fn close(self) -> NativeResult<()> {
        self.window.close()
    }
}
