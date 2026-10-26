const VALID: &str = "schema_version=1\n[application]\nid='example.monitor'\nname=\"Monitor \\u03a9\"\nkind='gui'\nbinary='gui_stat'\nicon='/assets/icons/monitor.rgba'\ncapabilities=['READ']\n";
#[test]
fn full_toml_and_native_rights_are_validated() {
    let app = dunit_app_manifest::parse(VALID).unwrap();
    assert_eq!(app.name, "Monitor Ω");
    assert_eq!(app.binary, "gui_stat");
    for text in [
        VALID.replace("schema_version=1", "schema_version=2"),
        VALID.replace("capabilities=['READ']", "capabilities=['ROOT']"),
        VALID.replace("capabilities=['READ']", "capabilities=['READ','READ']"),
        VALID.replace("id='example.monitor'", "id='../escape'"),
        VALID.replace("icon='/assets/icons/monitor.rgba'", "icon='/assets/../bad'"),
        format!("{VALID}unknown=42\n"),
        VALID.replace("binary='gui_stat'\n", ""),
    ] {
        assert!(dunit_app_manifest::parse(&text).is_err(), "{text}");
    }
}

#[test]
fn shared_host_and_guest_contract_fixtures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for file in std::fs::read_dir(root).unwrap() {
        let path = file.unwrap().path();
        let valid = path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(".valid.toml");
        assert_eq!(
            dunit_app_manifest::parse(&std::fs::read_to_string(&path).unwrap()).is_ok(),
            valid,
            "{}",
            path.display()
        );
    }
}
