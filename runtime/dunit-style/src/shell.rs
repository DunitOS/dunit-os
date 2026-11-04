//! Validated shared shell paint and geometry values. No runtime fallbacks.
use crate::{Result, StyleError, Tokens};
use alloc::format;

fn invalid(name: &str) -> StyleError {
    StyleError::Unexpected {
        line: 1,
        what: format!("invalid shell token ${}", name),
    }
}
fn integer(tokens: &Tokens, name: &str, min: i32, max: i32) -> Result<i32> {
    let n = tokens.number(name).ok_or_else(|| invalid(name))?;
    if !n.is_finite() || n < min as f32 || n > max as f32 || n != n as i32 as f32 {
        return Err(invalid(name));
    }
    Ok(n as i32)
}
fn color(tokens: &Tokens, name: &str) -> Result<u32> {
    let c = tokens.color(name).ok_or_else(|| invalid(name))?;
    if c.a != 255 {
        return Err(invalid(name));
    }
    Ok((c.a as u32) << 24 | (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32)
}

macro_rules! visuals {
    ($($field:ident: $token:literal, $min:expr, $max:expr;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Visuals {
            $(pub $field: i32,)*
            pub cursor_outline: u32,
            pub cursor_fill: u32,
            pub toggle_knob: u32,
        }
        impl Visuals {
            pub fn from_tokens(tokens: &Tokens) -> Result<Self> {
                let value = Self {
                    $($field: integer(tokens, $token, $min, $max)?,)*
                    cursor_outline: color(tokens, "shell-cursor-outline")?,
                    cursor_fill: color(tokens, "shell-cursor-fill")?,
                    toggle_knob: color(tokens, "shell-toggle-knob")?,
                };
                value.validate()?;
                cursor_source(tokens)?;
                Ok(value)
            }
        }
    };
}
visuals! {
    layout_title_h: "shell-layout-title-h", 0, 16384;
    layout_border: "shell-layout-border", 0, 16384;
    layout_panel_h: "shell-layout-panel-h", 0, 16384;
    layout_launcher_w: "shell-layout-launcher-w", 0, 16384;
    layout_taskbtn_w: "shell-layout-taskbtn-w", 0, 16384;
    layout_taskbtn_gap: "shell-layout-taskbtn-gap", 0, 16384;
    layout_menu_w: "shell-layout-menu-w", 0, 16384;
    layout_menu_item_h: "shell-layout-menu-item-h", 0, 16384;
    layout_dock_w: "shell-layout-dock-w", 24, 256;
    layout_dock_padding: "shell-layout-dock-padding", 0, 10;
    layout_dock_gap: "shell-layout-dock-gap", 0, 64;
    layout_dock_margin: "shell-layout-dock-margin", 0, 128;
    layout_dock_tile_alpha: "shell-layout-dock-tile-alpha", 0, 255;
    layout_ws_w: "shell-layout-ws-w", 0, 16384;
    layout_title_font_px: "shell-layout-title-font-px", 6, 64;
    layout_panel_font_px: "shell-layout-panel-font-px", 6, 64;
    effects_corner_radius: "shell-effects-corner-radius", 0, 64;
    effects_shadow: "shell-effects-shadow", 0, 32;
    effects_shadow_alpha: "shell-effects-shadow-alpha", 0, 255;
    effects_blur_radius: "shell-effects-blur-radius", 0, 16;
    effects_blur_iters: "shell-effects-blur-iters", 1, 4;
    effects_panel_alpha: "shell-effects-panel-alpha", 0, 255;
    effects_menu_alpha: "shell-effects-menu-alpha", 0, 255;
    effects_anim_ms: "shell-effects-anim-ms", 0, 2000;
    hover_alpha: "shell-hover-alpha", 0, 255;
    hover_radius: "shell-hover-radius", 0, 1024;
    hover_inset_x: "shell-hover-inset-x", 0, 1024;
    hover_inset_y: "shell-hover-inset-y", 0, 1024;
    menu_text_inset: "shell-menu-text-inset", 0, 1024;
    menu_icon_inset: "shell-menu-icon-inset", 0, 1024;
    menu_icon_padding: "shell-menu-icon-padding", 0, 1024;
    menu_icon_min: "shell-menu-icon-min", 0, 1024;
    menu_icon_max: "shell-menu-icon-max", 0, 1024;
    overlay_gradient_top: "shell-overlay-gradient-top", -255, 255;
    desktop_gradient_top: "shell-desktop-gradient-top", -255, 255;
    desktop_gradient_bottom: "shell-desktop-gradient-bottom", -255, 255;
    switcher_tile: "shell-switcher-tile", 1, 1024;
    switcher_gap: "shell-switcher-gap", 0, 1024;
    switcher_padding: "shell-switcher-padding", 0, 1024;
    switcher_label_height: "shell-switcher-label-height", 1, 1024;
    switcher_radius: "shell-switcher-radius", 0, 1024;
    switcher_shadow: "shell-switcher-shadow", 0, 1024;
    switcher_shadow_alpha: "shell-switcher-shadow-alpha", 0, 255;
    switcher_alpha: "shell-switcher-alpha", 0, 255;
    switcher_highlight_inset: "shell-switcher-highlight-inset", 0, 1024;
    switcher_highlight_radius: "shell-switcher-highlight-radius", 0, 1024;
    switcher_icon_inset: "shell-switcher-icon-inset", 0, 1024;
    switcher_text_inset: "shell-switcher-text-inset", 0, 1024;
    switcher_font_size: "shell-switcher-font-size", 1, 1024;
    switcher_baseline_inset: "shell-switcher-baseline-inset", 0, 1024;
    qs_width: "shell-qs-width", 1, 1024;
    qs_padding: "shell-qs-padding", 0, 1024;
    qs_radius: "shell-qs-radius", 0, 1024;
    qs_shadow: "shell-qs-shadow", 0, 1024;
    qs_shadow_alpha: "shell-qs-shadow-alpha", 0, 255;
    qs_hover_inset: "shell-qs-hover-inset", 0, 1024;
    qs_label_inset: "shell-qs-label-inset", 0, 1024;
    qs_baseline_offset: "shell-qs-baseline-offset", 0, 1024;
    qs_toggle_width: "shell-qs-toggle-width", 1, 1024;
    qs_toggle_height: "shell-qs-toggle-height", 1, 1024;
    qs_toggle_margin: "shell-qs-toggle-margin", 0, 1024;
    qs_knob_inset: "shell-qs-knob-inset", 0, 1024;
    notification_margin: "shell-notification-margin", 0, 1024;
    notification_height: "shell-notification-height", 1, 1024;
    notification_gap: "shell-notification-gap", 0, 1024;
    notification_padding: "shell-notification-padding", 0, 1024;
    notification_min_width: "shell-notification-min-width", 1, 1024;
    notification_slide: "shell-notification-slide", 0, 1024;
    notification_radius: "shell-notification-radius", 0, 1024;
    notification_shadow: "shell-notification-shadow", 0, 1024;
    notification_shadow_alpha: "shell-notification-shadow-alpha", 0, 255;
    notification_accent_inset: "shell-notification-accent-inset", 0, 1024;
    notification_accent_width: "shell-notification-accent-width", 1, 1024;
    notification_accent_radius: "shell-notification-accent-radius", 0, 1024;
    notification_text_inset: "shell-notification-text-inset", 0, 1024;
    notification_baseline_offset: "shell-notification-baseline-offset", 0, 1024;
    widget_width: "shell-widget-width", 1, 1024;
    widget_padding: "shell-widget-padding", 0, 1024;
    widget_clock_gap: "shell-widget-clock-gap", 0, 1024;
    widget_monitor_height: "shell-widget-monitor-height", 1, 1024;
    widget_margin: "shell-widget-margin", 0, 1024;
    widget_gradient_top: "shell-widget-gradient-top", -255, 255;
    widget_gradient_bottom: "shell-widget-gradient-bottom", -255, 255;
    widget_clock_baseline_inset: "shell-widget-clock-baseline-inset", 0, 1024;
    widget_bar_gap: "shell-widget-bar-gap", 0, 1024;
    widget_bar_height: "shell-widget-bar-height", 1, 1024;
    widget_bar_radius: "shell-widget-bar-radius", 0, 1024;
    widget_bar_shade: "shell-widget-bar-shade", -255, 255;
    group_list_width: "shell-group-list-width", 1, 1024;
    group_row_height: "shell-group-row-height", 1, 1024;
    group_padding: "shell-group-padding", 0, 1024;
    group_fan_size: "shell-group-fan-size", 1, 1024;
    group_fan_step_x: "shell-group-fan-step-x", 0, 1024;
    group_fan_step_y: "shell-group-fan-step-y", 0, 1024;
    group_grid_cell: "shell-group-grid-cell", 1, 1024;
    group_columns: "shell-group-columns", 1, 64;
}
impl Visuals {
    fn validate(&self) -> Result<()> {
        if self.qs_toggle_width < self.qs_toggle_height
            || self.qs_toggle_height <= 2 * self.qs_knob_inset
        {
            return Err(invalid("shell-qs-knob-inset"));
        }
        if self.qs_width <= 2 * self.qs_padding
            || self.switcher_tile <= 2 * self.switcher_icon_inset
        {
            return Err(invalid("shell-qs-width/shell-switcher-tile"));
        }
        if self.widget_width <= 2 * self.widget_padding
            || self.notification_height <= 2 * self.notification_accent_inset
        {
            return Err(invalid(
                "shell-widget-padding/shell-notification-accent-inset",
            ));
        }
        if self.menu_icon_min > self.menu_icon_max {
            return Err(invalid("shell-menu-icon-min"));
        }
        Ok(())
    }
    pub fn cursor_colors(&self) -> [u32; 2] {
        [self.cursor_outline, self.cursor_fill]
    }
    pub fn qs_size(&self, row_height: i32, rows: i32) -> (i32, i32) {
        (self.qs_width, rows * row_height + 2 * self.qs_padding)
    }
    pub fn qs_row(
        &self,
        flyout: (i32, i32, i32, i32),
        row_height: i32,
        index: i32,
    ) -> (i32, i32, i32, i32) {
        (
            flyout.0 + self.qs_padding,
            flyout.1 + self.qs_padding + index * row_height,
            flyout.2 - 2 * self.qs_padding,
            row_height,
        )
    }
}

/// The compositor reads this asset at a configuration boundary. SDK clients
/// validate the same selected path without loading a compositor resource.
pub fn cursor_source(tokens: &Tokens) -> Result<alloc::string::String> {
    let value = tokens
        .get("shell-cursor-source")
        .ok_or_else(|| invalid("shell-cursor-source"))?;
    let path = value
        .as_keyword()
        .ok_or_else(|| invalid("shell-cursor-source"))?;
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.len() > 4096
        || path.chars().any(|c| c.is_control())
        || path
            .split('/')
            .skip(1)
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(invalid("shell-cursor-source"));
    }
    Ok(path.into())
}
