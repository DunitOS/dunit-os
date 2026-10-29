//! Pure retained component model and CPU rendering, shared by host and guest.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

mod focus;
mod input;
mod interaction;
mod motion;
mod renderer;
mod theme;
mod view;
pub use dunit_style::value::Color;
pub use dunit_style::{self as style, Cascade};
pub use dunit_text::Font;
pub use dunit_ui::{Align, Attrs, Edges, Rect, Sizing};
pub use dunit_widgets::ScrollCommand;
pub use focus::Navigation;
pub use input::{EditCommand, InputOptions, TextEditor};
pub use interaction::PointerEvent;
pub use renderer::{Damage, RenderStats, Renderer};
pub use theme::Theme;
pub use view::{Key, View};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiError {
    DuplicateKey,
    EmptyKey,
    DuplicateBinding,
    ChildBinding,
    TreeLimit,
    InvalidClass,
    InvalidRadioGroup,
    InvalidInput,
    InvalidLayout,
    InvalidSize,
    Allocation,
}
pub type Result<T> = core::result::Result<T, UiError>;

/// State changes belong in `update`; `view` describes the resulting interface.
pub trait Component {
    type Message;
    fn update(&mut self, message: Self::Message);
    fn view(&self) -> View<Self::Message>;
}

/// Owns the root component. Only immutable model/view access is exposed.
/// Propagate a view validation error: the model may have consumed the message,
/// while the previous valid view is retained for diagnostics.
pub struct Application<C: Component> {
    component: C,
    view: View<C::Message>,
}
impl<C: Component> Application<C> {
    pub fn new(component: C) -> Result<Self> {
        let view = component.view();
        view.validate()?;
        Ok(Self { component, view })
    }
    pub fn component(&self) -> &C {
        &self.component
    }
    pub fn view(&self) -> &View<C::Message> {
        &self.view
    }
    pub fn update(&mut self, message: C::Message) -> Result<()> {
        self.component.update(message);
        let mut view = self.component.view();
        view.validate()?;
        view.reconcile(&self.view);
        self.view = view;
        Ok(())
    }
    /// Dispatch focused text editing and control navigation before root shortcuts.
    pub fn key(&mut self, key: Key) -> Result<bool>
    where
        C::Message: Clone,
    {
        if self.text_key(key)? || self.widget_key(key)? {
            return Ok(true);
        }
        let message = self
            .view
            .bindings
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, m)| m.clone());
        if let Some(message) = message {
            self.update(message)?;
            return Ok(true);
        }
        Ok(false)
    }
}
