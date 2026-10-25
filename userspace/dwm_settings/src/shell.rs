//! Declarative shell composition. Parsing and reference checks finish before
//! the compositor replaces a live tree; legacy strips resolve to the same model.
use crate::{validation, Applications, ConfigError, Edge, Settings};
use alloc::{format, string::String, vec, vec::Vec};

pub const MAX_PANELS: usize = 64;
pub const MAX_COMPONENTS: usize = 512;
pub const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}
impl Align {
    fn parse(s: &str) -> Result<Self, &'static str> {
        match s {
            "start" => Ok(Self::Start),
            "center" => Ok(Self::Center),
            "end" => Ok(Self::End),
            _ => Err("expected start, center or end"),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AppsSource {
    Pinned,
    Running,
    Both,
}
impl AppsSource {
    pub fn resolve_entries(self, pinned: &[usize], running: &[usize]) -> Vec<usize> {
        let mut out = Vec::new();
        if self != Self::Running {
            for &app in pinned {
                if !out.contains(&app) {
                    out.push(app);
                }
            }
        }
        if self != Self::Pinned {
            for &app in running {
                if !out.contains(&app) {
                    out.push(app);
                }
            }
        }
        out
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    Launcher,
    QuickSettings,
    Workspaces,
    Apps,
    AppGroup,
    Clock,
    AnalogClock,
    Tray,
    Title,
    SystemMonitor,
    Button,
    Indicator,
    Spacer,
    Container,
}
impl ComponentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Launcher => "launcher",
            Self::QuickSettings => "quicksettings",
            Self::Workspaces => "workspaces",
            Self::Apps => "apps",
            Self::AppGroup => "app_group",
            Self::Clock => "clock",
            Self::AnalogClock => "analog_clock",
            Self::Tray => "tray",
            Self::Title => "title",
            Self::SystemMonitor => "system_monitor",
            Self::Button => "button",
            Self::Indicator => "indicator",
            Self::Spacer => "spacer",
            Self::Container => "container",
        }
    }
    fn parse(s: &str) -> Result<Self, &'static str> {
        match s {
            "launcher" => Ok(Self::Launcher),
            "quicksettings" => Ok(Self::QuickSettings),
            "workspaces" => Ok(Self::Workspaces),
            "apps" => Ok(Self::Apps),
            "app_group" => Ok(Self::AppGroup),
            "clock" => Ok(Self::Clock),
            "analog_clock" => Ok(Self::AnalogClock),
            "tray" => Ok(Self::Tray),
            "title" => Ok(Self::Title),
            "system_monitor" => Ok(Self::SystemMonitor),
            "button" => Ok(Self::Button),
            "indicator" => Ok(Self::Indicator),
            "spacer" => Ok(Self::Spacer),
            "container" => Ok(Self::Container),
            _ => Err("unknown component kind"),
        }
    }
}
#[derive(Clone)]
pub struct PanelConfig {
    pub id: String,
    pub edge: Edge,
    pub align: Align,
    /// Thickness and optional length in pixels; length zero fits the content.
    pub size: i32,
    pub length: i32,
    pub margin: i32,
    pub main_margin: Option<i32>,
    pub padding: i32,
    pub gap: i32,
    pub reserve: bool,
    pub autohide: bool,
    pub enabled: bool,
    pub class: String,
    pub children: Vec<String>,
}
impl PanelConfig {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            edge: Edge::Top,
            align: Align::Start,
            size: 32,
            length: 0,
            margin: 0,
            main_margin: None,
            padding: 6,
            gap: 6,
            reserve: true,
            autohide: false,
            enabled: true,
            class: String::new(),
            children: Vec::new(),
        }
    }
}
#[derive(Clone)]
pub struct ComponentConfig {
    pub id: String,
    pub kind: ComponentKind,
    pub source: AppsSource,
    pub direction: Direction,
    pub font_px: u32,
    pub color: Option<u32>,
    pub interval_ms: u32,
    pub children: Vec<String>,
    /// Registry IDs, not positional indices, survive registry reordering.
    pub entries: Vec<String>,
    pub align: Align,
    pub size: i32,
    pub width: i32,
    pub height: i32,
    pub grow: u32,
    pub gap: i32,
    pub enabled: bool,
    pub format: String,
    pub label: String,
    pub icon: String,
    pub action: String,
    pub class: String,
    /// App-group presentation: grid, list or fan.
    pub presentation: String,
}
impl ComponentConfig {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            kind: ComponentKind::Spacer,
            source: AppsSource::Pinned,
            direction: Direction::Horizontal,
            font_px: 0,
            color: None,
            interval_ms: 1000,
            children: Vec::new(),
            entries: Vec::new(),
            align: Align::Start,
            size: 32,
            width: 0,
            height: 0,
            grow: 0,
            gap: 6,
            enabled: true,
            format: "%H:%M".into(),
            label: String::new(),
            icon: String::new(),
            action: String::new(),
            class: String::new(),
            presentation: "grid".into(),
        }
    }
}
#[derive(Clone, Default)]
pub struct ShellConfig {
    pub panels: Vec<PanelConfig>,
    pub components: Vec<ComponentConfig>,
    /// Explicit trees replace the legacy panel/dock/tray composition entirely.
    pub explicit: bool,
    pub stylesheet: String,
}
impl ShellConfig {
    /// Stage widget data as a new tree, preserving the original on any error.
    pub fn with_widget_updates<'a>(
        &self,
        updates: impl IntoIterator<Item = (u32, &'a str, Option<u32>)>,
    ) -> Result<Self, &'static str> {
        let mut candidate = self.clone();
        for (target, text, color) in updates {
            let index = target
                .checked_sub(1)
                .ok_or("widget target must not be zero")? as usize;
            let component = candidate
                .components
                .get_mut(index)
                .ok_or("unknown widget target")?;
            if !matches!(
                component.kind,
                ComponentKind::Indicator | ComponentKind::Button
            ) {
                return Err("target does not accept widget data");
            }
            if text.len() > 256 || text.chars().any(char::is_control) {
                return Err("widget text exceeds limit or contains controls");
            }
            component.label = text.into();
            if let Some(color) = color {
                component.color = Some(color);
            }
        }
        Ok(candidate)
    }
    pub fn component(&self, id: &str) -> Option<&ComponentConfig> {
        self.components.iter().find(|component| component.id == id)
    }
}
fn identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
pub(crate) fn section(name: &str) -> bool {
    name.starts_with("panels.") || name.starts_with("components.")
}
fn strings(value: &str) -> Result<Vec<String>, &'static str> {
    validation::list(value)?
        .into_iter()
        .map(|id| {
            if identifier(id) {
                Ok(id.into())
            } else {
                Err("invalid reference id")
            }
        })
        .collect()
}
fn boolean(value: &str) -> Result<bool, &'static str> {
    crate::parse_bool(value).ok_or("expected a boolean")
}
fn dimension(value: &str) -> Result<i32, &'static str> {
    Ok(validation::uint(value, 16384)? as i32)
}
fn issue(field: String, reason: &'static str) -> ConfigError {
    ConfigError {
        line: 0,
        field,
        reason,
    }
}

