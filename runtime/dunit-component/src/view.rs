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
/// Initial elements are text and layout containers; widget behaviour is separate.
#[derive(Debug, Clone, PartialEq)]
pub struct View<Msg> {
    pub(crate) key: String,
    pub(crate) kind: Kind,
    pub(crate) attrs: Attrs,
    pub(crate) text: Option<String>,
    pub(crate) children: Vec<View<Msg>>,
    pub(crate) classes: Vec<String>,
    pub(crate) bindings: Vec<(Key, Msg)>,
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
        self.check(0, &mut names)
    }
    fn check<'a>(&'a self, depth: usize, names: &mut BTreeSet<&'a str>) -> Result<()> {
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
            child.check(depth + 1, names)?;
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
