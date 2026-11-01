//! Compound workspace controls, built from the common typed tree.
use crate::{Align, Sizing, SliderOptions, View};
use alloc::{format, string::String, vec, vec::Vec};
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceRole {
    Tabs {
        label: String,
        selected: String,
        ids: Vec<String>,
        valid: bool,
    },
    TabList(String),
    Tab {
        panel: String,
    },
    TabPanel {
        header: String,
    },
    Toolbar(String),
    Status(String),
    Separator,
    ModalHost,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Tab<M> {
    pub key: String,
    pub label: String,
    pub message: M,
    pub content: View<M>,
    pub disabled: bool,
}
impl<M> Tab<M> {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        message: M,
        content: View<M>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            message,
            content,
            disabled: false,
        }
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DialogBinding<M> {
    pub label: String,
    pub cancel: Option<M>,
    pub default: Option<String>,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct DialogFocus {
    pub key: Option<String>,
    pub restore: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct DialogAction<M> {
    pub key: String,
    pub label: String,
    pub message: M,
}
impl<M> DialogAction<M> {
    pub fn new(key: impl Into<String>, label: impl Into<String>, message: M) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            message,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitterOptions {
    pub axis: SplitAxis,
    pub range: SliderOptions,
}
impl SplitterOptions {
    pub fn new(axis: SplitAxis) -> Self {
        Self {
            axis,
            range: SliderOptions::new(0, 1000, 10),
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct SplitBinding<M> {
    pub label: String,
    pub value: i64,
    pub options: SplitterOptions,
    pub on_change: fn(i64) -> M,
    pub grab: f32,
}
impl<M> PartialEq for SplitBinding<M> {
    fn eq(&self, b: &Self) -> bool {
        self.label == b.label
            && self.value == b.value
            && self.options == b.options
            && self.grab == b.grab
            && core::ptr::fn_addr_eq(self.on_change, b.on_change)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceCommand {
    First,
    Last,
    NextTab,
    PreviousTab,
}
impl<M> View<M> {
    pub fn tabs(
        key: impl Into<String>,
        label: impl Into<String>,
        selected: &str,
        pages: Vec<Tab<M>>,
    ) -> Self {
        let key = key.into();
        let label = label.into();
        let ids = pages.iter().map(|p| p.key.clone()).collect();
        let mut valid = true;
        let mut headers = Vec::new();
        let mut panel = None;
        for page in pages {
            let header = format!("{key}/tab/{}", page.key);
            let panel_key = format!("{key}/panel/{}", page.key);
            valid &= page.content.validate().is_ok()
                && page.content.bindings.is_empty()
                && crate::TextEditor::valid_text(&page.label)
                && !page.key.is_empty();
            let mut tab = Self::radio(
                header.clone(),
                page.label,
                format!("tabs/{key}"),
                page.key == selected,
                page.message,
            )
            .disabled(page.disabled);
            tab.kind = dunit_ui::Kind::Element("Tab".into());
            tab.workspace = Some(WorkspaceRole::Tab {
                panel: panel_key.clone(),
            });
            headers.push(tab);
            if page.key == selected {
                let mut p = Self::column(panel_key, vec![page.content]).class("tab-panel");
                p.workspace = Some(WorkspaceRole::TabPanel { header });
                p.attrs.height = Sizing::Grow(1.);
                panel = Some(p);
            }
        }
        let mut list = Self::row(format!("{key}/headers"), headers).class("tab-list");
        list.workspace = Some(WorkspaceRole::TabList(label.clone()));
        let mut children = vec![list];
        if let Some(panel) = panel {
            children.push(panel);
        }
        let mut root = Self::column(key, children).class("tabs");
        root.attrs.height = Sizing::Grow(1.);
        root.workspace = Some(WorkspaceRole::Tabs {
            label,
            selected: selected.into(),
            ids,
            valid,
        });
        root
    }
    pub fn toolbar(key: impl Into<String>, label: impl Into<String>, commands: Vec<Self>) -> Self {
        let mut v = Self::row(key, commands).class("toolbar");
        v.workspace = Some(WorkspaceRole::Toolbar(label.into()));
        v
    }
    pub fn toolbar_separator(key: impl Into<String>) -> Self {
        let mut v = Self::new(
            key,
            dunit_ui::Kind::Element("ToolbarSeparator".into()),
            None,
            Vec::new(),
        );
        v.workspace = Some(WorkspaceRole::Separator);
        v
    }
    pub fn status_bar(
        key: impl Into<String>,
        label: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        let mut v = Self::text(key, text).class("status-bar");
        v.workspace = Some(WorkspaceRole::Status(label.into()));
        v
    }
    pub fn dialog(
        key: impl Into<String>,
        title: impl Into<String>,
        body: Self,
        actions: Vec<DialogAction<M>>,
        default: Option<&str>,
        cancel: Option<M>,
    ) -> Self {
        let key = key.into();
        let title = title.into();
        let caption = Self::text(format!("{key}/title"), title.clone()).class("dialog-title");
        let commands = actions
            .into_iter()
            .map(|a| Self::button(a.key, a.label, a.message))
            .collect();
        let action_key = format!("{key}/actions");
        let mut v = Self::column(
            key,
            vec![
                caption,
                body,
                Self::row(action_key, commands).class("dialog-actions"),
            ],
        )
        .class("dialog");
        v.dialog = Some(DialogBinding {
            label: title,
            cancel,
            default: default.map(String::from),
        });
        v
    }
    pub fn message_box(
        key: impl Into<String>,
        title: impl Into<String>,
        text: impl Into<String>,
        actions: Vec<DialogAction<M>>,
        default: Option<&str>,
        cancel: Option<M>,
    ) -> Self {
        let key = key.into();
        let title = title.into();
        {
            let text = text.into();
            let lines = text
                .lines()
                .enumerate()
                .map(|(n, line)| Self::text(format!("{key}/line/{n}"), line))
                .collect();
            Self::dialog(
                key.clone(),
                title.clone(),
                Self::scroll(format!("{key}/message"), title, lines)
                    .scroll_axes(crate::ScrollAxes::Both)
                    .class("message-body"),
                actions,
                default,
                cancel,
            )
        }
    }
    pub fn modal(key: impl Into<String>, content: Self, dialog: Option<Self>) -> Self {
        let key = key.into();
        let mut children = vec![content];
        if let Some(dialog) = dialog {
            children.push(Self::new(
                format!("{key}/scrim"),
                dunit_ui::Kind::Element("ModalScrim".into()),
                None,
                Vec::new(),
            ));
            children.push(dialog);
        }
        let mut v = Self::stack(key, children).class("modal-layer");
        v.workspace = Some(WorkspaceRole::ModalHost);
        v.attrs.main_align = Align::Center;
        v.attrs.cross_align = Align::Center;
        v
    }
    pub fn splitter(
        key: impl Into<String>,
        label: impl Into<String>,
        value: i64,
        options: SplitterOptions,
        first: Self,
        second: Self,
        on_change: fn(i64) -> M,
    ) -> Self {
        let key = key.into();
        let label = label.into();
        let mut handle = Self::new(
            format!("{key}/handle"),
            dunit_ui::Kind::Element("SplitterHandle".into()),
            None,
            Vec::new(),
        );
        handle.split = Some(SplitBinding {
            label,
            value,
            options,
            on_change,
            grab: 0.,
        });
        let children = vec![first, handle, second];
        let mut v = if options.axis == SplitAxis::Horizontal {
            Self::row(key, children)
        } else {
            Self::column(key, children)
        }
        .class("splitter");
        v.attrs.height = Sizing::Grow(1.);
        v
    }
    pub fn selected_tab(&self) -> Option<&str> {
        if let Some(WorkspaceRole::Tabs { selected, .. }) = &self.workspace {
            Some(selected)
        } else {
            None
        }
    }
    pub fn splitter_value(&self) -> Option<i64> {
        self.split.as_ref().map(|s| s.value)
    }
    pub fn accessibility_orientation(&self) -> Option<&'static str> {
        if let Some(s) = &self.split {
            return Some(if s.options.axis == SplitAxis::Horizontal {
                "vertical"
            } else {
                "horizontal"
            });
        }
        if matches!(self.workspace, Some(WorkspaceRole::Separator)) {
            return Some("vertical");
        }
        matches!(
            self.workspace,
            Some(WorkspaceRole::TabList(_)) | Some(WorkspaceRole::Toolbar(_))
        )
        .then_some("horizontal")
    }
    pub fn accessibility_selected(&self) -> Option<bool> {
        matches!(self.workspace, Some(WorkspaceRole::Tab { .. }))
            .then_some(self.interaction.checked)
    }
    pub fn accessibility_controls(&self) -> Option<&str> {
        if let Some(WorkspaceRole::Tab { panel }) = &self.workspace {
            if !self.interaction.checked {
                return None;
            }
            Some(panel)
        } else {
            None
        }
    }
    pub fn accessibility_labelled_by(&self) -> Option<&str> {
        if let Some(WorkspaceRole::TabPanel { header }) = &self.workspace {
            Some(header)
        } else {
            None
        }
    }
    pub(crate) fn workspace_role(&self) -> Option<&'static str> {
        Some(match &self.workspace {
            Some(WorkspaceRole::Tabs { .. }) => "group",
            Some(WorkspaceRole::TabList(_)) => "tablist",
            Some(WorkspaceRole::Tab { .. }) => "tab",
            Some(WorkspaceRole::TabPanel { .. }) => "tabpanel",
            Some(WorkspaceRole::Toolbar(_)) => "toolbar",
            Some(WorkspaceRole::Status(_)) => "status",
            Some(WorkspaceRole::Separator) => "separator",
            _ if self.dialog.is_some() => "dialog",
            _ if self.split.is_some() => "separator",
            _ => return None,
        })
    }
    pub(crate) fn workspace_label(&self) -> Option<&str> {
        match &self.workspace {
            Some(WorkspaceRole::Tabs { label, .. })
            | Some(WorkspaceRole::TabList(label))
            | Some(WorkspaceRole::Toolbar(label))
            | Some(WorkspaceRole::Status(label)) => Some(label),
            _ => self
                .dialog
                .as_ref()
                .map(|d| d.label.as_str())
                .or_else(|| self.split.as_ref().map(|s| s.label.as_str())),
        }
    }
    pub(crate) fn check_workspace(&self) -> crate::Result<()> {
        use crate::UiError::InvalidWorkspace;
        if let Some(WorkspaceRole::Tabs {
            selected,
            ids,
            valid,
            ..
        }) = &self.workspace
        {
            let mut set = alloc::collections::BTreeSet::new();
            if !*valid
                || !ids.iter().all(|id| set.insert(id))
                || !ids.iter().any(|id| id == selected)
            {
                return Err(InvalidWorkspace);
            }
        }
        if let Some(split) = &self.split {
            if split.options.range.min < 0
                || split.options.range.max > 1000
                || !split.options.range.valid(split.value)
            {
                return Err(InvalidWorkspace);
            }
        }
        if matches!(self.workspace, Some(WorkspaceRole::Toolbar(_))) {
            for child in &self.children {
                let mut nodes = Vec::new();
                child.flatten(&mut nodes);
                if nodes.into_iter().any(|v| {
                    v.focusable()
                        && (v.key != child.key || (v.activation.is_none() && v.popup.is_none()))
                }) {
                    return Err(InvalidWorkspace);
                }
            }
        }
        if let Some(dialog) = &self.dialog {
            if !crate::TextEditor::valid_text(&dialog.label) {
                return Err(InvalidWorkspace);
            }
            if dialog
                .default
                .as_ref()
                .is_some_and(|key| self.find(key).is_none_or(|v| v.activation.is_none()))
            {
                return Err(InvalidWorkspace);
            }
        }
        if matches!(self.workspace, Some(WorkspaceRole::ModalHost))
            && (self.children.len() != 1
                && (self.children.len() != 3 || self.children[2].dialog.is_none()))
        {
            return Err(InvalidWorkspace);
        }
        Ok(())
    }
}
