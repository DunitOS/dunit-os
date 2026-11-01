//! Stable-key selection shared by collection controls. The caller owns state.
use alloc::{collections::BTreeSet, string::String};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Single,
    Multiple,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListSelection {
    pub selected: BTreeSet<String>,
    pub current: Option<String>,
    pub anchor: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub key: String,
    pub label: String,
    pub disabled: bool,
}
impl ListItem {
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
impl ListSelection {
    /// No hidden index identity: reorder preserves selection; removal drops it.
    pub fn retain(&mut self, items: &[ListItem]) {
        let enabled: BTreeSet<_> = items
            .iter()
            .filter(|i| !i.disabled)
            .map(|i| &i.key)
            .collect();
        self.selected.retain(|k| enabled.contains(k));
        if self.current.as_ref().is_some_and(|k| !enabled.contains(k)) {
            self.current = None;
        }
        if self.anchor.as_ref().is_some_and(|k| !enabled.contains(k)) {
            self.anchor = None;
        }
    }
    pub fn choose(
        &self,
        items: &[ListItem],
        mode: SelectionMode,
        index: usize,
        shift: bool,
        ctrl: bool,
        toggle: bool,
    ) -> Self {
        let Some(item) = items.get(index).filter(|i| !i.disabled) else {
            return self.clone();
        };
        let mut next = self.clone();
        next.retain(items);
        next.current = Some(item.key.clone());
        if mode == SelectionMode::Single {
            next.selected.clear();
            next.selected.insert(item.key.clone());
            next.anchor = next.current.clone();
            return next;
        }
        if shift {
            let start = next
                .anchor
                .as_ref()
                .or(self.current.as_ref())
                .and_then(|k| items.iter().position(|i| &i.key == k))
                .unwrap_or(index);
            if !ctrl {
                next.selected.clear();
            }
            for row in &items[start.min(index)..=start.max(index)] {
                if !row.disabled {
                    next.selected.insert(row.key.clone());
                }
            }
            if next.anchor.is_none() {
                next.anchor = Some(items[start].key.clone());
            }
        } else if toggle {
            if !next.selected.remove(&item.key) {
                next.selected.insert(item.key.clone());
            }
            next.anchor = next.current.clone();
        } else if !ctrl {
            next.selected.clear();
            next.selected.insert(item.key.clone());
            next.anchor = next.current.clone();
        }
        next
    }
    pub fn select_all(&self, items: &[ListItem], mode: SelectionMode) -> Self {
        if mode == SelectionMode::Single {
            return self.clone();
        }
        let mut next = self.clone();
        next.selected = items
            .iter()
            .filter(|i| !i.disabled)
            .map(|i| i.key.clone())
            .collect();
        next
    }
}
/// Uniform-height rows share the same visible range and pointer mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListGeometry {
    pub viewport: dunit_ui::Rect,
    pub offset: f32,
    pub row_height: f32,
    pub count: usize,
}
impl ListGeometry {
    fn valid(self) -> bool {
        [
            self.viewport.x,
            self.viewport.y,
            self.viewport.w,
            self.viewport.h,
            self.offset,
            self.row_height,
        ]
        .iter()
        .all(|v| v.is_finite())
            && self.viewport.w >= 0.
            && self.viewport.h >= 0.
            && self.offset >= 0.
            && self.row_height > 0.
    }
    pub fn visible(self) -> core::ops::Range<usize> {
        if !self.valid() || self.viewport.w <= 0. || self.viewport.h <= 0. {
            return 0..0;
        }
        let start = (self.offset / self.row_height).max(0.) as usize;
        let bottom = (self.offset.max(0.) + self.viewport.h) / self.row_height;
        let end = (bottom as usize).saturating_add(usize::from(bottom > bottom as usize as f32));
        start.min(self.count)..end.min(self.count)
    }
    pub fn row_at(self, x: f32, y: f32) -> Option<usize> {
        if !self.valid()
            || !x.is_finite()
            || !y.is_finite()
            || x < self.viewport.x
            || x >= self.viewport.x + self.viewport.w
            || y < self.viewport.y
            || y >= self.viewport.y + self.viewport.h
            || self.row_height <= 0.
        {
            return None;
        }
        let i = ((y - self.viewport.y + self.offset) / self.row_height) as usize;
        (i < self.count).then_some(i)
    }
    pub fn page(self) -> usize {
        if !self.valid() {
            return 1;
        }
        ((self.viewport.h / self.row_height) as usize)
            .saturating_sub(1)
            .max(1)
    }
}
