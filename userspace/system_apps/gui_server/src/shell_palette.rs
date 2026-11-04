//! Typed palette resolved from the shared SDK token registry.
use super::*;
use alloc::string::String;

pub fn palette(tokens: &dunit_style::Tokens) -> Result<Theme, String> {
    let color = |name: &str| {
        let c = tokens
            .color(name)
            .ok_or_else(|| alloc::format!("missing/color required: ${}", name))?;
        Ok::<u32, String>((c.a as u32) << 24 | (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32)
    };
    Ok(Theme {
        desktop: color("shell-desktop")?,
        title_focused: color("shell-title-focused")?,
        title_unfocused: color("shell-title-unfocused")?,
        border_focused: color("shell-border-focused")?,
        border_unfocused: color("shell-border-unfocused")?,
        close: color("shell-close")?,
        minimize: color("shell-minimize")?,
        maximize: color("shell-maximize")?,
        panel: color("shell-panel")?,
        panel_text: color("shell-panel-text")?,
        launcher: color("shell-launcher")?,
        taskbtn: color("shell-taskbtn")?,
        taskbtn_focused: color("shell-taskbtn-focused")?,
        menu: color("shell-menu")?,
        menu_hover: color("shell-menu-hover")?,
        title_text_focused: color("shell-title-text-focused")?,
        title_text_unfocused: color("shell-title-text-unfocused")?,
    })
}
