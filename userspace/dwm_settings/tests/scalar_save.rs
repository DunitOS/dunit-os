use dwm_settings::{apply_scalar_overlay, scalar_changes, validate_document, Settings};
const SOURCE: &str = "# Desktop source stays editable\n[meta]\nschema_version = 2\n[desktop] # selected SDK profile\n# Keep this note\nsdk_theme = \"/custom.dss\"\n[notifications]\nenabled = true\n[application.example]\nname = \"Example\"\nexec = \"/app/example\"\n[dock]\nentries = [\"example\"]\n";
fn values(source: &str) -> Settings {
    validate_document(source).unwrap().settings
}
#[test]
fn an_unrelated_scalar_save_does_not_materialize_inherited_appearance() {
    let before = values(SOURCE);
    let mut after = before;
    after.display.width = 1280;
    after.display.height = 720;
    let delta = scalar_changes(&before, &after).unwrap();
    assert!(
        !delta.contains("[theme]") && !delta.contains("[layout]") && !delta.contains("[effects]")
    );
    let saved = apply_scalar_overlay(SOURCE, &delta).unwrap();
    assert!(saved.starts_with(SOURCE));
    assert!(!saved.contains("[theme]") && !saved.contains("[layout]"));
    assert_eq!(
        (values(&saved).display.width, values(&saved).display.height),
        (1280, 720)
    );
}
#[test]
fn explicit_default_assignments_remain_explicit() {
    let before = values(SOURCE);
    assert_eq!(before.layout.title_h, 26);
    let saved = apply_scalar_overlay(SOURCE, "[layout]\ntitle_h=26").unwrap();
    assert!(saved.contains("title_h = 26"));
    assert!(validate_document(&saved)
        .unwrap()
        .appearance
        .contains("$shell-layout-title-h: 26;"));
}
#[test]
fn unchanged_snapshots_produce_no_patch_and_metadata_is_read_only() {
    let before = values(SOURCE);
    assert_eq!(scalar_changes(&before, &before).unwrap(), "");
    let mut after = before;
    after.meta.schema_version = 1;
    assert!(scalar_changes(&before, &after).is_err());
    let mut unsupported = before;
    unsupported.meta.schema_version = u32::MAX;
    assert!(scalar_changes(&unsupported, &unsupported).is_err());
    assert!(apply_scalar_overlay(SOURCE, "").is_err());
    assert!(apply_scalar_overlay(SOURCE, "[meta]\nschema_version=2").is_err());
}
#[test]
fn unrelated_changes_since_the_snapshot_survive() {
    let before = values(SOURCE);
    let mut after = before;
    after.notifications.enabled = false;
    let current = apply_scalar_overlay(SOURCE, "[layout]\ntitle_h=42").unwrap();
    let saved = apply_scalar_overlay(&current, &scalar_changes(&before, &after).unwrap()).unwrap();
    let v = values(&saved);
    assert_eq!(v.layout.title_h, 42);
    assert!(!v.notifications.enabled);
    assert!(saved.contains("[desktop] # selected SDK profile\n# Keep this note\n"));
}
#[test]
fn staged_explicit_updates_replace_each_other_without_duplicate_tables() {
    let first = apply_scalar_overlay("", "[layout]\ntitle_h=26").unwrap();
    let staged =
        apply_scalar_overlay(&first, "[layout]\ntitle_h=34\n[effects]\nshadow=12").unwrap();
    let saved = apply_scalar_overlay(SOURCE, &staged).unwrap();
    assert_eq!(saved.matches("[layout]").count(), 1);
    assert_eq!(saved.matches("title_h =").count(), 1);
    assert_eq!(
        (values(&saved).layout.title_h, values(&saved).effects.shadow),
        (34, 12)
    );
}
#[test]
fn partial_theme_import_keeps_explicit_equal_values_and_independent_source() {
    let serialized = dwm_settings::to_toml(&values(SOURCE));
    let color = dwm_settings::value(&serialized, "theme.panel_text").unwrap();
    let overlay=format!("[meta]\nschema_version=2\n[theme]\npanel_text={color}\n[desktop]\nsdk_theme=\"/light.dss\"\n");
    let saved = apply_scalar_overlay(SOURCE, &overlay).unwrap();
    assert!(saved.contains("panel_text ="));
    assert!(saved.contains("[application.example]\nname = \"Example\"\nexec = \"/app/example\"\n[dock]\nentries = [\"example\"]\n"));
    assert!(saved.contains(
        "[desktop] # selected SDK profile\n# Keep this note\nsdk_theme = \"/light.dss\""
    ));
    assert_eq!(saved.matches("schema_version").count(), 1);
    assert!(!saved.contains("[layout]"));
}
#[test]
fn invalid_or_non_scalar_updates_leave_the_source_untouched() {
    let original = SOURCE.to_string();
    for bad in [
        "[layout]\ndock_w=99999",
        "[theme]\nclose=\"broken\"",
        "[application.example]\nname=\"Changed\"",
        "[unknown]\nvalue=3",
    ] {
        assert!(apply_scalar_overlay(&original, bad).is_err(), "{bad}");
        assert_eq!(original, SOURCE);
    }
}
#[test]
fn changed_fields_keep_table_order_and_untouched_comments() {
    let saved = apply_scalar_overlay(SOURCE, "[notifications]\nenabled=false").unwrap();
    assert_eq!(saved, SOURCE.replace("enabled = true", "enabled = false"));
}

#[test]
fn packaged_gui_smoke_scalar_state_survives_a_sparse_resolution_save() {
    let root=std::path::Path::new(env!("DUNIT_ROOT"));
    let source=std::fs::read_to_string(root.join("assets/dwm/test.toml")).unwrap();
    let before=values(&source);
    let mut after=before;after.display.width=1600;after.display.height=900;
    let updated=apply_scalar_overlay(&source,&scalar_changes(&before,&after).unwrap()).unwrap();
    assert_eq!(dwm_settings::to_toml(&values(&updated)),dwm_settings::to_toml(&after));
    assert_eq!(validate_document(&source).unwrap().apps.self_test,validate_document(&updated).unwrap().apps.self_test);
}
