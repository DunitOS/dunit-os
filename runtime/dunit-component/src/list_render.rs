use super::{Renderer, Snapshot};
use crate::{ListGeometry, View};
use alloc::vec::Vec;
use dunit_render::{ListPaint, ListRowPaint, ScrollArea};
use dunit_style::Style;
use dunit_ui::{Layout, Tree};
impl Renderer {
    pub fn tree_toggle_at(&self, key: &str, x: f32, y: f32) -> Option<usize> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        let list = p.lists[id.0 as usize].as_ref()?;
        let index = list.geometry.row_at(x, y)?;
        let tree = list.rows.iter().find(|r| r.index == index)?.tree.as_ref()?;
        tree.expanded?;
        let style = &p.styles[id.0 as usize];
        let rect = dunit_widgets::tree::disclosure(
            list.geometry,
            index,
            tree.depth,
            style.number("tree-indent")?,
            style.number("tree-toggle-width")?,
        )?;
        (x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h).then_some(index)
    }
    pub fn list_geometry(&self, key: &str) -> Option<ListGeometry> {
        let p = self.previous.as_ref()?;
        Some(p.lists[p.tree.by_name(key)?.0 as usize].as_ref()?.geometry)
    }
    /// Number of materialized paint rows is independent of total model length.
    pub fn list_visible_rows(&self, key: &str) -> Option<usize> {
        let p = self.previous.as_ref()?;
        Some(
            p.lists[p.tree.by_name(key)?.0 as usize]
                .as_ref()?
                .rows
                .len(),
        )
    }
    pub(super) fn prepare_lists<M>(
        &self,
        tree: &Tree,
        layout: &Layout,
        styles: &[Style],
        views: &[&View<M>],
        areas: &[Option<ScrollArea>],
        width: u32,
        height: u32,
    ) -> Vec<Option<ListPaint>> {
        tree.iter()
            .map(|n| {
                let v = views[n.id.0 as usize];
                let l = v.list.as_ref()?;
                let g = ListGeometry {
                    viewport: areas[n.id.0 as usize].as_ref()?.viewport,
                    offset: self.scroll_offset(&v.key).unwrap_or(0.),
                    row_height: styles[n.id.0 as usize]
                        .number("list-row-height")
                        .unwrap_or(0.),
                    count: l.items.len(),
                };
                let range =
                    super::visible_rect(tree, layout, areas, n.id, g.viewport, width, height)
                        .map(|r| {
                            ListGeometry {
                                viewport: r,
                                offset: g.offset + r.y - g.viewport.y,
                                ..g
                            }
                            .visible()
                        })
                        .unwrap_or(0..0);
                Some(ListPaint {
                    geometry: g,
                    focused: v.interaction.focused && !v.interaction.disabled,
                    rows: range
                        .map(|index| {
                            let i = &l.items[index];
                            ListRowPaint {
                                tree: v.tree.as_ref().and_then(|t| t.rows.get(index)).map(|r| {
                                    dunit_render::TreeRowPaint {
                                        depth: r.depth,
                                        expanded: r.expanded,
                                    }
                                }),
                                index,
                                text: i.label.clone(),
                                selected: l.selection.selected.contains(&i.key),
                                current: l.selection.current.as_ref() == Some(&i.key),
                                disabled: i.disabled || v.interaction.disabled,
                            }
                        })
                        .collect(),
                })
            })
            .collect()
    }
}
impl Snapshot {
    pub(super) fn list_current(&self, key: &str) -> Option<(&str, usize)> {
        self.list_current[self.tree.by_name(key)?.0 as usize]
            .as_ref()
            .map(|(k, i)| (k.as_str(), *i))
    }
}
