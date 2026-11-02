use dwm_settings::{validate, validate_document, Config};
#[test]
fn render_cadence_resolves_default_and_configured_values() {
    assert_eq!(Config::defaults().render.frame_interval_ms, 16);
    let base = "[application.test]\nname=\"Test\"\nexec=\"/app/test\"\n[workspaces]\ncount=1\n";
    assert_eq!(
        validate_document(base).unwrap().render.frame_interval_ms,
        16
    );
    for interval in [1, 8, 16, 33, 1000] {
        let config =
            validate_document(&format!("{base}[render]\nframe_interval_ms={interval}\n")).unwrap();
        assert!(config.valid);
        assert_eq!(config.render.frame_interval_ms, interval);
    }
}
#[test]
fn malformed_or_duplicate_render_policy_rejects_candidate() {
    for body in [
        "frame_interval_ms=0",
        "frame_interval_ms=1001",
        "frame_interval_ms=-1",
        "frame_interval_ms=1.5",
        "frame_interval_ms=\"16\"",
        "frame_interval_ms=true",
        "unknown=16",
        "frame_interval_ms=16\nframe_interval_ms=33",
    ] {
        assert!(validate(&format!("[render]\n{body}")).is_err(), "{body}");
    }
    assert!(validate("[render]\nframe_interval_ms=16\n[render]\nframe_interval_ms=33").is_err());
}

#[path = "../src/preserve.rs"]
mod preserve;
#[test]
fn settings_save_preserves_independent_cadence_table() {
    let source = "[render]\nframe_interval_ms=33\n";
    let merged = preserve::merge(
        &dwm_settings::to_toml(&dwm_settings::Settings::defaults()),
        source,
    );
    assert_eq!(
        validate_document(&merged).unwrap().render.frame_interval_ms,
        33
    );
}
