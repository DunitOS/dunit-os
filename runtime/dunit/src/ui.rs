//! Native window adapter for the pure retained component runtime.
use crate::{
    time::Duration,
    window::{Window, WindowEvent, IN_KEY},
    Error, Result as NativeResult,
};
pub use dunit_component::*;

fn native(error: UiError) -> Error {
    Error(match error {
        UiError::Allocation => libdunit::ENOMEM,
        UiError::DuplicateKey
        | UiError::EmptyKey
        | UiError::DuplicateBinding
        | UiError::ChildBinding
        | UiError::TreeLimit
        | UiError::InvalidLayout
        | UiError::InvalidSize => libdunit::EINVAL,
    })
}

/// One root component in the existing single-window IPC session. Shortcuts are
/// dispatched through `update`; resize redraws at the compositor's geometry.
/// CPU invalidation is retained locally. Presentation still copies a complete
/// frame into fresh shared storage, since live compositor damage is unfinished.
pub struct ComponentWindow<C: Component> {
    application: Application<C>,
    renderer: Renderer,
    window: Window,
    last_render: RenderStats,
    failed: bool,
}
impl<C: Component> ComponentWindow<C> {
    pub fn open(width: u32, height: u32, component: C, renderer: Renderer) -> NativeResult<Self> {
        let application = Application::new(component).map_err(native)?;
        let mut ui = Self {
            application,
            renderer,
            window: Window::open(width, height)?,
            last_render: RenderStats::default(),
            failed: false,
        };
        ui.redraw()?;
        Ok(ui)
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
        let renderer = &mut self.renderer;
        let view = self.application.view();
        let last = &mut self.last_render;
        let result = self.window.try_draw(|frame| {
            let (w, h) = frame.size();
            *last = renderer.render(view, w, h).map_err(native)?;
            frame.pixels_mut().copy_from_slice(renderer.pixels());
            Ok(())
        });
        self.failed |= result.is_err();
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
        let event = self.window.next_event(timeout)?;
        match &event {
            Some(WindowEvent::Resized { .. }) => self.redraw()?,
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
