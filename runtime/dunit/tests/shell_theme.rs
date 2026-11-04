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
    dunit_component::Theme::parse_with_defaults(&profile, DEFAULTS, BASE, "").unwrap()
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
    assert!(dunit_component::Theme::parse_with_defaults(&bad, DEFAULTS, BASE, "").is_err());
    assert!(dunit_component::Theme::parse_with_defaults(
        DARK,
        "Panel { background:$surface; }",
        BASE,
        ""
    )
    .is_err());
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
