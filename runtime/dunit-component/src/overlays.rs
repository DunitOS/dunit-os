//! Overlay scenes reuse the normal tree painter and font/layout mechanisms.
use super::Renderer;
use crate::popup::{OverlayState, PopupItem};
use crate::{Attrs, Edges, PopupHit, Rect, Sizing, View};
use alloc::{format, string::String, vec, vec::Vec};
use dunit_style::{cascade::NodeStyle, Style};
use dunit_ui::{layout_measured, tree::TreeBuilder, Kind, Layout, NodeId, Tree};
use dunit_widgets::{
    intrinsic_size,
    overlay::{place, Placement},
    FontMeasure, Widget,
};
#[derive(Debug, Clone, PartialEq)]
struct Row {
    path: Vec<usize>,
    label: String,
    rect: Rect,
    style: Style,
    separator: bool,
    enabled: bool,
}
#[derive(Debug, Clone)]
pub(super) struct Panel {
    pub owner: String,
    pub prefix: Vec<usize>,
    pub rect: Rect,
    pub first: usize,
    rows: Vec<Row>,
    style: Style,
    tree: Tree,
    layout: Layout,
    styles: Vec<Style>,
    tooltip: bool,
    text: Option<String>,
}
impl PartialEq for Panel {
    fn eq(&self, b: &Self) -> bool {
        self.owner == b.owner
            && self.prefix == b.prefix
            && self.rect == b.rect
            && self.rows == b.rows
            && self.style == b.style
            && self.tooltip == b.tooltip
            && self.text == b.text
    }
}
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Scene {
    pub panels: Vec<Panel>,
}
fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}
fn padding(style: &Style) -> Edges {
    let [t, r, b, l] = style.padding().unwrap_or([0.; 4]);
    let border = style.number("border-width").unwrap_or(0.).max(0.);
    Edges {
        top: t + border,
        right: r + border,
        bottom: b + border,
        left: l + border,
    }
}
fn label(item: &PopupItem, style: &Style) -> String {
    if item.submenu {
        format!(
            "{} {}",
            item.label,
            style.keyword("menu-submenu-mark").unwrap_or("")
        )
    } else {
        item.label.clone()
    }
}
impl Scene {
    pub fn hit(&self, x: f32, y: f32) -> Option<PopupHit> {
        for p in self.panels.iter().rev().filter(|p| !p.tooltip) {
            if contains(p.rect, x, y) {
                return Some(p.rows.iter().find(|row| contains(row.rect, x, y)).map_or(
                    PopupHit::Panel,
                    |row| PopupHit::Item {
                        owner: p.owner.clone(),
                        path: row.path.clone(),
                    },
                ));
            }
        }
        None
    }
    pub fn paint(
        &self,
        font: &crate::Font,
        surface: &mut dunit_render::Surface,
        clip: Rect,
    ) -> dunit_render::PaintResult {
        for panel in &self.panels {
            dunit_render::paint_clipped(
                &panel.tree,
                &panel.layout,
                &|id| panel.styles[id.0 as usize].clone(),
                font,
                surface,
                clip,
            )?;
        }
        Ok(())
    }
    pub fn requires_backdrop_rebuild(&self) -> bool {
        self.panels.iter().any(|p| {
            p.styles
                .iter()
                .any(|s| s.number("backdrop-blur").unwrap_or(0.) > 0.)
        })
    }
    pub fn bounds(&self) -> impl Iterator<Item = Rect> + '_ {
        self.panels
            .iter()
            .map(|p| dunit_render::visual_bounds(p.rect, &p.style))
    }
}
impl Renderer {
    pub fn popup_hit(&self, x: f32, y: f32) -> Option<PopupHit> {
        self.previous.as_ref()?.overlays.hit(x, y)
    }
    pub fn popup_item_rect(&self, owner: &str, path: &[usize]) -> Option<Rect> {
        self.previous
            .as_ref()?
            .overlays
            .panels
            .iter()
            .filter(|p| p.owner == owner && !p.tooltip)
            .flat_map(|p| &p.rows)
            .find(|r| r.path == path)
            .map(|r| r.rect)
    }
    pub fn widget_rect(&self, key: &str) -> Option<Rect> {
        let p = self.previous.as_ref()?;
        Some(p.layout.rect(p.tree.by_name(key)?))
    }
    pub fn tooltip_delay_ms(&self, key: &str) -> Option<u64> {
        let p = self.previous.as_ref()?;
        let id = p.tree.by_name(key)?;
        p.styles[id.0 as usize]
            .get("tooltip-delay")?
            .as_duration()
            .filter(|v| v.is_finite() && *v >= 0.)
            .map(|v| v as u64)
    }
    pub(super) fn overlays<M>(
        &self,
        view: &View<M>,
        tree: &Tree,
        layout: &Layout,
        state: Option<&OverlayState>,
        areas: &[Option<dunit_render::ScrollArea>],
        width: u32,
        height: u32,
    ) -> Scene {
        let mut scene = Scene::default();
        let Some(state) = state else {
            return scene;
        };
        let viewport = Rect {
            x: 0.,
            y: 0.,
            w: width as f32,
            h: height as f32,
        };
        if let Some(open) = &state.open {
            if let Some(owner) = view.find(&open.owner) {
                if let Some(id) = tree.by_name(&owner.key) {
                    let anchor = layout.rect(id);
                    let mut prefix = Vec::new();
                    let mut anchor = if let Some((x, y)) = open.point {
                        Rect { x, y, w: 0., h: 0. }
                    } else {
                        anchor
                    };
                    for depth in 0..=open.path.len() {
                        let Some(items) = owner.popup_items(open.context, &prefix) else {
                            break;
                        };
                        let active = open.path.get(depth).copied();
                        let panel = self.menu_panel(
                            owner,
                            &prefix,
                            &items,
                            active,
                            open.pressed.as_deref(),
                            anchor,
                            viewport,
                            if depth > 0 {
                                Placement::Side
                            } else if open.point.is_some() {
                                Placement::Point
                            } else {
                                Placement::Below
                            },
                        );
                        let next = active
                            .and_then(|i| panel.rows.iter().find(|row| row.path.last() == Some(&i)))
                            .map(|r| r.rect);
                        scene.panels.push(panel);
                        let Some(index) = active else {
                            break;
                        };
                        if !items.get(index).is_some_and(|i| i.submenu)
                            || depth + 1 >= open.path.len()
                        {
                            break;
                        }
                        let Some(next) = next else {
                            break;
                        };
                        prefix.push(index);
                        anchor = next;
                    }
                }
            }
        } else if state.tooltip.visible {
            if let Some(owner) = state
                .tooltip
                .owner
                .as_deref()
                .and_then(|key| view.find(key))
            {
                if let Some((id, text)) = tree.by_name(&owner.key).zip(owner.tooltip.as_deref()) {
                    let anchor = layout.rect(id);
                    let mut clip = anchor;
                    let mut parent = tree.node(id).parent;
                    while let Some(id) = parent {
                        let r = areas[id.0 as usize]
                            .as_ref()
                            .map_or(layout.rect(id), |a| a.viewport);
                        let x = clip.x.max(r.x);
                        let y = clip.y.max(r.y);
                        let right = (clip.x + clip.w).min(r.x + r.w);
                        let bottom = (clip.y + clip.h).min(r.y + r.h);
                        clip = Rect {
                            x,
                            y,
                            w: (right - x).max(0.),
                            h: (bottom - y).max(0.),
                        };
                        parent = tree.node(id).parent;
                    }
                    if clip.w > 0.
                        && clip.h > 0.
                        && clip.x < viewport.w
                        && clip.y < viewport.h
                        && clip.x + clip.w > 0.
                        && clip.y + clip.h > 0.
                    {
                        scene
                            .panels
                            .push(self.tooltip_panel(owner, text, anchor, viewport));
                    }
                }
            }
        }
        scene
    }
    fn resolve_overlay<M>(
        &self,
        owner: &View<M>,
        tag: &str,
        key: Option<&str>,
        states: &[&str],
    ) -> Style {
        let classes: Vec<_> = owner.classes.iter().map(|s| s.as_str()).collect();
        self.cascade.resolve(&NodeStyle {
            element: tag,
            id: key,
            classes: &classes,
            states,
        })
    }
    fn menu_panel<M>(
        &self,
        owner: &View<M>,
        prefix: &[usize],
        items: &[PopupItem],
        active: Option<usize>,
        pressed: Option<&[usize]>,
        anchor: Rect,
        viewport: Rect,
        placement: Placement,
    ) -> Panel {
        let style = self.resolve_overlay(owner, "Popup", Some(&owner.key), &[]);
        let pad = padding(&style);
        let fm = FontMeasure { font: &self.font };
        let mut row_styles = Vec::new();
        let mut labels = Vec::new();
        let mut heights = Vec::new();
        let mut width =
            style
                .number("menu-min-width")
                .unwrap_or(0.)
                .max(if placement == Placement::Below {
                    anchor.w
                } else {
                    0.
                });
        for (i, item) in items.iter().enumerate() {
            let mut states = Vec::new();
            if !item.enabled {
                states.push("disabled");
            }
            if item.selected {
                states.push("checked");
            }
            if active == Some(i) {
                states.push("focus");
                states.push("hover");
            }
            let mut path = prefix.to_vec();
            path.push(i);
            if pressed == Some(path.as_slice()) {
                states.push("active");
            }
            let tag = if item.separator {
                "MenuSeparator"
            } else {
                "MenuItem"
            };
            let row_style = self.resolve_overlay(owner, tag, Some(&item.key), &states);
            let text = label(item, &row_style);
            let (w, h) = intrinsic_size(Widget::Button, Some(&text), &row_style, &fm);
            let h = if item.separator {
                row_style.number("menu-separator-height").unwrap_or(0.)
            } else {
                h.max(row_style.number("menu-row-height").unwrap_or(0.))
            };
            width = width.max(w + pad.left + pad.right);
            labels.push(text);
            heights.push(h.max(0.));
            row_styles.push(row_style);
        }
        let available = (viewport.h - pad.top - pad.bottom).max(0.);
        let previous = self
            .previous
            .as_ref()
            .and_then(|p| {
                p.overlays
                    .panels
                    .iter()
                    .find(|p| p.owner == owner.key && p.prefix == prefix && !p.tooltip)
            })
            .map_or(0, |p| p.first);
        let mut first = previous.min(items.len().saturating_sub(1));
        if active.is_some_and(|i| i < first) {
            first = active.unwrap();
        }
        if let Some(index) = active.filter(|i| *i < items.len()) {
            while first < index && heights[first..=index].iter().sum::<f32>() > available {
                first += 1;
            }
        }
        let mut end = first;
        let mut total = 0.;
        while end < items.len() && (end == first || total + heights[end] <= available) {
            total += heights[end];
            end += 1;
        }
        let rect = place(
            anchor,
            width,
            total + pad.top + pad.bottom,
            viewport,
            placement,
            style.number("menu-gap").unwrap_or(0.),
        );
        let mut builder = TreeBuilder::new();
        let root = builder.push(
            None,
            Kind::Column,
            Attrs {
                padding: pad,
                ..Default::default()
            },
            None,
            None,
        );
        let mut styles = vec![style.clone()];
        let mut rows = Vec::new();
        for i in first..end {
            let mut path = prefix.to_vec();
            path.push(i);
            let id = builder.push(
                None,
                Kind::Element(
                    if items[i].separator {
                        "MenuSeparator"
                    } else {
                        "MenuItem"
                    }
                    .into(),
                ),
                Attrs {
                    width: Sizing::Grow(1.),
                    height: Sizing::Px(heights[i]),
                    ..Default::default()
                },
                Some(labels[i].clone()),
                Some(root),
            );
            builder.add_child(root, id);
            styles.push(row_styles[i].clone());
            rows.push(Row {
                path,
                label: labels[i].clone(),
                rect: Rect::default(),
                style: row_styles[i].clone(),
                separator: items[i].separator,
                enabled: items[i].enabled,
            });
        }
        let tree = builder.finish(root);
        let mut layout = layout_measured(&tree, rect.w, rect.h, &|_| (0., 0.));
        layout.translate_subtree(&tree, root, rect.x, rect.y);
        for (i, row) in rows.iter_mut().enumerate() {
            row.rect = layout.rect(NodeId((i + 1) as u32));
        }
        Panel {
            owner: owner.key.clone(),
            prefix: prefix.to_vec(),
            rect,
            first,
            rows,
            style,
            tree,
            layout,
            styles,
            tooltip: false,
            text: None,
        }
    }
    fn tooltip_panel<M>(&self, owner: &View<M>, text: &str, anchor: Rect, viewport: Rect) -> Panel {
        let style = self.resolve_overlay(owner, "Tooltip", Some(&owner.key), &[]);
        let text_style = self.resolve_overlay(owner, "TooltipText", None, &[]);
        let pad = padding(&style);
        let px = text_style.number("font-size").unwrap_or(0.);
        let line_height = self.font.line_metrics(px).line_height();
        let max_width = style
            .number("tooltip-max-width")
            .unwrap_or(viewport.w)
            .min(viewport.w);
        let available = (max_width - pad.left - pad.right).max(0.);
        let mut lines: Vec<String> = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            let candidate = if line.is_empty() {
                String::from(word)
            } else {
                format!("{} {}", line, word)
            };
            if self.font.layout_line(&candidate, px).1 <= available {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(core::mem::take(&mut line));
            }
            for ch in word.chars() {
                let mut next = line.clone();
                next.push(ch);
                if !line.is_empty() && self.font.layout_line(&next, px).1 > available {
                    lines.push(core::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
        if !line.is_empty() || lines.is_empty() {
            lines.push(line);
        }
        let max_lines = if line_height > 0. {
            ((viewport.h - pad.top - pad.bottom).max(0.) / line_height) as usize
        } else {
            0
        };
        lines.truncate(max_lines.max(1));
        let width = lines
            .iter()
            .map(|s| self.font.layout_line(s, px).1)
            .fold(0., f32::max)
            + pad.left
            + pad.right;
        let rect = place(
            anchor,
            width,
            (lines.len() as f32) * line_height + pad.top + pad.bottom,
            viewport,
            Placement::Below,
            style.number("tooltip-gap").unwrap_or(0.),
        );
        let mut builder = TreeBuilder::new();
        let root = builder.push(
            None,
            Kind::Column,
            Attrs {
                padding: pad,
                ..Default::default()
            },
            None,
            None,
        );
        let mut styles = vec![style.clone()];
        for line in &lines {
            let id = builder.push(
                None,
                Kind::Element("TooltipText".into()),
                Attrs {
                    height: Sizing::Px(line_height),
                    ..Default::default()
                },
                Some(line.clone()),
                Some(root),
            );
            builder.add_child(root, id);
            styles.push(text_style.clone());
        }
        let tree = builder.finish(root);
        let mut layout = layout_measured(&tree, rect.w, rect.h, &|_| (0., 0.));
        layout.translate_subtree(&tree, root, rect.x, rect.y);
        Panel {
            owner: owner.key.clone(),
            prefix: Vec::new(),
            rect,
            first: 0,
            rows: Vec::new(),
            style,
            tree,
            layout,
            styles,
            tooltip: true,
            text: Some(text.into()),
        }
    }
}
