//! Validated stable-key hierarchy. Only visible rows feed collection painting.
use crate::selection::{ListGeometry, ListItem, ListSelection};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    string::String,
    vec,
    vec::Vec,
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeItem {
    pub item: ListItem,
    pub parent: Option<String>,
}
impl TreeItem {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            item: ListItem::new(key, label),
            parent: None,
        }
    }
    pub fn under(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.item.disabled = value;
        self
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeError {
    EmptyKey,
    DuplicateKey,
    MissingParent,
    Cycle,
    DepthLimit,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Data {
    items: Vec<TreeItem>,
    index: BTreeMap<String, usize>,
    children: Vec<Vec<usize>>,
    parents: Vec<Option<usize>>,
    depth: Vec<usize>,
    disabled: Vec<bool>,
    order: Vec<usize>,
    positions: Vec<(usize, usize)>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeModel(Rc<Data>);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRow {
    pub index: usize,
    pub depth: usize,
    pub expanded: Option<bool>,
    pub position: usize,
    pub siblings: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TreeState {
    pub selection: ListSelection,
    pub expanded: BTreeSet<String>,
}
impl TreeModel {
    pub fn new(items: Vec<TreeItem>) -> Result<Self, TreeError> {
        let n = items.len();
        let mut index = BTreeMap::new();
        for (i, node) in items.iter().enumerate() {
            if node.item.key.is_empty() {
                return Err(TreeError::EmptyKey);
            }
            if index.insert(node.item.key.clone(), i).is_some() {
                return Err(TreeError::DuplicateKey);
            }
        }
        let mut children = vec![Vec::new(); n];
        let mut parents = vec![None; n];
        let mut roots = Vec::new();
        for (i, node) in items.iter().enumerate() {
            if let Some(key) = &node.parent {
                let p = *index.get(key).ok_or(TreeError::MissingParent)?;
                parents[i] = Some(p);
                children[p].push(i);
            } else {
                roots.push(i);
            }
        }
        let mut depth = vec![0; n];
        let mut disabled = vec![false; n];
        let mut positions = vec![(0, 0); n];
        for siblings in core::iter::once(&roots).chain(children.iter()) {
            for (p, &i) in siblings.iter().enumerate() {
                positions[i] = (p + 1, siblings.len());
            }
        }
        let mut stack: Vec<_> = roots.iter().rev().map(|&i| (i, 0, false)).collect();
        let mut order = Vec::with_capacity(n);
        while let Some((i, d, blocked)) = stack.pop() {
            if d >= 64 {
                return Err(TreeError::DepthLimit);
            }
            depth[i] = d;
            disabled[i] = blocked || items[i].item.disabled;
            order.push(i);
            for &child in children[i].iter().rev() {
                stack.push((child, d + 1, disabled[i]));
            }
        }
        if order.len() != n {
            return Err(TreeError::Cycle);
        }
        Ok(Self(Rc::new(Data {
            items,
            index,
            children,
            parents,
            depth,
            disabled,
            order,
            positions,
        })))
    }
    pub fn items(&self) -> &[TreeItem] {
        &self.0.items
    }
    pub fn index(&self, key: &str) -> Option<usize> {
        self.0.index.get(key).copied()
    }
    pub fn parent(&self, index: usize) -> Option<usize> {
        self.0.parents.get(index).copied().flatten()
    }
    pub fn children(&self, index: usize) -> &[usize] {
        self.0.children.get(index).map_or(&[], |v| v.as_slice())
    }
    pub fn item(&self, index: usize) -> Option<ListItem> {
        let mut i = self.0.items.get(index)?.item.clone();
        i.disabled = self.0.disabled[index];
        Some(i)
    }
    pub fn visible(&self, expanded: &BTreeSet<String>) -> Vec<TreeRow> {
        let mut shown = vec![false; self.0.items.len()];
        let mut rows = Vec::new();
        for &i in &self.0.order {
            shown[i] = self.0.parents[i]
                .is_none_or(|p| shown[p] && expanded.contains(&self.0.items[p].item.key));
            if shown[i] {
                let (position, siblings) = self.0.positions[i];
                rows.push(TreeRow {
                    index: i,
                    depth: self.0.depth[i],
                    expanded: (!self.0.children[i].is_empty())
                        .then(|| expanded.contains(&self.0.items[i].item.key)),
                    position,
                    siblings,
                });
            }
        }
        rows
    }
}
impl TreeState {
    /// Drop removed/hidden selections. A hidden current/anchor moves to its nearest
    /// visible enabled ancestor; a selected current promotes that ancestor too.
    pub fn retain(&mut self, model: &TreeModel) {
        self.expanded.retain(|k| {
            model
                .index(k)
                .is_some_and(|i| !model.children(i).is_empty())
        });
        let rows = model.visible(&self.expanded);
        let enabled: BTreeSet<_> = rows
            .iter()
            .filter(|r| !model.0.disabled[r.index])
            .map(|r| model.0.items[r.index].item.key.as_str())
            .collect();
        let promote = |key: &str| {
            let mut i = model.index(key)?;
            loop {
                let k = &model.0.items[i].item.key;
                if enabled.contains(k.as_str()) {
                    return Some(k.clone());
                }
                i = model.parent(i)?;
            }
        };
        let old = self.selection.current.clone();
        let was_selected = old
            .as_ref()
            .is_some_and(|k| self.selection.selected.contains(k));
        self.selection
            .selected
            .retain(|k| enabled.contains(k.as_str()));
        self.selection.current = old.as_deref().and_then(promote);
        self.selection.anchor = self.selection.anchor.as_deref().and_then(promote);
        if was_selected && self.selection.current != old {
            if let Some(k) = &self.selection.current {
                self.selection.selected.insert(k.clone());
            }
        }
    }
}
/// The same indent/disclosure rectangle is used by paint and pointer input.
pub fn disclosure(
    geometry: ListGeometry,
    row: usize,
    depth: usize,
    indent: f32,
    width: f32,
) -> Option<dunit_ui::Rect> {
    if !geometry.visible().contains(&row)
        || row >= geometry.count
        || !indent.is_finite()
        || !width.is_finite()
        || indent < 0.
        || width <= 0.
    {
        return None;
    }
    let x = geometry.viewport.x + depth as f32 * indent;
    let right = (x + width).min(geometry.viewport.x + geometry.viewport.w);
    if !x.is_finite() || right <= x {
        return None;
    }
    Some(dunit_ui::Rect {
        x,
        y: geometry.viewport.y + row as f32 * geometry.row_height - geometry.offset,
        w: right - x,
        h: geometry.row_height,
    })
}
