//! Typed shell tree sizing, arrangement and clipping at scene boundaries.
use super::*;
fn intrinsic(
    c: &settings::shell::ComponentConfig,
    horizontal: bool,
    apps: &Applications,
) -> (i32, i32) {
    let count = match c.kind {
        Kind::Workspaces => apps.workspaces,
        Kind::Apps => c.entries.len(),
        _ => 1,
    }
    .max(1) as i32;
    let font_px = if c.font_px == 0 { 14 } else { c.font_px as i32 };
    let main = match c.kind {
        Kind::Workspaces | Kind::Apps => count * c.size + (count - 1) * c.gap,
        Kind::Clock => (c.format.len() as i32 * font_px * 3 / 5 + 16).max(c.size),
        Kind::SystemMonitor => c.size.max(160),
        Kind::Tray => c.size.max(210),
        Kind::Title => c.size.max(160),
        Kind::Launcher | Kind::Button | Kind::Indicator | Kind::QuickSettings => {
            let label = if !c.label.is_empty() {
                c.label.as_str()
            } else if c.kind == Kind::Launcher {
                "Dunit"
            } else if c.kind == Kind::QuickSettings {
                "Settings"
            } else {
                ""
            };
            if c.icon.is_empty() {
                (label.chars().count() as i32 * font_px * 3 / 5 + 24).max(c.size)
            } else {
                c.size
            }
        }
        _ => c.size,
    };
    let (w, h) = if horizontal {
        (main, c.size)
    } else {
        (c.size, main)
    };
    (
        if c.width > 0 { c.width } else { w },
        if c.height > 0 { c.height } else { h },
    )
}
fn natural(
    config: &ShellConfig,
    id: &str,
    horizontal: bool,
    apps: &Applications,
) -> Option<(i32, i32)> {
    let c = config.component(id).filter(|c| c.enabled)?;
    if c.kind != Kind::Container {
        return Some(intrinsic(c, horizontal, apps));
    }
    let row = c.direction == Direction::Horizontal;
    let sizes: Vec<_> = c
        .children
        .iter()
        .filter_map(|id| natural(config, id, row, apps))
        .collect();
    let main = sizes
        .iter()
        .map(|&(w, h)| if row { w } else { h })
        .sum::<i32>()
        + c.gap * sizes.len().saturating_sub(1) as i32;
    let cross = sizes
        .iter()
        .map(|&(w, h)| if row { h } else { w })
        .max()
        .unwrap_or(0);
    let (w, h) = if row { (main, cross) } else { (cross, main) };
    Some((
        if c.width > 0 { c.width } else { w },
        if c.height > 0 { c.height } else { h },
    ))
}

fn append(
    builder: &mut TreeBuilder,
    config: &ShellConfig,
    id: &str,
    parent: NodeId,
    horizontal: bool,
    apps: &Applications,
    mapping: &mut Vec<(NodeId, usize, bool)>,
) {
    let Some(index) = config
        .components
        .iter()
        .position(|c| c.id == id && c.enabled)
    else {
        return;
    };
    let c = &config.components[index];
    let (w, h) = intrinsic(c, horizontal, apps);
    let container = c.kind == Kind::Container;
    let mut a = Attrs::default();
    a.width = if container && c.width == 0 {
        Sizing::Auto
    } else {
        Sizing::Px(w as f32)
    };
    a.height = if container && c.height == 0 {
        Sizing::Auto
    } else {
        Sizing::Px(h as f32)
    };
    if horizontal && c.height == 0 {
        a.height = Sizing::Pct(1.0);
    }
    if !horizontal && c.width == 0 {
        a.width = Sizing::Pct(1.0);
    }
    a.grow = c.grow as f32;
    a.spacing = c.gap as f32;
    a.main_align = ui_align(c.align);
    a.cross_align = dunit_ui::Align::Center;
    let child_horizontal = c.direction == Direction::Horizontal;
    let kind = if container {
        if child_horizontal {
            UiKind::Row
        } else {
            UiKind::Column
        }
    } else {
        UiKind::Element("Component".into())
    };
    let node = builder.push(Some(c.id.clone()), kind, a, None, Some(parent));
    builder.add_child(parent, node);
    mapping.push((node, index, horizontal));
    for child in &c.children {
        append(
            builder,
            config,
            child,
            node,
            child_horizontal,
            apps,
            mapping,
        );
    }
}

