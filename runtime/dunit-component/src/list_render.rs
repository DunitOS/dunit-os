use super::{Renderer, Snapshot};
use crate::{ListGeometry, View};
use alloc::vec::Vec;
use dunit_render::{ListPaint, ListRowPaint, ScrollArea};
use dunit_style::Style;
use dunit_ui::{Layout, Tree};
impl Renderer {
    pub fn table_header_at(&self, key: &str, x: f32, y: f32) -> Option<usize> {
        let p = self.previous.as_ref()?;
        p.lists[p.tree.by_name(key)?.0 as usize]
            .as_ref()?
            .table
            .as_ref()?
            .header_at(x, y)
    }
    pub fn table_cell_at(&self, key: &str, x: f32, y: f32) -> Option<(usize, usize)> {
        let p = self.previous.as_ref()?;
        let l = p.lists[p.tree.by_name(key)?.0 as usize].as_ref()?;
        Some((
            l.geometry.row_at(x, y)?,
            l.table.as_ref()?.cell_at(l.geometry.viewport, x, y)?,
        ))
    }
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
                let table = v.table.as_ref().and_then(|t| {
                    let widths = t
                        .model
                        .widths(styles[n.id.0 as usize].number("table-column-width")?)?;
                    let outer = layout.rect(n.id);
                    let border = styles[n.id.0 as usize].number("border-width").unwrap_or(0.);
                    let height = styles[n.id.0 as usize].number("table-header-height")?;
                    Some(dunit_render::TablePaint {
                        header: dunit_ui::Rect {
                            x: g.viewport.x,
                            y: g.viewport.y - height,
                            w: g.viewport.w,
                            h: height.min((outer.h - border * 2.).max(0.)),
                        },
                        columns: t.model.columns().iter().map(|c| c.label.clone()).collect(),
                        widths,
                        offset: self.scroll_offset_x(&v.key).unwrap_or(0.),
                        current_column: t
                            .column
                            .as_deref()
                            .and_then(|c| t.model.column(c))
                            .unwrap_or(0),
                        sort: t.sort.as_ref().and_then(|s| {
                            t.model
                                .column(&s.column)
                                .map(|i| (i, s.order == crate::SortOrder::Ascending))
                        }),
                    })
                });
                Some(ListPaint {
                    table,
                    geometry: g,
                    focused: v.interaction.focused && !v.interaction.disabled,
                    rows: range
                        .map(|index| {
                            let i = &l.items[index];
                            ListRowPaint {
                                cells: v.table.as_ref().map(|t| {
                                    t.model.rows()[t.order[index]]
                                        .cells
                                        .iter()
                                        .map(|c| c.text())
                                        .collect()
                                }),
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