/// Resolve a checked document, or report an invalid tree without touching live state.
/// This entry point also performs structural parsing checks for direct callers.
pub fn resolve(
    text: &str,
    settings: &Settings,
    apps: &Applications,
) -> Result<ShellConfig, ConfigError> {
    let mut shell = ShellConfig::default();
    let mut current = "";
    let mut seen = Vec::new();
    let mut kinds = Vec::new();
    let mut fields = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = crate::strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let error = |field: String, reason| ConfigError {
            line: index + 1,
            field,
            reason,
        };
        if let Some(header) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current = header.trim();
            if current == "shell" {
                continue;
            }
            if !section(current) {
                continue;
            }
            shell.explicit = true;
            let (prefix, id) = current.split_once('.').unwrap();
            if !identifier(id) {
                return Err(error(current.into(), "invalid shell id"));
            }
            if seen.contains(&current) {
                return Err(error(current.into(), "duplicate section"));
            }
            seen.push(current);
            if prefix == "panels" {
                if shell.panels.len() == MAX_PANELS {
                    return Err(error(current.into(), "too many panels"));
                }
                shell.panels.push(PanelConfig::new(id));
            } else {
                if shell.components.len() == MAX_COMPONENTS {
                    return Err(error(current.into(), "too many components"));
                }
                shell.components.push(ComponentConfig::new(id));
            }
            continue;
        }
        if !section(current) && current != "shell" {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| error(current.into(), "expected key = value"))?;
        let (key, value) = (key.trim(), value.trim());
        let field = format!("{}.{}", current, key);
        if fields.contains(&field) {
            return Err(error(field, "duplicate key"));
        }
        fields.push(field.clone());
        let result = (|| {
            if current == "shell" {
                if key != "stylesheet" {
                    return Err("unknown shell field");
                }
                let path = validation::config_string(value)?;
                if !path.is_empty()
                    && (!path.starts_with('/') || path.split('/').any(|s| s == ".."))
                {
                    return Err("stylesheet requires an absolute path without traversal");
                }
                shell.stylesheet = path.into();
            } else if current.starts_with("panels.") {
                let panel = shell.panels.last_mut().unwrap();
                match key {
                    "edge" => {
                        panel.edge = Edge::parse(validation::config_string(value)?)
                            .ok_or("invalid panel edge")?
                    }
                    "align" => panel.align = Align::parse(validation::config_string(value)?)?,
                    "size" => {
                        panel.size = dimension(value)?;
                        if panel.size < 8 {
                            return Err("panel size must be at least 8");
                        }
                    }
                    "length" => panel.length = dimension(value)?,
                    "margin" => panel.margin = dimension(value)?,
                    "main_margin" => panel.main_margin = Some(dimension(value)?),
                    "padding" => panel.padding = dimension(value)?,
                    "gap" => panel.gap = dimension(value)?,
                    "reserve" => panel.reserve = boolean(value)?,
                    "autohide" => panel.autohide = boolean(value)?,
                    "enabled" => panel.enabled = boolean(value)?,
                    "class" => panel.class = validation::config_string(value)?.into(),
                    "children" => panel.children = strings(value)?,
                    _ => return Err("unknown panel field"),
                }
            } else {
                let component = shell.components.last_mut().unwrap();
                match key {
                    "source" => {
                        component.source = match validation::config_string(value)? {
                            "pinned" => AppsSource::Pinned,
                            "running" => AppsSource::Running,
                            "both" => AppsSource::Both,
                            _ => return Err("expected pinned, running or both"),
                        }
                    }
                    "font_px" => component.font_px = validation::uint(value, 128)?,
                    "color" => component.color = Some(validation::color(value)?),
                    "interval_ms" => {
                        component.interval_ms = validation::uint(value, 3600000)?;
                        if component.interval_ms < 16 {
                            return Err("interval must be at least 16 ms");
                        }
                    }
                    "direction" => {
                        component.direction = match validation::config_string(value)? {
                            "horizontal" => Direction::Horizontal,
                            "vertical" => Direction::Vertical,
                            _ => return Err("expected horizontal or vertical"),
                        }
                    }
                    "kind" => {
                        component.kind = ComponentKind::parse(validation::config_string(value)?)?;
                        kinds.push(component.id.clone());
                    }
                    "children" => component.children = strings(value)?,
                    "entries" => component.entries = strings(value)?,
                    "align" => component.align = Align::parse(validation::config_string(value)?)?,
                    "size" => component.size = dimension(value)?,
                    "width" => component.width = dimension(value)?,
                    "height" => component.height = dimension(value)?,
                    "grow" => component.grow = validation::uint(value, 1024)?,
                    "gap" => component.gap = dimension(value)?,
                    "enabled" => component.enabled = boolean(value)?,
                    "format" => component.format = validation::config_string(value)?.into(),
                    "label" => component.label = validation::config_string(value)?.into(),
                    "icon" => {
                        let path = validation::config_string(value)?;
                        if !path.is_empty()
                            && (!path.starts_with('/')
                                || path.split('/').any(|s| s == "..")
                                || !(path.ends_with(".svg") || path.ends_with(".rgba")))
                        {
                            return Err("component icon requires an absolute SVG or RGBA path");
                        }
                        component.icon = path.into();
                    }
                    "action" => component.action = validation::config_string(value)?.into(),
                    "class" => component.class = validation::config_string(value)?.into(),
                    "presentation" => {
                        let s = validation::config_string(value)?;
                        if !matches!(s, "grid" | "list" | "fan") {
                            return Err("expected grid, list or fan");
                        }
                        component.presentation = s.into();
                    }
                    _ => return Err("unknown component field"),
                }
            }
            Ok(())
        })();
        result.map_err(|reason| error(field, reason))?;
    }
    if !shell.explicit {
        let mut migrated = legacy(settings, apps);
        migrated.stylesheet = shell.stylesheet;
        return Ok(migrated);
    }
    if shell.panels.is_empty() {
        return Err(issue("panels".into(), "components require a panel"));
    }
    let mut owners = Vec::new();
    for component in &shell.components {
        if !kinds.contains(&component.id) {
            return Err(issue(
                format!("components.{}.kind", component.id),
                "component requires a kind",
            ));
        }
        if component.kind != ComponentKind::Container && !component.children.is_empty() {
            return Err(issue(
                format!("components.{}.children", component.id),
                "only containers accept children",
            ));
        }
        if !matches!(
            component.kind,
            ComponentKind::Launcher | ComponentKind::Apps | ComponentKind::AppGroup
        ) && !component.entries.is_empty()
        {
            return Err(issue(
                format!("components.{}.entries", component.id),
                "component does not accept applications",
            ));
        }
        if component.source != AppsSource::Pinned && component.kind != ComponentKind::Apps {
            return Err(issue(
                format!("components.{}.source", component.id),
                "only apps components accept running sources",
            ));
        }
        if !component.icon.is_empty()
            && !matches!(
                component.kind,
                ComponentKind::Launcher
                    | ComponentKind::QuickSettings
                    | ComponentKind::Button
                    | ComponentKind::AppGroup
            )
        {
            return Err(issue(
                format!("components.{}.icon", component.id),
                "component does not accept a custom icon",
            ));
        }
        if !component.action.is_empty() {
            crate::policy::action(&component.action, apps)
                .map_err(|reason| issue(format!("components.{}.action", component.id), reason))?;
        }
        for id in &component.entries {
            if !apps.apps.iter().any(|app| app.id == *id) {
                return Err(issue(
                    format!("components.{}.entries", component.id),
                    "unknown application reference",
                ));
            }
        }
    }
    // Every component has exactly one parent, including disabled nodes. This
    // rules out shared mutable UI state and makes hit testing unambiguous.
    for (field, children) in shell
        .panels
        .iter()
        .map(|p| (format!("panels.{}.children", p.id), &p.children))
        .chain(
            shell
                .components
                .iter()
                .map(|c| (format!("components.{}.children", c.id), &c.children)),
        )
    {
        for id in children {
            if shell.component(id).is_none() {
                return Err(issue(field.clone(), "unknown component reference"));
            }
            if owners.contains(id) {
                return Err(issue(field.clone(), "component has multiple parents"));
            }
            owners.push(id.clone());
        }
    }
    let mut visited = Vec::new();
    fn visit(
        shell: &ShellConfig,
        id: &str,
        depth: usize,
        visited: &mut Vec<String>,
    ) -> Result<(), ConfigError> {
        if depth > MAX_DEPTH {
            return Err(issue(
                format!("components.{}", id),
                "component nesting exceeds limit",
            ));
        }
        if visited.iter().any(|s| s == id) {
            return Err(issue(format!("components.{}", id), "component cycle"));
        }
        visited.push(id.into());
        for child in &shell.component(id).unwrap().children {
            visit(shell, child, depth + 1, visited)?;
        }
        Ok(())
    }
    for panel in &shell.panels {
        for child in &panel.children {
            visit(&shell, child, 1, &mut visited)?;
        }
    }
    if visited.len() != shell.components.len() {
        return Err(issue("components".into(), "unreachable component or cycle"));
    }
    Ok(shell)
}

