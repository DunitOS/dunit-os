//! Controlled hierarchy over the shared virtual collection controller.
use crate::{SelectionMode, UiError, View};
use alloc::{collections::BTreeSet, string::String, vec::Vec};
pub use dunit_widgets::tree::{TreeError, TreeItem, TreeModel, TreeRow, TreeState};
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TreeBinding {
    pub model: TreeModel,
    pub expanded: BTreeSet<String>,
    pub rows: Vec<TreeRow>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeCommand {
    Expand,
    Collapse,
    Toggle,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeItemState<'a> {
    pub role: &'static str,
    pub key: &'a str,
    pub label: &'a str,
    pub parent: Option<&'a str>,
    pub level: usize,
    pub position: usize,
    pub siblings: usize,
    pub expanded: Option<bool>,
    pub disabled: bool,
    pub selected: bool,
    pub current: bool,
}
impl<M> View<M> {
    pub fn tree_view(
        key: impl Into<String>,
        label: impl Into<String>,
        model: TreeModel,
        state: TreeState,
        mode: SelectionMode,
        on_change: fn(TreeState) -> M,
    ) -> Self {
        let rows = model.visible(&state.expanded);
        let items = rows.iter().map(|r| model.item(r.index).unwrap()).collect();
        let mut view = Self::new(
            key,
            dunit_ui::Kind::Element("Tree".into()),
            None,
            Vec::new(),
        );
        view.list = Some(crate::list::ListBinding {
            label: label.into(),
            items,
            selection: state.selection,
            mode,
            on_change: crate::list::SelectionHandler::Tree(on_change),
        });
        view.tree = Some(TreeBinding {
            model,
            expanded: state.expanded,
            rows,
        });
        view
    }
    pub fn tree_state(&self) -> Option<TreeState> {
        Some(TreeState {
            selection: self.list.as_ref()?.selection.clone(),
            expanded: self.tree.as_ref()?.expanded.clone(),
        })
    }
    pub fn tree_item(&self, key: &str) -> Option<TreeItemState<'_>> {
        let tree = self.tree.as_ref()?;
        let list = self.list.as_ref()?;
        let (index, row) = tree
            .rows
            .iter()
            .enumerate()
            .find(|(_, r)| tree.model.items()[r.index].item.key == key)?;
        let node = &tree.model.items()[row.index];
        Some(TreeItemState {
            role: "treeitem",
            key: &node.item.key,
            label: &node.item.label,
            parent: node.parent.as_deref(),
            level: row.depth + 1,
            position: row.position,
            siblings: row.siblings,
            expanded: row.expanded,
            disabled: self.interaction.disabled || list.items[index].disabled,
            selected: list.selection.selected.contains(key),
            current: list.selection.current.as_deref() == Some(key),
        })
    }
    pub(crate) fn check_tree(&self) -> crate::Result<()> {
        let Some(tree) = &self.tree else {
            return Ok(());
        };
        let state = self.tree_state().ok_or(UiError::InvalidTree)?;
        let mut clean = state.clone();
        clean.retain(&tree.model);
        if clean != state {
            return Err(UiError::InvalidTree);
        }
        Ok(())
    }
}
