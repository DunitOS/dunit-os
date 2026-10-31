//! Declarative choices and menus; transient overlay state belongs to Application.
use crate::{Result, UiError, View};
use alloc::{collections::BTreeSet, string::String, vec::Vec};
#[derive(Debug, Clone, PartialEq)]
pub enum MenuEntry<M> {
    Action(M),
    Submenu(Vec<MenuItem<M>>),
    Separator,
}
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem<M> {
    pub key: String,
    pub label: String,
    pub disabled: bool,
    pub mnemonic: Option<char>,
    pub entry: MenuEntry<M>,
}
impl<M> MenuItem<M> {
    pub fn action(key: impl Into<String>, label: impl Into<String>, message: M) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            disabled: false,
            mnemonic: None,
            entry: MenuEntry::Action(message),
        }
    }
    pub fn submenu(key: impl Into<String>, label: impl Into<String>, items: Vec<Self>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            disabled: false,
            mnemonic: None,
            entry: MenuEntry::Submenu(items),
        }
    }
    pub fn separator(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: String::new(),
            disabled: false,
            mnemonic: None,
            entry: MenuEntry::Separator,
        }
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    pub fn mnemonic(mut self, value: char) -> Self {
        self.mnemonic = Some(value.to_ascii_lowercase());
        self
    }
    pub fn enabled(&self) -> bool {
        !self.disabled && !matches!(self.entry, MenuEntry::Separator)
    }
    pub fn accessibility_role(&self) -> &'static str {
        if matches!(self.entry, MenuEntry::Separator) {
            "separator"
        } else {
            "menuitem"
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    pub key: String,
    pub label: String,
    pub disabled: bool,
}
impl SelectOption {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            disabled: false,
        }
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Trigger<M> {
    Menu(Vec<MenuItem<M>>),
    Select {
        options: Vec<SelectOption>,
        selected: Option<String>,
        on_change: fn(String) -> M,
    },
}
impl<M: PartialEq> PartialEq for Trigger<M> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Menu(a), Self::Menu(b)) => a == b,
            (
                Self::Select {
                    options: a,
                    selected: av,
                    on_change: af,
                },
                Self::Select {
                    options: b,
                    selected: bv,
                    on_change: bf,
                },
            ) => a == b && av == bv && core::ptr::fn_addr_eq(*af, *bf),
            _ => false,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PopupBinding<M> {
    pub label: String,
    pub mnemonic: Option<char>,
    pub trigger: Trigger<M>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PopupItem {
    pub key: String,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub submenu: bool,
    pub selected: bool,
    pub mnemonic: Option<char>,
    pub role: &'static str,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OpenPopup {
    pub owner: String,
    pub context: bool,
    /// Highlighted indexes by level; usize::MAX marks an empty/disabled submenu.
    pub path: Vec<usize>,
    pub point: Option<(f32, f32)>,
    pub(crate) pressed: Option<Vec<usize>>,
    pub(crate) restore: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum PopupHit {
    Item { owner: String, path: Vec<usize> },
    Panel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupCommand {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Activate,
    Escape,
    MenuBar,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct TooltipState {
    pub owner: Option<String>,
    pub since: u64,
    pub visible: bool,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct OverlayState {
    pub open: Option<OpenPopup>,
    pub tooltip: TooltipState,
    pub suppress_up: bool,
    pub pointer_down: bool,
    pub bar_focus: alloc::collections::BTreeMap<String, String>,
}
impl<M> View<M> {
    pub fn menu(key: impl Into<String>, label: impl Into<String>, items: Vec<MenuItem<M>>) -> Self {
        let label = label.into();
        let mut v = Self::new(
            key,
            dunit_ui::Kind::Element("MenuButton".into()),
            Some(label.clone()),
            Vec::new(),
        );
        v.popup = Some(PopupBinding {
            label,
            mnemonic: None,
            trigger: Trigger::Menu(items),
        });
        v
    }
    pub fn select(
        key: impl Into<String>,
        label: impl Into<String>,
        selected: Option<&str>,
        options: Vec<SelectOption>,
        on_change: fn(String) -> M,
    ) -> Self {
        let label = label.into();
        let shown = selected
            .and_then(|key| options.iter().find(|o| o.key == key))
            .map_or_else(|| label.clone(), |o| o.label.clone());
        let mut v = Self::new(
            key,
            dunit_ui::Kind::Element("Select".into()),
            Some(shown),
            Vec::new(),
        );
        v.popup = Some(PopupBinding {
            label,
            mnemonic: None,
            trigger: Trigger::Select {
                options,
                selected: selected.map(String::from),
                on_change,
            },
        });
        v
    }
    pub fn menu_bar(key: impl Into<String>, menus: Vec<Self>) -> Self {
        let mut v = Self::row(key, menus).class("menu-bar");
        v.menu_bar = true;
        v
    }
    pub fn context_menu(mut self, items: Vec<MenuItem<M>>) -> Self {
        self.context_menu = Some(items);
        self
    }
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.tooltip = (!text.trim().is_empty()).then_some(text);
        self
    }
    pub fn menu_mnemonic(mut self, ch: char) -> Self {
        if let Some(p) = &mut self.popup {
            p.mnemonic = Some(ch.to_ascii_lowercase());
        }
        self
    }
    pub fn selected_option(&self) -> Option<&str> {
        match &self.popup.as_ref()?.trigger {
            Trigger::Select { selected, .. } => selected.as_deref(),
            _ => None,
        }
    }
    pub fn accessibility_description(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }
    pub fn accessibility_has_popup(&self) -> Option<&'static str> {
        self.popup
            .as_ref()
            .map(|p| {
                if matches!(p.trigger, Trigger::Select { .. }) {
                    "listbox"
                } else {
                    "menu"
                }
            })
            .or(self.context_menu.as_ref().map(|_| "menu"))
    }
    pub(crate) fn find(&self, key: &str) -> Option<&Self> {
        if self.key == key {
            Some(self)
        } else {
            self.children.iter().find_map(|c| c.find(key))
        }
    }
    pub(crate) fn ancestor_with(
        &self,
        target: &str,
        predicate: &impl Fn(&Self) -> bool,
    ) -> Option<&Self> {
        if self.key == target {
            return predicate(self).then_some(self);
        }
        for c in &self.children {
            if c.find(target).is_some() {
                return c
                    .ancestor_with(target, predicate)
                    .or_else(|| predicate(self).then_some(self));
            }
        }
        None
    }
    pub(crate) fn popup_items(&self, context: bool, prefix: &[usize]) -> Option<Vec<PopupItem>> {
        if !context {
            if let Some(PopupBinding {
                trigger:
                    Trigger::Select {
                        options, selected, ..
                    },
                ..
            }) = &self.popup
            {
                if !prefix.is_empty() {
                    return None;
                }
                return Some(
                    options
                        .iter()
                        .map(|o| PopupItem {
                            key: o.key.clone(),
                            label: o.label.clone(),
                            enabled: !o.disabled,
                            separator: false,
                            submenu: false,
                            selected: selected.as_deref() == Some(o.key.as_str()),
                            mnemonic: None,
                            role: "option",
                        })
                        .collect(),
                );
            }
        }
        let mut items = if context {
            self.context_menu.as_ref()?
        } else {
            match &self.popup.as_ref()?.trigger {
                Trigger::Menu(items) => items,
                _ => return None,
            }
        };
        for &index in prefix {
            items = match &items.get(index)?.entry {
                MenuEntry::Submenu(children) => children,
                _ => return None,
            };
        }
        Some(
            items
                .iter()
                .map(|i| PopupItem {
                    key: i.key.clone(),
                    label: i.label.clone(),
                    enabled: i.enabled(),
                    separator: matches!(i.entry, MenuEntry::Separator),
                    submenu: matches!(i.entry, MenuEntry::Submenu(_)),
                    selected: false,
                    mnemonic: i.mnemonic,
                    role: if matches!(i.entry, MenuEntry::Separator) {
                        "separator"
                    } else {
                        "menuitem"
                    },
                })
                .collect(),
        )
    }
    pub(crate) fn menu_entry(&self, context: bool, path: &[usize]) -> Option<&MenuItem<M>> {
        let mut items = if context {
            self.context_menu.as_ref()?
        } else {
            match &self.popup.as_ref()?.trigger {
                Trigger::Menu(items) => items,
                _ => return None,
            }
        };
        for (depth, &i) in path.iter().enumerate() {
            let item = items.get(i)?;
            if depth + 1 == path.len() {
                return Some(item);
            }
            items = match &item.entry {
                MenuEntry::Submenu(children) => children,
                _ => return None,
            };
        }
        None
    }
    pub(crate) fn check_popups(&self) -> Result<()> {
        if self.menu_bar
            && (self.kind != dunit_ui::Kind::Row
                || self.children.iter().any(|v| {
                    !matches!(v.popup.as_ref().map(|p| &p.trigger), Some(Trigger::Menu(_)))
                }))
        {
            return Err(UiError::InvalidPopup);
        }
        let mut count = 0;
        fn entries<M>(items: &[MenuItem<M>], depth: usize, count: &mut usize) -> Result<()> {
            if depth >= 16 {
                return Err(UiError::TreeLimit);
            }
            let mut keys = BTreeSet::new();
            let mut mnemonics = BTreeSet::new();
            for item in items {
                *count += 1;
                if *count > 4096 {
                    return Err(UiError::TreeLimit);
                }
                if item.key.is_empty()
                    || !keys.insert(item.key.as_str())
                    || !crate::TextEditor::valid_text(&item.label)
                    || item.mnemonic.is_some_and(|m| {
                        m.is_control() || !mnemonics.insert(m.to_ascii_lowercase())
                    })
                {
                    return Err(UiError::InvalidPopup);
                }
                if let MenuEntry::Submenu(children) = &item.entry {
                    entries(children, depth + 1, count)?;
                }
            }
            Ok(())
        }
        if let Some(p) = &self.popup {
            if !crate::TextEditor::valid_text(&p.label) || p.mnemonic.is_some_and(char::is_control)
            {
                return Err(UiError::InvalidPopup);
            }
            match &p.trigger {
                Trigger::Menu(items) => entries(items, 0, &mut count)?,
                Trigger::Select {
                    options, selected, ..
                } => {
                    let mut keys = BTreeSet::new();
                    if options.len() > 4096 {
                        return Err(UiError::TreeLimit);
                    }
                    for o in options {
                        if o.key.is_empty()
                            || !keys.insert(o.key.as_str())
                            || !crate::TextEditor::valid_text(&o.label)
                        {
                            return Err(UiError::InvalidPopup);
                        }
                    }
                    if selected
                        .as_ref()
                        .is_some_and(|s| !keys.contains(s.as_str()))
                    {
                        return Err(UiError::InvalidPopup);
                    }
                }
            }
        }
        if let Some(items) = &self.context_menu {
            entries(items, 0, &mut count)?;
        }
        if self
            .tooltip
            .as_ref()
            .is_some_and(|t| !crate::TextEditor::valid_text(t))
        {
            return Err(UiError::InvalidPopup);
        }
        Ok(())
    }
}
