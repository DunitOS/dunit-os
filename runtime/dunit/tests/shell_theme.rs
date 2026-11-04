#![cfg(feature = "gui")]
extern crate alloc;
use dunit_component::style as dunit_style;
use dwm_settings as settings;
use settings::Theme;
#[path = "../../../userspace/system_apps/gui_server/src/shell_palette.rs"]
mod shell_palette;
const DARK: &str = include_str!("../../../assets/dwm/sdk/dark.dss");
const LIGHT: &str = include_str!("../../../assets/dwm/sdk/light.dss");
const DEFAULTS: &str = include_str!("../../../assets/dwm/sdk/shell.dss");
const BASE: &str = include_str!("../../../assets/dwm/sdk/base.dss");
fn argb(c: dunit_style::Color) -> u32 {
    (c.a as u32) << 24 | (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
}
fn resolve(source: &str, document: &str) -> dunit_component::Theme {
    let config = settings::validate_document(document).unwrap();
    let profile = settings::appearance::profile(source, &config);
    dunit_component::Theme::parse_shell(&profile, DEFAULTS, BASE, "").unwrap()
}
#[test]
fn compositor_and_sdk_inherit_the_selected_profile() {
    for source in [DARK, LIGHT] {
        let shared = resolve(source, "[desktop]\nsdk_theme=\"/theme.dss\"");
        let shell = shell_palette::palette(&shared.tokens).unwrap();
        let c = shared.tokens.color("surface").unwrap();
        assert_eq!(
            shell.panel,
            (c.a as u32) << 24 | (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
        );
        assert_eq!(shell.desktop, argb(shared.background));
    }
}
#[test]
fn explicit_legacy_fields_override_only_supplied_tokens_and_preserve_alpha() {
    let shared = resolve(
        LIGHT,
        "[theme]\npanel=\"#80112233\" # user color\nclose=\"#c0ffee\"\n[widgets]\nenabled=false",
    );
    let shell = shell_palette::palette(&shared.tokens).unwrap();
    assert_eq!(shell.panel, 0x80112233);
    assert_eq!(shell.close, 0xffc0ffee);
    assert_eq!(shell.desktop, argb(shared.background));
    assert_ne!(shell.title_focused, shell.panel);
}
#[test]
fn profile_shell_overrides_win_over_defaults_and_invalid_colors_are_rejected() {
    let source = format!("{DARK}\n$shell-panel: #123456;");
    assert_eq!(
        shell_palette::palette(&resolve(&source, "[widgets]\nenabled=false").tokens)
            .unwrap()
            .panel,
        0xff123456
    );
    let bad = format!("{DARK}\n$shell-panel: 12;");
    assert!(dunit_component::Theme::parse_shell(&bad, DEFAULTS, BASE, "").is_err());
    assert!(
        dunit_component::Theme::parse_shell(DARK, "Panel { background:$surface; }", BASE, "")
            .is_err()
    );
}
#[test]
fn shell_stylesheet_can_use_sdk_tokens() {
    let shared = resolve(LIGHT, "[widgets]\nenabled=false");
    let sheet = dunit_style::parse_with_tokens(
        "Panel { background:$accent; color:$shell-panel-text; }",
        &shared.tokens,
    )
    .unwrap();
    assert_eq!(
        sheet.rules[0].declarations[0].value.as_color(),
        shared.tokens.color("accent")
    );
}

use settings::{Effects, Layout};
#[path = "../../../userspace/system_apps/gui_server/src/shell_metrics.rs"]
mod shell_metrics;
fn visuals(source: &str, document: &str) -> dunit_style::shell::Visuals {
    dunit_style::shell::Visuals::from_tokens(&resolve(source, document).tokens).unwrap()
}
#[test]
fn dss_geometry_reaches_the_legacy_shell_and_explicit_toml_keeps_precedence() {
    let source = format!("{DARK}\n$shell-layout-title-h: 40; $shell-layout-dock-w: 72; $shell-effects-corner-radius: 13;");
    let config = settings::validate_document("[application.example]\nname=\"Example\"\nexec=\"/app/example\"\n[dock]\nentries=[\"example\"]\n[layout]\nborder=3\n[effects]\nshadow=12").unwrap();
    let skin = visuals(&source, "[layout]\nborder=3\n[effects]\nshadow=12");
    let mut scalar = config.settings;
    scalar.layout = shell_metrics::layout(&scalar.layout, &skin);
    scalar.effects = shell_metrics::effects(&scalar.effects, &skin);
    assert_eq!(
        (
            scalar.layout.title_h,
            scalar.layout.border,
            scalar.layout.dock_w
        ),
        (40, 3, 72)
    );
    assert_eq!(
        (scalar.effects.corner_radius, scalar.effects.shadow),
        (13, 12)
    );
    let tree = settings::shell::from_legacy(&scalar, &config.apps);
    let dock = tree.panels.iter().find(|p| p.id == "legacy-dock").unwrap();
    assert_eq!(dock.size, 72);
    let skin = visuals(
        &source,
        "[layout]\ntitle_h=31\ndock_w=56\n[effects]\ncorner_radius=9",
    );
    assert_eq!(
        (
            skin.layout_title_h,
            skin.layout_dock_w,
            skin.effects_corner_radius
        ),
        (31, 56, 9)
    );
}
#[test]
fn quick_settings_rows_use_the_selected_card_geometry() {
    let source = format!("{LIGHT}\n$shell-qs-width: 196; $shell-qs-padding: 10;");
    let skin = visuals(&source, "[widgets]\nenabled=false");
    let (width, height) = skin.qs_size(30, 2);
    assert_eq!((width, height), (196, 80));
    let flyout = (100, 28, width, height);
    let first = skin.qs_row(flyout, 30, 0);
    let second = skin.qs_row(flyout, 30, 1);
    assert_eq!(first, (110, 38, 176, 30));
    assert_eq!(second.1, first.1 + first.3);
    assert!(second.1 + second.3 <= flyout.1 + flyout.3);
}
#[test]
fn invalid_shell_geometry_is_rejected_before_candidate_application() {
    for patch in [
        "$shell-qs-padding: -1;",
        "$shell-qs-padding: 1.5;",
        "$shell-hover-alpha: 256;",
        "$shell-layout-title-font-px: 0;",
        "$shell-group-columns: 0;",
        "$shell-qs-width: 4;",
        "$shell-qs-knob-inset: 6;",
        "$shell-switcher-icon-inset: 32;",
        "$shell-widget-padding: 118;",
        "$shell-notification-accent-inset: 15;",
        "$shell-menu-icon-min: 25;",
        "$shell-cursor-fill: #ffffff80;",
    ] {
        assert!(
            dunit_component::Theme::parse_shell(&format!("{DARK}\n{patch}"), DEFAULTS, BASE, "")
                .is_err(),
            "{patch}"
        );
    }
}

#[test]
fn removing_shell_schema_tokens_does_not_disable_candidate_validation() {
    let defaults = DEFAULTS.replace("$shell-cursor-outline: #000000;", "");
    assert!(dunit_component::Theme::parse_shell(DARK, &defaults, BASE, "").is_err());
    for patch in [
        "$shell-cursor-source: relative.txt;",
        "$shell-cursor-source:/assets/../cursor.txt;",
    ] {
        assert!(dunit_component::Theme::parse_shell(
            &format!("{DARK}\n{patch}"),
            DEFAULTS,
            BASE,
            ""
        )
        .is_err());
    }
}

#[test]
fn sparse_settings_save_retains_dss_inheritance_but_explicit_default_overrides_it() {
    let source = "[desktop]\nsdk_theme=\"/theme.dss\"\n[widgets]\nenabled=false\n";
    let profile = format!("{DARK}\n$shell-layout-title-h:40; $shell-panel-text:#123456;");
    let before = settings::validate_document(source).unwrap().settings;
    let mut after = before;
    after.display.width = 1280;
    after.display.height = 720;
    let patch = settings::scalar_changes(&before, &after).unwrap();
    let saved = settings::apply_scalar_overlay(source, &patch).unwrap();
    assert_eq!(visuals(&profile, &saved).layout_title_h, 40);
    assert_eq!(
        shell_palette::palette(&resolve(&profile, &saved).tokens)
            .unwrap()
            .panel_text,
        0xff123456
    );
    let pinned = settings::apply_scalar_overlay(
        &saved,
        "[layout]\ntitle_h=26\n[theme]\npanel_text=\"#CDD6F4\"",
    )
    .unwrap();
    assert_eq!(visuals(&profile, &pinned).layout_title_h, 26);
    assert_eq!(
        shell_palette::palette(&resolve(&profile, &pinned).tokens)
            .unwrap()
            .panel_text,
        0xffcdd6f4
    );
}