fn legacy(settings: &Settings, apps: &Applications) -> ShellConfig {
    let mut shell = ShellConfig::default();
    let mut add = |id: &str, kind| {
        shell.components.push(ComponentConfig {
            kind,
            ..ComponentConfig::new(id)
        });
    };
    add("legacy-launcher", ComponentKind::Launcher);
    if settings.layout.show_workspaces {
        add("legacy-workspaces", ComponentKind::Workspaces);
    }
    if settings.quicksettings.enabled {
        add("legacy-quicksettings-gap", ComponentKind::Spacer);
        add("legacy-quicksettings", ComponentKind::QuickSettings);
    }
    add("legacy-title", ComponentKind::Title);
    if settings.tray.pos == settings.panel.pos {
        add("legacy-tray", ComponentKind::Tray);
    }
    let mut panel = PanelConfig::new("legacy-panel");
    panel.edge = settings.panel.pos;
    panel.size = settings.layout.panel_h;
    panel.length = 16384; // clamped to the screen by layout
    panel.padding = 0;
    panel.gap = 0;
    panel.children = shell.components.iter().map(|c| c.id.clone()).collect();
    shell.panels.push(panel);
    shell.component_mut("legacy-title").grow = 1;
    if settings.layout.show_workspaces {
        let c = shell.component_mut("legacy-workspaces");
        c.size = (settings.layout.ws_w - 2).max(1);
        c.gap = 2;
        if settings.panel.pos.is_horizontal() {
            c.width = settings.layout.ws_w * apps.workspaces as i32;
        } else {
            c.height = settings.layout.ws_w * apps.workspaces as i32;
        }
    }
    if settings.quicksettings.enabled {
        let c = shell.component_mut("legacy-quicksettings-gap");
        c.size = 8;
        c.width = 8;
        let c = shell.component_mut("legacy-quicksettings");
        c.width = 28;
        c.size = settings.layout.panel_h;
    }
    for c in &mut shell.components {
        if matches!(c.kind, ComponentKind::Title | ComponentKind::Tray) {
            c.font_px = settings.layout.title_font_px as u32;
        }
    }
    shell.component_mut("legacy-launcher").width = settings.layout.launcher_w;
    if settings.tray.pos != settings.panel.pos {
        let mut tray = PanelConfig::new("legacy-tray-panel");
        tray.edge = settings.tray.pos;
        tray.size = settings.tray.size;
        tray.length = 16384;
        tray.children = vec!["legacy-tray".into()];
        shell.components.push(ComponentConfig {
            kind: ComponentKind::Tray,
            font_px: settings.layout.title_font_px as u32,
            ..ComponentConfig::new("legacy-tray")
        });
        shell.panels.push(tray);
    }
    if !apps.dock.is_empty() {
        let layout = settings.layout;
        let mut dock = PanelConfig::new("legacy-dock");
        dock.edge = layout.dock_edge;
        dock.size = layout.dock_w;
        dock.length = if layout.dock_centered { 0 } else { 16384 };
        dock.main_margin = Some(0);
        dock.align = if layout.dock_centered {
            Align::Center
        } else {
            Align::Start
        };
        dock.padding = layout.dock_padding;
        dock.margin = layout.dock_margin;
        dock.gap = layout.dock_gap;
        dock.children = vec!["legacy-apps".into()];
        shell.components.push(ComponentConfig {
            kind: ComponentKind::Apps,
            size: (layout.dock_w - 2 * layout.dock_padding).max(1),
            gap: layout.dock_gap,
            entries: apps.dock.iter().map(|i| apps.apps[*i].id.clone()).collect(),
            ..ComponentConfig::new("legacy-apps")
        });
        shell.panels.push(dock);
    }
    shell
}
impl ShellConfig {
    fn component_mut(&mut self, id: &str) -> &mut ComponentConfig {
        self.components
            .iter_mut()
            .find(|component| component.id == id)
            .unwrap()
    }
}