pub(super) fn layout_panels(
    config: &ShellConfig,
    apps: &Applications,
    bw: usize,
    bh: usize,
    cascade: &Cascade,
) -> Result<(Vec<Panel>, Insets), String> {
    let mut effective = config.clone();
    for panel in &mut effective.panels {
        let s = style(cascade, "Panel", &panel.id, &panel.class, None);
        panel.size = number(&s, "size", panel.size).max(8);
        panel.length = number(&s, "length", panel.length);
        panel.margin = number(&s, "margin", panel.margin);
        panel.padding = number(&s, "padding", panel.padding);
        panel.gap = number(&s, "gap", panel.gap);
    }
    for component in &mut effective.components {
        let s = style(cascade, "Component", &component.id, &component.class, None);
        component.size = number(&s, "size", component.size);
        component.width = number(&s, "width", component.width);
        component.height = number(&s, "height", component.height);
        component.gap = number(&s, "gap", component.gap);
        if let Some(px) = s.number("font-size") {
            component.font_px = px.clamp(6.0, 128.0) as u32;
        }
    }
    let config = &effective;
    let mut panels = Vec::new();
    let mut occupied = Insets {
        top: 0,
        bottom: 0,
        left: 0,
        right: 0,
    };
    let mut insets = occupied;
    for (index, p) in config.panels.iter().enumerate().filter(|(_, p)| p.enabled) {
        let horizontal = p.edge.is_horizontal();
        let available = if horizontal {
            bw as i32 - occupied.left - occupied.right
        } else {
            bh as i32 - occupied.top - occupied.bottom
        };
        let mut builder = TreeBuilder::new();
        let mut a = Attrs::default();
        a.padding = Edges::all(p.padding as f32);
        a.spacing = p.gap as f32;
        a.cross_align = dunit_ui::Align::Center;
        let root = builder.push(
            None,
            if horizontal {
                UiKind::Row
            } else {
                UiKind::Column
            },
            a,
            None,
            None,
        );
        let mut mapping = Vec::new();
        for id in &p.children {
            append(
                &mut builder,
                &config,
                id,
                root,
                horizontal,
                apps,
                &mut mapping,
            );
        }
        let tree = builder.finish(root);
        let sizes: Vec<_> = p
            .children
            .iter()
            .filter_map(|id| natural(&config, id, horizontal, apps))
            .collect();
        let natural = sizes
            .iter()
            .map(|&(w, h)| if horizontal { w } else { h })
            .sum::<i32>()
            + p.padding * 2
            + p.gap * sizes.len().saturating_sub(1) as i32;
        let main_margin = p.main_margin.unwrap_or(p.margin);
        let length = if p.length == 0 { natural } else { p.length }
            .clamp(1, (available - main_margin * 2).max(1));
        let cross_available = if horizontal {
            bh as i32 - occupied.top - occupied.bottom
        } else {
            bw as i32 - occupied.left - occupied.right
        };
        if available <= main_margin * 2 || cross_available <= p.margin {
            return Err("panels exceed the display bounds".into());
        }
        let thickness = p.size.min(cross_available - p.margin);
        let offset = match p.align {
            Align::Start => main_margin,
            Align::Center => (available - length) / 2,
            Align::End => available - length - main_margin,
        };
        let rect = match p.edge {
            Edge::Top => (
                occupied.left + offset,
                occupied.top + p.margin,
                length,
                thickness,
            ),
            Edge::Bottom => (
                occupied.left + offset,
                bh as i32 - occupied.bottom - p.margin - thickness,
                length,
                thickness,
            ),
            Edge::Left => (
                occupied.left + p.margin,
                occupied.top + offset,
                thickness,
                length,
            ),
            Edge::Right => (
                bw as i32 - occupied.right - p.margin - thickness,
                occupied.top + offset,
                thickness,
                length,
            ),
        };
        let layout = dunit_ui::layout(&tree, rect.2 as f32, rect.3 as f32);
        let cells = mapping
            .iter()
            .map(|&(node, component, horizontal)| {
                let r = layout.rect(node);
                let c = &config.components[component];
                // Clip through every ancestor, so nested content cannot
                // paint or receive input outside its container.
                let mut clipped = geometry::intersect(
                    (
                        rect.0 + r.x as i32,
                        rect.1 + r.y as i32,
                        r.w as i32,
                        r.h as i32,
                    ),
                    rect,
                );
                let mut parent = tree.node(node).parent;
                while let Some(id) = parent {
                    let pr = layout.rect(id);
                    clipped = geometry::intersect(
                        clipped,
                        (
                            rect.0 + pr.x as i32,
                            rect.1 + pr.y as i32,
                            pr.w as i32,
                            pr.h as i32,
                        ),
                    );
                    parent = tree.node(id).parent;
                }
                Cell {
                    base_rect: (0, 0, 0, 0),
                    parent: tree.node(node).parent.and_then(|node| {
                        mapping
                            .iter()
                            .find(|(candidate, _, _)| *candidate == node)
                            .map(|(_, index, _)| *index)
                    }),
                    component,
                    horizontal,
                    gap: c.gap,
                    rect: clipped,
                    transitions: [
                        style(&cascade, "Component", &c.id, &c.class, None).transitions(),
                        style(&cascade, "Component", &c.id, &c.class, Some("hover")).transitions(),
                        style(&cascade, "Component", &c.id, &c.class, Some("focus")).transitions(),
                    ],
                    normal: style(&cascade, "Component", &c.id, &c.class, None),
                    hover: style(&cascade, "Component", &c.id, &c.class, Some("hover")),
                    focused: style(&cascade, "Component", &c.id, &c.class, Some("focus")),
                }
            })
            .collect();
        panels.push(Panel {
            base_rect: rect,
            opacity: 255,
            index,
            rect,
            cells,
            style: style(&cascade, "Panel", &p.id, &p.class, None),
        });
        add_edge(&mut occupied, p.edge, thickness + p.margin);
        if p.reserve && !p.autohide {
            match p.edge {
                Edge::Top => insets.top = occupied.top,
                Edge::Bottom => insets.bottom = occupied.bottom,
                Edge::Left => insets.left = occupied.left,
                Edge::Right => insets.right = occupied.right,
            }
        }
    }
    if insets.left + insets.right >= bw as i32 || insets.top + insets.bottom >= bh as i32 {
        return Err("panels consume the entire work area".into());
    }
    for panel in &mut panels {
        for cell in &mut panel.cells {
            cell.base_rect = cell.rect;
        }
    }
    Ok((panels, insets))
}
