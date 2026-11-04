//! Compatibility at the configuration boundary, not a second shell palette.
use crate::{Config, Settings};
use alloc::string::String;

pub const SHELL_TOKENS: &str = "/assets/dwm/sdk/shell.dss";
pub const SDK_BASE: &str = "/assets/dwm/sdk/base.dss";

/// Both SDK clients and the compositor parse this same token registry.
/// Explicit legacy fields retain precedence; omitted fields inherit DSS.
pub fn profile(source: &str, config: &Config) -> String {
    alloc::format!("{}\n{}", source, config.appearance)
}

pub(crate) fn overrides(document: &str, settings: &Settings) -> String {
    let mut output = String::new();
    let mut section = "";
    for raw in document.lines() {
        let line = crate::strip_comment(raw).trim();
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim();
        } else if section == "layout" || section == "effects" {
            if let Some((field, _)) = line.split_once('=') {
                let field = field.trim();
                let number = match (section, field) {
                    ("layout", "title_h") => settings.layout.title_h as i32,
                    ("layout", "border") => settings.layout.border as i32,
                    ("layout", "panel_h") => settings.layout.panel_h as i32,
                    ("layout", "launcher_w") => settings.layout.launcher_w as i32,
                    ("layout", "taskbtn_w") => settings.layout.taskbtn_w as i32,
                    ("layout", "taskbtn_gap") => settings.layout.taskbtn_gap as i32,
                    ("layout", "menu_w") => settings.layout.menu_w as i32,
                    ("layout", "menu_item_h") => settings.layout.menu_item_h as i32,
                    ("layout", "dock_w") => settings.layout.dock_w as i32,
                    ("layout", "dock_padding") => settings.layout.dock_padding as i32,
                    ("layout", "dock_gap") => settings.layout.dock_gap as i32,
                    ("layout", "dock_margin") => settings.layout.dock_margin as i32,
                    ("layout", "dock_tile_alpha") => settings.layout.dock_tile_alpha as i32,
                    ("layout", "ws_w") => settings.layout.ws_w as i32,
                    ("layout", "title_font_px") => settings.layout.title_font_px as i32,
                    ("layout", "panel_font_px") => settings.layout.panel_font_px as i32,
                    ("effects", "corner_radius") => settings.effects.corner_radius as i32,
                    ("effects", "shadow") => settings.effects.shadow as i32,
                    ("effects", "shadow_alpha") => settings.effects.shadow_alpha as i32,
                    ("effects", "blur_radius") => settings.effects.blur_radius as i32,
                    ("effects", "blur_iters") => settings.effects.blur_iters as i32,
                    ("effects", "panel_alpha") => settings.effects.panel_alpha as i32,
                    ("effects", "menu_alpha") => settings.effects.menu_alpha as i32,
                    ("effects", "anim_ms") => settings.effects.anim_ms as i32,
                    _ => continue,
                };
                output.push_str(&alloc::format!(
                    "$shell-{}-{}: {};\n",
                    section,
                    field.replace('_', "-"),
                    number
                ));
            }
        } else if section == "theme" {
            if let Some((field, _)) = line.split_once('=') {
                let field = field.trim();
                let color = match field {
                    "desktop" => settings.theme.desktop,
                    "title_focused" => settings.theme.title_focused,
                    "title_unfocused" => settings.theme.title_unfocused,
                    "border_focused" => settings.theme.border_focused,
                    "border_unfocused" => settings.theme.border_unfocused,
                    "close" => settings.theme.close,
                    "minimize" => settings.theme.minimize,
                    "maximize" => settings.theme.maximize,
                    "panel" => settings.theme.panel,
                    "panel_text" => settings.theme.panel_text,
                    "launcher" => settings.theme.launcher,
                    "taskbtn" => settings.theme.taskbtn,
                    "taskbtn_focused" => settings.theme.taskbtn_focused,
                    "menu" => settings.theme.menu,
                    "menu_hover" => settings.theme.menu_hover,
                    "title_text_focused" => settings.theme.title_text_focused,
                    "title_text_unfocused" => settings.theme.title_text_unfocused,
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
