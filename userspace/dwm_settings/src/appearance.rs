//! Compatibility at the configuration boundary, not a second shell palette.
use crate::{Config, Theme};
use alloc::string::String;

pub const SHELL_TOKENS: &str = "/assets/dwm/sdk/shell.dss";
pub const SDK_BASE: &str = "/assets/dwm/sdk/base.dss";

/// Both SDK clients and the compositor parse this same token registry.
/// Explicit legacy fields retain precedence; omitted fields inherit DSS.
pub fn profile(source: &str, config: &Config) -> String {
    alloc::format!("{}\n{}", source, config.appearance)
}

pub(crate) fn overrides(document: &str, theme: &Theme) -> String {
    let mut output = String::new();
    let mut section = "";
    for raw in document.lines() {
        let line = crate::strip_comment(raw).trim();
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim();
        } else if section == "theme" {
            if let Some((field, _)) = line.split_once('=') {
                let field = field.trim();
                let color = match field {
                    "desktop" => theme.desktop,
                    "title_focused" => theme.title_focused,
                    "title_unfocused" => theme.title_unfocused,
                    "border_focused" => theme.border_focused,
                    "border_unfocused" => theme.border_unfocused,
                    "close" => theme.close,
                    "minimize" => theme.minimize,
                    "maximize" => theme.maximize,
                    "panel" => theme.panel,
                    "panel_text" => theme.panel_text,
                    "launcher" => theme.launcher,
                    "taskbtn" => theme.taskbtn,
                    "taskbtn_focused" => theme.taskbtn_focused,
                    "menu" => theme.menu,
                    "menu_hover" => theme.menu_hover,
                    "title_text_focused" => theme.title_text_focused,
                    "title_text_unfocused" => theme.title_text_unfocused,
                    _ => continue, // validated by the desktop document parser
                };
                // TOML colors are ARGB; DSS colors are RGBA.
                output.push_str(&alloc::format!(
                    "$shell-{}: #{:06x}{:02x};\n",
                    field.replace('_', "-"),
                    color & 0x00ff_ffff,
                    color >> 24
                ));
            }
        }
    }
    output
}
