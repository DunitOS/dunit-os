//! Canonical scalar document used by validation and checked edits.
use super::*;

/// Append a byte as two uppercase hex digits.
fn push_hex_byte(out: &mut String, v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push(HEX[((v >> 4) & 0xF) as usize] as char);
    out.push(HEX[(v & 0xF) as usize] as char);
}

/// Append a non-negative integer in decimal (no separators, no sign).
fn push_uint(out: &mut String, mut v: u64) {
    if v == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    for &b in &buf[i..] {
        out.push(b as char);
    }
}

/// Append `key = "#AARRGGBB"\n` for an ARGB8888 color (matches `parse_color`).
fn push_color(out: &mut String, key: &str, argb: u32) {
    out.push_str(key);
    out.push_str(" = \"#");
    push_hex_byte(out, (argb >> 24) & 0xFF);
    push_hex_byte(out, (argb >> 16) & 0xFF);
    push_hex_byte(out, (argb >> 8) & 0xFF);
    push_hex_byte(out, argb & 0xFF);
    out.push_str("\"\n");
}

/// Append `key = <n>\n`. Values are conceptually non-negative (`parse_uint`
/// cannot read a sign), so a stray negative is clamped to 0 to stay parseable.
pub(super) fn push_int(out: &mut String, key: &str, v: i64) {
    out.push_str(key);
    out.push_str(" = ");
    push_uint(out, v.max(0) as u64);
    out.push('\n');
}

/// Append `key = true\n` / `key = false\n`.
pub(super) fn push_bool(out: &mut String, key: &str, b: bool) {
    out.push_str(key);
    out.push_str(if b { " = true\n" } else { " = false\n" });
}

/// Append `key = "value"\n` for a string value (matches `unquote` on read).
fn push_string(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(" = \"");
    out.push_str(value);
    out.push_str("\"\n");
}

