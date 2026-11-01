use super::Renderer;
use crate::{Result, SplitAxis, SplitGeometry, UiError, View};
use dunit_style::Style;
use dunit_ui::{layout::Measurer, Layout, NodeId, Rect, Tree};
impl Renderer {
    pub fn splitter_geometry(&self, key: &str) -> Option<SplitGeometry> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        let parent = p.tree.node(id).parent?;
        let binding = p.splitters[id.0 as usize]?;
        Some(geometry(
            &p.tree, &p.layout, &p.styles, parent, binding.0, binding.1,
        ))
    }
    pub(super) fn layout_splitters<M>(
        &self,
        tree: &Tree,
        layout: &mut Layout,
        styles: &[Style],
        views: &[&View<M>],
        measure: Measurer<'_>,
    ) -> Result<()> {
        for node in tree
            .iter()
            .filter(|n| views[n.id.0 as usize].dialog.is_some())
        {
            if let Some(parent) = node.parent.filter(|p| {
                matches!(
                    views[p.0 as usize].workspace,
                    Some(crate::workspace::WorkspaceRole::ModalHost)
                )
            }) {
                let outer = layout.rect(parent);
                let pad = tree.node(parent).attrs.padding;
                let r = Rect {
                    x: outer.x + pad.left,
                    y: outer.y + pad.top,
                    w: (outer.w - pad.horizontal()).max(0.),
                    h: (outer.h - pad.vertical()).max(0.),
                };
                let measured = dunit_ui::layout::measure_subtree(tree, node.id, r.w, r.h, measure)
                    .ok_or(UiError::InvalidLayout)?;
                let width = measured.0.min(r.w);
                let height = measured.1.min(r.h);
                let rect = Rect {
                    x: r.x + (r.w - width) / 2.,
                    y: r.y + (r.h - height) / 2.,
                    w: width,
                    h: height,
                };
                if !layout.relayout_subtree(tree, node.id, rect, measure) {
                    return Err(UiError::InvalidLayout);
                }
            }
        }
        // Keep dialog actions reachable when the client is smaller than its
        // preferred body height; the body viewport absorbs the reduction.
        for node in tree
            .iter()
            .filter(|n| views[n.id.0 as usize].dialog.is_some())
        {
            let r = layout.rect(node.id);
            let p = node.attrs.padding;
            let inner = Rect {
                x: r.x + p.left,
                y: r.y + p.top,
                w: (r.w - p.horizontal()).max(0.),
                h: (r.h - p.vertical()).max(0.),
            };
            let title = node.children[0];
            let body = node.children[1];
            let actions = node.children[2];
            let ah = layout.rect(actions).h.min(inner.h);
            let th = layout.rect(title).h.min((inner.h - ah).max(0.));
            let bh = layout.rect(body).h.min((inner.h - ah - th).max(0.));
            for (id, rect) in [
                (title, Rect { h: th, ..inner }),
                (
                    body,
                    Rect {
                        y: inner.y + th,
                        h: bh,
                        ..inner
                    },
                ),
                (
                    actions,
                    Rect {
                        y: inner.y + th + bh,
                        h: ah,
                        ..inner
                    },
                ),
            ] {
                if !layout.relayout_subtree(tree, id, rect, measure) {
                    return Err(UiError::InvalidLayout);
                }
            }
        }
        // Parents must be arranged before nested splitters in either pane.
        for node in tree.iter() {
            let Some(handle) = node.children.get(1).copied() else {
                continue;
            };
            let Some(split) = &views[handle.0 as usize].split else {
                continue;
            };
            if node.children.len() != 3 {
                return Err(UiError::InvalidWorkspace);
            }
            let g = geometry(
                tree,
                layout,
                styles,
                node.id,
                split.value,
                split.options.axis,
            );
            for (child, rect) in node.children.iter().zip([g.first, g.handle, g.second]) {
                if !layout.relayout_subtree(tree, *child, rect, measure) {
                    return Err(UiError::InvalidLayout);
                }
            }
        }
        Ok(())
    }
}
fn geometry(
    tree: &Tree,
    layout: &Layout,
    styles: &[Style],
    parent: NodeId,
    value: i64,
    axis: SplitAxis,
) -> SplitGeometry {
    let root = tree.node(parent);
    let r = layout.rect(parent);
    let pad = root.attrs.padding;
    let area = Rect {
        x: r.x + pad.left,
        y: r.y + pad.top,
        w: (r.w - pad.horizontal()).max(0.),
        h: (r.h - pad.vertical()).max(0.),
    };
    let a = &tree.node(root.children[0]).attrs;
    let b = &tree.node(root.children[2]).attrs;
    let vertical = axis == SplitAxis::Vertical;
    SplitGeometry::new(
        area,
        styles[root.children[1].0 as usize]
            .number("splitter-thickness")
            .unwrap_or(0.),
        vertical,
        value,
        if vertical { a.min_h } else { a.min_w }.unwrap_or(0.),
        if vertical { a.max_h } else { a.max_w },
        if vertical { b.min_h } else { b.min_w }.unwrap_or(0.),
        if vertical { b.max_h } else { b.max_w },
    )
}
