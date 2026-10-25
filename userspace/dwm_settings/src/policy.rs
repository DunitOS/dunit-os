//! Checked keyboard actions and ordered, partial window rules.
use crate::{validation, Applications, ConfigError, MAX_WS};
use alloc::{format, string::String, vec::Vec};

pub const MAX_BINDINGS: usize = 128;
pub const MAX_RULES: usize = 128;
#[derive(Clone, PartialEq, Eq)]
pub enum Action {
    Launcher,
    Close,
    Maximize,
    Minimize,
    Retile,
    Workspace(usize),
    Launch(String),
}
/// Parse the same action language for panel buttons and global key bindings.
/// Workspace numbers in documents are one-based; resolved indices are zero-based.
pub fn action(value: &str, apps: &Applications) -> Result<Action, &'static str> {
    match value {
        "launcher" => Ok(Action::Launcher),
        "close" => Ok(Action::Close),
        "maximize" => Ok(Action::Maximize),
        "minimize" => Ok(Action::Minimize),
        "retile" => Ok(Action::Retile),
        _ => {
            if let Some(id) = value.strip_prefix("launch:") {
                if apps.apps.iter().any(|app| app.id == id) {
                    return Ok(Action::Launch(id.into()));
                }
                return Err("unknown application reference");
            }
            if let Some(number) = value.strip_prefix("workspace:") {
                return Ok(Action::Workspace(workspace(number, apps)?));
            }
            Err("unknown action")
        }
    }
}
#[derive(Clone)]
pub struct Binding {
    pub id: String,
    pub scancode: u8,
    pub modifiers: u8,
    pub action: Action,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub struct WindowPolicy {
    pub workspace: Option<usize>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub floating: Option<bool>,
    pub opacity: Option<u8>,
    pub animation: Option<String>,
}
#[derive(Clone)]
pub struct WindowRule {
    pub id: String,
    pub app: String,
    pub policy: WindowPolicy,
}
#[derive(Clone, Default)]
pub struct PolicyConfig {
    pub bindings: Vec<Binding>,
    pub rules: Vec<WindowRule>,
}
impl PolicyConfig {
    /// Later matching rules override only supplied fields. User interaction can
    /// subsequently override these initial window defaults in the compositor.
    pub fn for_app(&self, app: &str) -> WindowPolicy {
        let mut result = WindowPolicy::default();
        for rule in &self.rules {
            if rule.app != app && rule.app != "*" {
                continue;
            }
            let p = &rule.policy;
            macro_rules! merge { ($($field:ident),*) => { $(if p.$field.is_some() { result.$field = p.$field.clone(); })* }; }
            merge!(workspace, x, y, width, height, floating, opacity, animation);
        }
        result
    }
}
fn workspace(value: &str, apps: &Applications) -> Result<usize, &'static str> {
    let number = validation::uint(value, MAX_WS as u32)? as usize;
    if number == 0 || number > apps.workspaces {
        return Err("workspace outside configured count");
    }
    Ok(number - 1)
}
fn modifiers(value: &str) -> Result<u8, &'static str> {
    if value.is_empty() || value == "none" {
        return Ok(0);
    }
    let mut mask = 0;
    for part in value.split('+') {
        let bit = match part {
            "shift" => libdunit::KEYMOD_SHIFT,
            "ctrl" => libdunit::KEYMOD_CTRL,
            "alt" => libdunit::KEYMOD_ALT,
            "super" => libdunit::KEYMOD_SUPER,
            _ => return Err("unknown key modifier"),
        };
        if mask & bit != 0 {
            return Err("duplicate key modifier");
        }
        mask |= bit;
    }
    Ok(mask)
}
pub(crate) fn section(section: &str) -> bool {
    section.starts_with("bindings.") || section.starts_with("rules.")
}
pub fn resolve(text: &str, apps: &Applications) -> Result<PolicyConfig, ConfigError> {
    let mut result = PolicyConfig::default();
    let mut current = "";
    let mut sections = Vec::new();
    let mut fields = Vec::new();
    let mut binding_ready = Vec::new();
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
            if !section(current) {
                continue;
            }
            let (_, id) = current.split_once('.').unwrap();
            if id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(error(current.into(), "invalid policy id"));
            }
            if sections.contains(&current) {
                return Err(error(current.into(), "duplicate section"));
            }
            sections.push(current);
            if current.starts_with("bindings.") {
                if result.bindings.len() == MAX_BINDINGS {
                    return Err(error(current.into(), "too many bindings"));
                }
                result.bindings.push(Binding {
                    id: id.into(),
                    scancode: 0,
                    modifiers: 0,
                    action: Action::Launcher,
                });
                binding_ready.push((false, false));
            } else {
                if result.rules.len() == MAX_RULES {
                    return Err(error(current.into(), "too many rules"));
                }
                result.rules.push(WindowRule {
                    id: id.into(),
                    app: String::new(),
                    policy: WindowPolicy::default(),
                });
            }
            continue;
        }
        if !section(current) {
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
        (|| {
            if current.starts_with("bindings.") {
                let binding = result.bindings.last_mut().unwrap();
                match key {
                    "scancode" => {
                        binding.scancode = validation::uint(value, 127)? as u8;
                        if binding.scancode == 0 {
                            return Err("scancode must not be zero");
                        }
                        binding_ready.last_mut().unwrap().0 = true;
                    }
                    "modifiers" => {
                        binding.modifiers = modifiers(validation::config_string(value)?)?
                    }
                    "action" => {
                        binding.action = action(validation::config_string(value)?, apps)?;
                        binding_ready.last_mut().unwrap().1 = true;
                    }
                    _ => return Err("unknown binding field"),
                }
            } else {
                let rule = result.rules.last_mut().unwrap();
                match key {
                    "app" => {
                        let id = validation::config_string(value)?;
                        if id != "*" && !apps.apps.iter().any(|app| app.id == id) {
                            return Err("unknown application reference");
                        }
                        rule.app = id.into();
                    }
                    "workspace" => rule.policy.workspace = Some(workspace(value, apps)?),
                    "x" | "y" => {
                        let n = value
                            .parse::<i32>()
                            .map_err(|_| "expected signed coordinate")?;
                        if !(-16384..=16384).contains(&n) {
                            return Err("coordinate outside accepted range");
                        }
                        if key == "x" {
                            rule.policy.x = Some(n);
                        } else {
                            rule.policy.y = Some(n);
                        }
                    }
                    "width" | "height" => {
                        let n = validation::uint(value, 16384)? as i32;
                        if n < 1 {
                            return Err("window dimension must be positive");
                        }
                        if key == "width" {
                            rule.policy.width = Some(n);
                        } else {
                            rule.policy.height = Some(n);
                        }
                    }
                    "floating" => {
                        rule.policy.floating =
                            Some(crate::parse_bool(value).ok_or("expected a boolean")?)
                    }
                    "opacity" => rule.policy.opacity = Some(validation::uint(value, 255)? as u8),
                    "animation" => {
                        let name = validation::config_string(value)?;
                        if !matches!(
                            name,
                            "none" | "fade" | "slide" | "scale" | "spring" | "minimize" | "genie"
                        ) {
                            return Err("unknown animation");
                        }
                        rule.policy.animation = Some(name.into());
                    }
                    _ => return Err("unknown rule field"),
                }
            }
            Ok(())
        })()
        .map_err(|reason| error(field, reason))?;
    }
    for (index, binding) in result.bindings.iter().enumerate() {
        if binding_ready[index] != (true, true) {
            return Err(ConfigError {
                line: 0,
                field: format!("bindings.{}", binding.id),
                reason: "binding requires scancode and action",
            });
        }
        if result.bindings[..index]
            .iter()
            .any(|b| b.scancode == binding.scancode && b.modifiers == binding.modifiers)
        {
            return Err(ConfigError {
                line: 0,
                field: format!("bindings.{}", binding.id),
                reason: "duplicate keyboard chord",
            });
        }
    }
    for rule in &result.rules {
        if rule.app.is_empty() {
            return Err(ConfigError {
                line: 0,
                field: format!("rules.{}", rule.id),
                reason: "rule requires an application selector",
            });
        }
    }
    Ok(result)
}