/// Serialize `Settings` back to the TOML dialect `parse_into` accepts, mirroring
/// `assets/dwm/default.toml`. Lossless for every recognised key: parsing the
/// output reproduces the same `theme`/`layout`/`effects` (see `roundtrip_ok`).
pub fn to_toml(s: &Settings) -> String {
    let mut out = String::new();
    out.push_str("# Dunit DWM settings (written by gui_settings; live in RAM this session).\n\n");

    out.push_str("[meta]\n");
    push_int(&mut out, "schema_version", s.meta.schema_version as i64);
    out.push('\n');

    out.push_str("[theme]\n");
    push_color(&mut out, "desktop", s.theme.desktop);
    push_color(&mut out, "title_focused", s.theme.title_focused);
    push_color(&mut out, "title_unfocused", s.theme.title_unfocused);
    push_color(&mut out, "border_focused", s.theme.border_focused);
    push_color(&mut out, "border_unfocused", s.theme.border_unfocused);
    push_color(&mut out, "close", s.theme.close);
    push_color(&mut out, "minimize", s.theme.minimize);
    push_color(&mut out, "maximize", s.theme.maximize);
    push_color(&mut out, "panel", s.theme.panel);
    push_color(&mut out, "panel_text", s.theme.panel_text);
    push_color(&mut out, "launcher", s.theme.launcher);
    push_color(&mut out, "taskbtn", s.theme.taskbtn);
    push_color(&mut out, "taskbtn_focused", s.theme.taskbtn_focused);
    push_color(&mut out, "menu", s.theme.menu);
    push_color(&mut out, "menu_hover", s.theme.menu_hover);
    push_color(&mut out, "title_text_focused", s.theme.title_text_focused);
    push_color(
        &mut out,
        "title_text_unfocused",
        s.theme.title_text_unfocused,
    );
    out.push('\n');

    out.push_str("[layout]\n");
    push_int(&mut out, "title_h", s.layout.title_h as i64);
    push_int(&mut out, "border", s.layout.border as i64);
    push_int(&mut out, "panel_h", s.layout.panel_h as i64);
    push_int(&mut out, "launcher_w", s.layout.launcher_w as i64);
    push_int(&mut out, "menu_w", s.layout.menu_w as i64);
    push_int(&mut out, "menu_item_h", s.layout.menu_item_h as i64);
    push_int(&mut out, "dock_w", s.layout.dock_w as i64);
    push_string(&mut out, "dock_edge", s.layout.dock_edge.as_str());
    push_bool(&mut out, "dock_centered", s.layout.dock_centered);
    push_int(&mut out, "dock_padding", s.layout.dock_padding as i64);
    push_int(&mut out, "dock_gap", s.layout.dock_gap as i64);
    push_int(&mut out, "dock_margin", s.layout.dock_margin as i64);
    push_int(&mut out, "dock_tile_alpha", s.layout.dock_tile_alpha as i64);
    push_bool(&mut out, "traffic_lights", s.layout.traffic_lights);
    push_bool(&mut out, "show_workspaces", s.layout.show_workspaces);
    push_int(&mut out, "ws_w", s.layout.ws_w as i64);
    push_int(&mut out, "max_windows", s.layout.max_windows as i64);
    push_int(&mut out, "title_font_px", s.layout.title_font_px as i64);
    push_int(&mut out, "panel_font_px", s.layout.panel_font_px as i64);
    out.push('\n');

    out.push_str("[effects]\n");
    push_int(&mut out, "corner_radius", s.effects.corner_radius as i64);
    push_int(&mut out, "shadow", s.effects.shadow as i64);
    push_int(&mut out, "shadow_alpha", s.effects.shadow_alpha as i64);
    push_bool(&mut out, "blur", s.effects.blur);
    push_int(&mut out, "blur_radius", s.effects.blur_radius as i64);
    push_int(&mut out, "blur_iters", s.effects.blur_iters as i64);
    push_int(&mut out, "panel_alpha", s.effects.panel_alpha as i64);
    push_int(&mut out, "menu_alpha", s.effects.menu_alpha as i64);
    push_bool(&mut out, "gradient", s.effects.gradient);
    push_bool(&mut out, "anim", s.effects.anim);
    push_int(&mut out, "anim_ms", s.effects.anim_ms as i64);
    out.push('\n');

    out.push_str("[desktop]\n");
    push_string(&mut out, "wallpaper", s.desktop.wallpaper.as_str());
    push_string(&mut out, "icon_theme", s.desktop.icon_theme.as_str());
    push_string(&mut out, "font", s.desktop.font.as_str());
    push_string(&mut out, "sdk_theme", s.desktop.sdk_theme.as_str());
    push_string(&mut out, "logo", s.desktop.logo.as_str());
    push_string(&mut out, "palette", s.desktop.palette.as_str());
    push_string(&mut out, "themes", s.desktop.themes.as_str());
    out.push('\n');

    out.push_str("[widgets]\n");
    push_bool(&mut out, "enabled", s.widgets.enabled);
    push_int(&mut out, "corner", s.widgets.corner as i64);
    out.push('\n');

    out.push_str("[display]\n");
    push_int(&mut out, "width", s.display.width as i64);
    push_int(&mut out, "height", s.display.height as i64);
    out.push('\n');

    out.push_str("[panel]\n");
    push_string(&mut out, "pos", s.panel.pos.as_str());
    out.push('\n');

    out.push_str("[tray]\n");
    push_string(&mut out, "pos", s.tray.pos.as_str());
    push_int(&mut out, "size", s.tray.size as i64);
    out.push('\n');

    out.push_str("[shortcuts]\n");
    push_bool(&mut out, "switcher", s.shortcuts.switcher);
    push_string(&mut out, "switch_mod", s.shortcuts.switch_mod.as_str());
    push_string(&mut out, "cmd_mod", s.shortcuts.cmd_mod.as_str());
    push_bool(&mut out, "ws_switch", s.shortcuts.ws_switch);
    push_bool(&mut out, "launcher", s.shortcuts.launcher);
    push_bool(&mut out, "win_close", s.shortcuts.win_close);
    push_bool(&mut out, "win_max", s.shortcuts.win_max);
    out.push('\n');

    out.push_str("[quicksettings]\n");
    push_bool(&mut out, "enabled", s.quicksettings.enabled);
    out.push('\n');

    out.push_str("[notifications]\n");
    push_bool(&mut out, "enabled", s.notifications.enabled);
    push_int(&mut out, "timeout_ms", s.notifications.timeout_ms as i64);
    push_int(&mut out, "corner", s.notifications.corner as i64);
    out.push('\n');

    out.push_str("[session]\n");
    push_bool(&mut out, "restore", s.session.restore);
    out.push_str("\n[transfers]\n");
    push_bool(&mut out,"trace",s.transfers.trace);
    push_int(&mut out,"timeout_ms",s.transfers.timeout_ms as i64);
    push_int(&mut out,"drag_threshold_px",s.transfers.drag_threshold_px as i64);
    out.push_str("\n[scripts]\n");
    push_bool(&mut out, "enabled", s.scripts.enabled);
    push_string(&mut out, "runner", s.scripts.runner.as_str());
    push_string(&mut out, "layout", s.scripts.layout.as_str());
    push_string(&mut out, "events", s.scripts.events.as_str());
    push_int(&mut out, "timeout_ms", s.scripts.timeout_ms as i64);

    out
}

/// True iff `to_toml` round-trips `s` through `parse_into` (meta/theme/layout/
/// effects/desktop/widgets/panel/tray/shortcuts/quicksettings/notifications/
/// session; the `from_file`/`applied` bookkeeping is not part of the value). A
/// cheap startup self-check that the serializer and parser stay in lockstep.
pub fn roundtrip_ok(s: &Settings) -> bool {
    let text = to_toml(s);
    let mut back = Settings::defaults();
    parse_into(&text, &mut back);
    back.meta == s.meta
        && back.theme == s.theme
        && back.layout == s.layout
        && back.effects == s.effects
        && back.desktop == s.desktop
        && back.widgets == s.widgets
        && back.display == s.display
        && back.panel == s.panel
        && back.tray == s.tray
        && back.shortcuts == s.shortcuts
        && back.quicksettings == s.quicksettings
        && back.notifications == s.notifications
        && back.session == s.session
        && back.scripts == s.scripts
        && back.transfers == s.transfers
}
