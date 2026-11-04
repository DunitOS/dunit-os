//! Convert shared validated DSS metrics to the existing compositor geometry.
use super::*;
use dunit_style::shell::Visuals;

pub fn layout(base: &Layout, skin: &Visuals) -> Layout {
    Layout {
        title_h: skin.layout_title_h,
        border: skin.layout_border,
        panel_h: skin.layout_panel_h,
        launcher_w: skin.layout_launcher_w,
        taskbtn_w: skin.layout_taskbtn_w,
        taskbtn_gap: skin.layout_taskbtn_gap,
        menu_w: skin.layout_menu_w,
        menu_item_h: skin.layout_menu_item_h,
        dock_w: skin.layout_dock_w,
        dock_padding: skin.layout_dock_padding,
        dock_gap: skin.layout_dock_gap,
        dock_margin: skin.layout_dock_margin,
        dock_tile_alpha: skin.layout_dock_tile_alpha as u32,
        ws_w: skin.layout_ws_w,
        title_font_px: skin.layout_title_font_px,
        panel_font_px: skin.layout_panel_font_px,
        ..*base
    }
}

pub fn effects(base: &Effects, skin: &Visuals) -> Effects {
    Effects {
        corner_radius: skin.effects_corner_radius,
        shadow: skin.effects_shadow,
        shadow_alpha: skin.effects_shadow_alpha,
        blur_radius: skin.effects_blur_radius,
        blur_iters: skin.effects_blur_iters,
        panel_alpha: skin.effects_panel_alpha,
        menu_alpha: skin.effects_menu_alpha,
        anim_ms: skin.effects_anim_ms,
        ..*base
    }
}
