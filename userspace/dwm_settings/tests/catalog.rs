use dwm_settings::validate_document_with_manifests as resolve;
fn app() -> dunit_app_manifest::Application {
    dunit_app_manifest::parse(
        r#"schema_version=1
[application]
id="example.monitor"
name="Manifest Monitor"
kind="gui"
binary="gui_stat"
icon="/assets/icons/monitor.rgba"
capabilities=[]
"#,
    )
    .unwrap()
}
#[test]
fn manifest_registry_resolves_lists_without_desktop_app_tables() {
    let text="[meta]\nschema_version=2\n[dock]\nentries=[\"example.monitor\"]\n[launcher]\nentries=[\"example.monitor\"]\n[startup]\nentries=[\"example.monitor\"]";
    let config = resolve(text, &[app()]).unwrap();
    assert!(config.valid);
    let found = &config.apps.apps[config.apps.dock[0]];
    assert_eq!(found.id, "example.monitor");
    assert_eq!(found.name, "Manifest Monitor");
    assert_eq!(found.exec, "/app/gui_stat");
    assert_eq!(config.apps.launcher, config.apps.dock);
    assert_eq!(config.apps.startup, config.apps.dock);
}
#[test]
fn explicit_presentation_and_empty_lists_override_defaults() {
    let config=resolve("[meta]\nschema_version=2\n[application.example.monitor]\nname=\"My Monitor\"\nlabel=\"MY\"\n[dock]\nentries=[]\n[launcher]\nentries=[]",&[app()]).unwrap();
    assert!(config.valid);
    assert_eq!(config.apps.apps[0].name, "My Monitor");
    assert_eq!(config.apps.apps[0].label, "MY");
    assert_eq!(config.apps.apps[0].exec, "/app/gui_stat");
    assert!(config.apps.dock.is_empty());
    assert!(config.apps.launcher.is_empty());
}
#[test]
fn legacy_apps_mix_with_catalog_and_unknown_references_still_fail() {
    let config = resolve("[application.legacy]\nname=\"Legacy\"", &[app()]).unwrap();
    assert_eq!(config.apps.apps.len(), 2);
    assert_eq!(config.apps.launcher.len(), 2);
    assert!(resolve("[launcher]\nentries=[\"missing\"]", &[app()]).is_err());
    assert!(resolve("[meta]\nschema_version=2", &[app(), app()]).is_err());
}
#[test]
fn cli_metadata_does_not_spawn_an_invisible_console_from_the_desktop() {
    let mut app = app();
    app.kind = dunit_app_manifest::Kind::Cli;
    app.icon = None;
    assert!(resolve("[meta]\nschema_version=2", &[app]).is_err());
}

#[test]
fn combined_registry_limit_rejects_instead_of_dropping_an_app() {
    let mut manifests = Vec::new();
    for index in 0..dwm_settings::MAX_APPS {
        let mut entry = app();
        entry.id = format!("installed{}", index);
        manifests.push(entry);
    }
    assert!(resolve("[application.overflow]\nname=\"Overflow\"", &manifests).is_err());
    for id in [".", ".."] {
        assert!(resolve(&format!("[application.{id}]\nname=\"bad\""), &[]).is_err());
    }
}

#[test]
fn console_entries_keep_kind_and_use_a_configured_adapter() {
    let mut cli = app();
    cli.id = "example.cli".into();
    cli.kind = dwm_settings::AppKind::Cli;
    cli.icon = None;
    let mut tui = cli.clone();
    tui.id = "example.tui".into();
    tui.kind = dwm_settings::AppKind::Tui;
    let config = resolve("[launcher]\nterminal=\"/app/custom-terminal\"", &[cli, tui]).unwrap();
    assert!(config.valid);
    assert_eq!(
        config.apps.terminal.as_deref(),
        Some("/app/custom-terminal")
    );
    assert_eq!(config.apps.apps[0].kind, dwm_settings::AppKind::Cli);
    assert_eq!(config.apps.apps[1].kind, dwm_settings::AppKind::Tui);
    assert_eq!(config.apps.launcher, [0, 1]);
    assert_eq!(config.apps.apps[0].exec, "/app/gui_stat");
}
#[test]
fn unselected_console_entries_do_not_require_a_terminal() {
    let mut cli = app();
    cli.kind = dwm_settings::AppKind::Cli;
    let config = resolve(
        "[dock]\nentries=[]\n[launcher]\nentries=[]\n[startup]\nentries=[]",
        &[cli],
    )
    .unwrap();
    assert!(config.valid);
    assert!(config.apps.terminal.is_none());
}
#[test]
fn invalid_adapter_paths_reject_the_complete_document() {
    for path in [
        "",
        "relative",
        "/",
        "//app/term",
        "/app/../term",
        "/app//term",
        "/app/./term",
    ] {
        assert!(
            resolve(&format!("[launcher]\nterminal={path:?}"), &[app()]).is_err(),
            "{path}"
        );
    }
}
