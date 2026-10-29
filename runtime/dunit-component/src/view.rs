use crate::{Result, UiError};
use alloc::{collections::BTreeSet, string::String, vec::Vec};
use dunit_ui::{tree::TreeBuilder, Attrs, Kind, NodeId, Tree};

/// A cooked character with the native Shift/Ctrl/Alt/Super modifier bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub character: char,
    pub modifiers: u8,
}
impl Key {
    pub const fn plain(character: char) -> Self {
        Self {
            character,
            modifiers: 0,
        }
    }
}

/// Typed tree description. Keys are global, nonempty and unique within a view.
/// Elements include text, push buttons and layout containers. The application
/// owns button interaction; this description owns identity, configuration and messages.
#[derive(Debug, Clone, PartialEq)]
pub struct View<Msg> {
    pub(crate) key: String,
    pub(crate) kind: Kind,
    pub(crate) attrs: Attrs,
    pub(crate) text: Option<String>,
    pub(crate) children: Vec<View<Msg>>,
    pub(crate) classes: Vec<String>,
    pub(crate) bindings: Vec<(Key, Msg)>,
    pub(crate) activation: Option<Msg>,
    pub(crate) interaction: dunit_widgets::Interaction,
    pub(crate) radio_group: Option<String>,
    pub(crate) input: Option<crate::input::InputBinding<Msg>>,
    pub(crate) scroll_label: Option<String>,
    pub(crate) scroll_axes: crate::ScrollAxes,
}
impl<Msg> View<Msg> {
    fn new(key: impl Into<String>, kind: Kind, text: Option<String>, children: Vec<Self>) -> Self {
        Self {
            key: key.into(),
            kind,
            attrs: Attrs::default(),
            text,
            children,
            classes: Vec::new(),
            bindings: Vec::new(),
            activation: None,
            interaction: dunit_widgets::Interaction::default(),
            radio_group: None,
            input: None,
            scroll_label: None,
            scroll_axes: crate::ScrollAxes::Vertical,
        }
    }
    pub fn text(key: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(
            key,
            Kind::Element(String::from("Text")),
            Some(text.into()),
            Vec::new(),
        )
    }
    /// A focusable push button. Its label also supplies its accessibility name.
    pub fn button(key: impl Into<String>, label: impl Into<String>, message: Msg) -> Self {
        let mut view = Self::new(
            key,
            Kind::Element(String::from("Button")),
            Some(label.into()),
            Vec::new(),
        );
        view.activation = Some(message);
        view
    }
    fn checkable(
        key: impl Into<String>,
        label: impl Into<String>,
        tag: &str,
        checked: bool,
        message: Msg,
    ) -> Self {
        let mut view = Self::button(key, label, message);
        view.kind = Kind::Element(String::from(tag));
        view.interaction.checked = checked;
        view
    }
    /// Controlled toggle: `checked` is read from the model after activation.
    pub fn toggle_button(
        key: impl Into<String>,
        label: impl Into<String>,
        checked: bool,
        message: Msg,
    ) -> Self {
        Self::checkable(key, label, "ToggleButton", checked, message)
    }
    pub fn checkbox(
        key: impl Into<String>,
        label: impl Into<String>,
        checked: bool,
        message: Msg,
    ) -> Self {
        Self::checkable(key, label, "Checkbox", checked, message)
    }
    pub fn switch(
        key: impl Into<String>,
        label: impl Into<String>,
        checked: bool,
        message: Msg,
    ) -> Self {
        Self::checkable(key, label, "Switch", checked, message)
    }
    /// At most one member of a nonempty group may be checked. The model owns
    /// selection; activating an already checked radio does not dispatch again.
    pub fn radio(
        key: impl Into<String>,
        label: impl Into<String>,
        group: impl Into<String>,
        checked: bool,
        message: Msg,
    ) -> Self {
        let mut view = Self::checkable(key, label, "Radio", checked, message);
        view.radio_group = Some(group.into());
        view
    }
    /// Controlled single-line field. The callback maps a proposed value to a
    /// model message; the next view determines whether that edit is accepted.
    pub fn input(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
        on_change: fn(String) -> Msg,
        options: crate::InputOptions,
    ) -> Self {
        let value = value.into();
        let mut view = Self::new(
            key,
            Kind::Element(String::from("Input")),
            Some(value.clone()),
            Vec::new(),
        );
        view.input = Some(crate::input::InputBinding {
            label: label.into(),
            options,
            editor: crate::TextEditor::new(value),
            on_change,
            dragging: false,
            caret_visible: true,
            caret_revision: 0,
        });
        view
    }
    /// LF-delimited, controlled multiline field. It fills available height by
    /// default; explicit layout may choose a fixed viewport instead.
    pub fn text_area(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
        on_change: fn(String) -> Msg,
        options: crate::InputOptions,
    ) -> Self {
        let mut view = Self::input(key, label, value, on_change, options);
        view.kind = Kind::Element(String::from("Textarea"));
        view.attrs.height = dunit_ui::Sizing::Grow(1.);
        if let Some(input) = &mut view.input {
            input.editor = crate::TextEditor::new_multiline(view.text.as_deref().unwrap_or(""));
        }
        view
    }
    /// Disabled buttons cannot receive focus, presses or activation.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.interaction.disabled = disabled;
        self
    }
    pub fn interaction(&self) -> dunit_widgets::Interaction {
        self.interaction
    }
    pub fn accessibility_role(&self) -> Option<&'static str> {
        if self.scroll_label.is_some() {
            return Some("region");
        }
        if self.input.is_some() {
            return Some("textbox");
        }
        self.activation.as_ref()?;
        Some(match self.kind.tag() {
            "Checkbox" => "checkbox",
            "Radio" => "radio",
            "Switch" => "switch",
            _ => "button",
        })
    }
    pub fn accessibility_checked(&self) -> Option<bool> {
        matches!(self.kind.tag(), "Checkbox" | "Radio" | "Switch")
            .then_some(self.interaction.checked)
    }
    pub fn accessibility_pressed(&self) -> Option<bool> {
        (self.kind.tag() == "ToggleButton").then_some(self.interaction.checked)
    }
    pub fn accessibility_label(&self) -> Option<&str> {
        if let Some(label) = &self.scroll_label {
            return Some(label);
        }
        if let Some(input) = &self.input {
            return Some(&input.label);
        }
        self.activation.as_ref().and(self.text.as_deref())
    }
    /// Focusable vertical viewport with a theme-sized scrollbar gutter.
    pub fn scroll(key: impl Into<String>, label: impl Into<String>, children: Vec<Self>) -> Self {
        let mut view = Self::new(key, Kind::Scroll, None, children);
        view.scroll_label = Some(label.into());
        view.attrs.height = dunit_ui::Sizing::Grow(1.);
        view
    }
    /// Choose axes explicitly; the default remains vertical.
    pub fn scroll_axes(mut self, axes: crate::ScrollAxes) -> Self {
        self.scroll_axes = axes;
        self
    }
    pub fn column(key: impl Into<String>, children: Vec<Self>) -> Self {
        Self::new(key, Kind::Column, None, children)
    }
    pub fn row(key: impl Into<String>, children: Vec<Self>) -> Self {
        Self::new(key, Kind::Row, None, children)
    }
    pub fn stack(key: impl Into<String>, children: Vec<Self>) -> Self {
        Self::new(key, Kind::Stack, None, children)
    }
    pub fn grid(key: impl Into<String>, children: Vec<Self>) -> Self {
        Self::new(key, Kind::Grid, None, children)
    }
    pub fn layout(mut self, attrs: Attrs) -> Self {
        self.attrs = attrs;
        self
    }
    /// Add a DSS appearance class chosen by the model. This does not add widget
    /// interaction behaviour. Names follow the stylesheet identifier grammar.
    pub fn class(mut self, name: impl Into<String>) -> Self {
        self.classes.push(name.into());
        self
    }
    pub fn classes(&self) -> &[String] {
        &self.classes
    }
    pub(crate) fn flatten<'a>(&'a self, output: &mut Vec<&'a Self>) {
        output.push(self);
        for child in &self.children {
            child.flatten(output);
        }
    }
    /// Bind a root-view shortcut. Duplicate or nested bindings are errors.
    pub fn on_key(mut self, key: Key, message: Msg) -> Self {
        self.bindings.push((key, message));
        self
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn content(&self) -> Option<&str> {
        self.text.as_deref()
    }
    pub fn children(&self) -> &[Self] {
        &self.children
    }
    pub fn validate(&self) -> Result<()> {
        let mut names = BTreeSet::new();
        self.check(0, &mut names, &mut BTreeSet::new())
    }
    fn check<'a>(
        &'a self,
        depth: usize,
        names: &mut BTreeSet<&'a str>,
        checked_groups: &mut BTreeSet<&'a str>,
    ) -> Result<()> {
        if depth >= 64 || names.len() >= 4096 {
            return Err(UiError::TreeLimit);
        }
        if self
            .classes
            .iter()
            .any(|name| !dunit_style::parse::valid_identifier(name))
        {
            return Err(UiError::InvalidClass);
        }
        if self.key.is_empty() {
            return Err(UiError::EmptyKey);
        }
        if !names.insert(&self.key) {
            return Err(UiError::DuplicateKey);
        }
        if let Some(input) = &self.input {
            let value = self.text.as_deref().unwrap_or("");
            let valid = if input.editor.is_multiline() {
                crate::TextEditor::valid_multiline_text
            } else {
                crate::TextEditor::valid_text
            };
            if input.editor.is_multiline() != (self.kind.tag() == "Textarea")
                || !valid(value)
                || !valid(&input.options.placeholder)
                || (input.editor.is_multiline() && input.options.password)
                || value.chars().count() > input.options.max_chars
            {
                return Err(UiError::InvalidInput);
            }
        }
        if let Some(group) = self.radio_group.as_deref() {
            if group.trim().is_empty()
                || (self.interaction.checked && !checked_groups.insert(group))
            {
                return Err(UiError::InvalidRadioGroup);
            }
        }
        if depth != 0 && !self.bindings.is_empty() {
            return Err(UiError::ChildBinding);
        }
        let mut keys = BTreeSet::new();
        for (key, _) in &self.bindings {
            if !keys.insert(*key) {
                return Err(UiError::DuplicateBinding);
            }
        }
        // Keep NaN/infinity/negative dimensions out of the layout engine.
        let a = &self.attrs;
        let sizing = [a.width, a.height].iter().all(|v| match v {
            dunit_ui::Sizing::Auto => true,
            dunit_ui::Sizing::Px(n) | dunit_ui::Sizing::Pct(n) | dunit_ui::Sizing::Grow(n) => {
                n.is_finite() && *n >= 0.0
            }
        });
        if !sizing
            || [
                a.grow,
                a.spacing,
                a.padding.top,
                a.padding.right,
                a.padding.bottom,
                a.padding.left,
            ]
            .iter()
            .any(|n| !n.is_finite() || *n < 0.0)
            || [a.min_w, a.min_h, a.max_w, a.max_h]
                .iter()
                .flatten()
                .any(|n| !n.is_finite() || *n < 0.0)
        {
            return Err(UiError::InvalidLayout);
        }
        for child in &self.children {
            child.check(depth + 1, names, checked_groups)?;
        }
        Ok(())
    }
    pub fn tree(&self) -> Result<Tree> {
        self.validate()?;
        let mut builder = TreeBuilder::new();
        let root = self.append(&mut builder, None);
        Ok(builder.finish(root))
    }
    fn append(&self, builder: &mut TreeBuilder, parent: Option<NodeId>) -> NodeId {
        let id = builder.push(
            Some(self.key.clone()),
            self.kind.clone(),
            self.attrs.clone(),
            self.text.clone(),
            parent,
        );
        for child in &self.children {
            let child = child.append(builder, Some(id));
            builder.add_child(id, child);
        }
        id
    }
}
