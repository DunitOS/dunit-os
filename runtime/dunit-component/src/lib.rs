//! Pure retained component model and CPU rendering, shared by host and guest.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

mod routing;
pub use routing::{EventPhase, EventResponse, RouteOutcome, UiEvent, UiEventKind};
mod file_dialog;
pub use file_dialog::{
    DirectorySource, FileDialog, FileDialogLabels, FileDialogMessage, FileDialogMode,
    FileDialogOptions, FileDialogOutcome, FileEntry, FileFilter, FileKind,
};
mod callback;
mod click_selection;
mod layout_style;
pub use click_selection::{TextClickConfig, TextClickSample, TextClickState, TextSelectionUnit};
mod message_map;
mod rename;
pub use rename::{RenameEvent, RenameState};
mod table;
pub use table::{
    SortOrder, TableCellState, TableColumn, TableCommand, TableError, TableModel, TableRow,
    TableSort, TableState, TableValue,
};
mod tree;
mod tree_events;
pub use tree::{TreeCommand, TreeError, TreeItem, TreeItemState, TreeModel, TreeRow, TreeState};
mod list;
pub use list::{ListCommand, ListGeometry, ListItem, ListSelection, SelectionMode};
mod focus;
mod splitter;
mod workspace;
pub use splitter::SplitGeometry;
mod workspace_events;
pub use workspace::{DialogAction, SplitAxis, SplitterOptions, Tab, WorkspaceCommand};
mod popup;
mod popup_events;
mod tooltip;
pub use popup::{MenuEntry, MenuItem, OpenPopup, PopupCommand, PopupHit, PopupItem, SelectOption};
mod progress;
pub use progress::{ProgressOptions, ProgressState};
mod spinbox;
pub use spinbox::{SpinButton, SpinboxOptions, SpinboxRange};
mod slider;
pub use slider::{SliderCommand, SliderGeometry, SliderOptions, SliderOrientation, SliderRange};
mod input;
pub use input::InputContext;
mod interaction;
mod lifecycle;
mod motion;
mod renderer;
mod theme;
mod view;
pub use dunit_render::{CaretAffinity, TextPosition};
pub use dunit_style::value::Color;
pub use dunit_style::{self as style, Cascade};
pub use dunit_text::Font;
pub use dunit_ui::{Align, Attrs, Edges, Rect, Sizing};
pub use dunit_widgets::file_browser;
pub use dunit_widgets::ScrollCommand;
pub use focus::Navigation;
pub use input::{EditCommand, InputOptions, TextEditor};
pub use interaction::PointerEvent;
pub use renderer::{Damage, RenderStats, Renderer};
pub use theme::Theme;
pub use view::{Key, View};

/// Shared retained-tree resource bounds, including native adapters.
pub const MAX_VIEW_NODES: usize = 4096;
pub const MAX_VIEW_DEPTH: usize = 64;

/// Content axes enabled by a scroll container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollAxes {
    #[default]
    Vertical,
    Horizontal,
    Both,
}
impl ScrollAxes {
    pub fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }
    pub fn vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Horizontal,
    Vertical,
}

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
    InvalidSlider,
    InvalidSpinbox,
    InvalidProgress,
    InvalidPopup,
    InvalidWorkspace,
    InvalidList,
    InvalidTree,
    InvalidTable,
    InvalidRename,
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
    pub(crate) keyboard_press: Option<lifecycle::KeyboardPress>,
    pub(crate) held_activation_scans: [bool; 128],
    pub(crate) window_focused: bool,
    pub(crate) clipboard_revision:u64,
    pub(crate) suspended_focus: Option<alloc::string::String>,
    pub(crate) event_dispatch_depth: usize,
    pub(crate) rename_focus: Option<(alloc::string::String, alloc::string::String)>,
    pub(crate) dialog_focus: workspace::DialogFocus,
    component: C,
    view: View<C::Message>,
    pub(crate) overlays: popup::OverlayState,
}
impl<C: Component> Application<C> {
    pub fn new(component: C) -> Result<Self> {
        let view = component.view();
        view.validate()?;
        let mut app = Self {
            keyboard_press: None,
            held_activation_scans: [false; 128],
            window_focused: true,
            clipboard_revision:0,
            suspended_focus: None,
            event_dispatch_depth: 0,
            rename_focus: None,
            dialog_focus: Default::default(),
            component,
            view,
            overlays: popup::OverlayState::default(),
        };
        app.sync_dialog(None);
        app.sync_rename();
        Ok(app)
    }
    pub fn component(&self) -> &C {
        &self.component
    }
    pub fn view(&self) -> &View<C::Message> {
        &self.view
    }
    pub fn update(&mut self, message: C::Message) -> Result<()> {
        self.clipboard_revision=self.clipboard_revision.wrapping_add(1);
        let focused = self.focused_key().map(alloc::string::String::from);
        self.component.update(message);
        let mut view = self.component.view();
        view.validate()?;
        self.reconcile_overlays(&view);
        view.reconcile(&self.view);
        self.view = view;
        self.sync_dialog(focused);
        self.sync_rename();
        self.reconcile_keyboard_press();
        Ok(())
    }
    /// Dispatch focused text editing and control navigation before root shortcuts.
    pub fn key(&mut self, key: Key) -> Result<bool>
    where
        C::Message: Clone,
    {
        let target = self.focused_key().map(alloc::string::String::from);
        self.dispatch_input(target.as_deref(), UiEventKind::Key(key), |app| {
            app.key_default(key)
        })
    }
    fn key_default(&mut self, key: Key) -> Result<bool>
    where
        C::Message: Clone,
    {
        self.dismiss_tooltip();
        if self.popup_key(key)?
            || self.mnemonic_key(key)?
            || self.rename_key(key)?
            || self.list_key(key)?
            || self.workspace_key(key)?
            || self.spinbox_key(key)?
            || self.submit_key(key)?
            || self.text_key(key)?
            || self.dialog_default(key)?
            || self.widget_key(key)?
        {
            return Ok(true);
        }
        if self.dialog_focus.key.is_some() {
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
