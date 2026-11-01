//! Place the shared editor in its final scrolled row/cell, never in app code.
use super::Renderer;
use crate::{Result, UiError, View};
use dunit_render::ScrollArea;
use dunit_style::Style;
use dunit_ui::{Layout, Rect, Tree};
impl Renderer {
    pub(super) fn layout_collection_editors<M>(
        &self,
        tree: &Tree,
        layout: &mut Layout,
        styles: &[Style],
        views: &[&View<M>],
        areas: &[Option<ScrollArea>],
        measure: dunit_ui::layout::Measurer,
    ) -> Result<()> {
        for node in tree.iter() {
            let v = views[node.id.0 as usize];
            let Some(state) = v.rename.as_ref().and_then(|r| r.state.as_ref()) else {
                continue;
            };
            let list = v.list.as_ref().ok_or(UiError::InvalidRename)?;
            let index = list
                .items
                .iter()
                .position(|r| r.key == state.row)
                .ok_or(UiError::InvalidRename)?;
            let viewport = areas[node.id.0 as usize]
                .as_ref()
                .ok_or(UiError::InvalidLayout)?
                .viewport;
            let row = styles[node.id.0 as usize]
                .number("list-row-height")
                .ok_or(UiError::InvalidLayout)?;
            let mut rect = Rect {
                y: viewport.y + index as f32 * row - self.scroll_offset(&v.key).unwrap_or(0.),
                h: row,
                ..viewport
            };
            if let Some(t) = &v.table {
                let c = state
                    .column
                    .as_deref()
                    .and_then(|c| t.model.column(c))
                    .ok_or(UiError::InvalidRename)?;
                let widths = t
                    .model
                    .widths(
                        styles[node.id.0 as usize]
                            .number("table-column-width")
                            .ok_or(UiError::InvalidLayout)?,
                    )
                    .ok_or(UiError::InvalidLayout)?;
                rect = dunit_widgets::table::column_rect(
                    rect,
                    &widths,
                    self.scroll_offset_x(&v.key).unwrap_or(0.),
                    c,
                )
                .ok_or(UiError::InvalidLayout)?;
            } else if let Some(t) = &v.tree {
                let offset = t.rows[index].depth as f32
                    * styles[node.id.0 as usize]
                        .number("tree-indent")
                        .unwrap_or(0.)
                    + styles[node.id.0 as usize]
                        .number("tree-toggle-width")
                        .unwrap_or(0.);
                rect.x += offset.min(rect.w);
                rect.w = (rect.w - offset).max(0.);
            }
            let right = (rect.x + rect.w).min(viewport.x + viewport.w);
            let bottom = (rect.y + rect.h).min(viewport.y + viewport.h);
            rect.x = rect.x.max(viewport.x);
            rect.y = rect.y.max(viewport.y);
            rect.w = (right - rect.x).max(0.);
            rect.h = (bottom - rect.y).max(0.);
            let editor = node
                .children
                .iter()
                .find(|&&id| {
                    views[id.0 as usize]
                        .input
                        .as_ref()
                        .is_some_and(|i| i.owner.as_deref() == Some(&v.key))
                })
                .copied()
                .ok_or(UiError::InvalidRename)?;
            if !layout.relayout_subtree(tree, editor, rect, measure) {
                return Err(UiError::InvalidLayout);
            }
        }
        Ok(())
    }
}
