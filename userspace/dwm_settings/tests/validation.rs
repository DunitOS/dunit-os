use dwm_settings::{to_toml, validate, Settings};
#[test]
fn serializer_and_shipped_presets_validate() {
    validate(&to_toml(&Settings::defaults())).unwrap();
    let root = std::path::Path::new(env!("DUNIT_ROOT"));
    validate(&std::fs::read_to_string(root.join("assets/dwm/default.toml")).unwrap()).unwrap();
    validate(&std::fs::read_to_string(root.join("assets/dwm/test.toml")).unwrap()).unwrap();
    for path in std::fs::read_dir(root.join("assets/dwm/themes")).unwrap() {
        let path = path.unwrap().path();
        if path.extension().is_some_and(|s| s == "toml") {
            validate(&std::fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("{}: {:?}", path.display(), e));
        }
    }
}
#[test]
fn rejects_invalid_known_fields_unknowns_and_clamping() {
    for text in [
        "[theme]\ngarbage=42",
        "[theme]\nclose=\"broken\"",
        "[effects]\nblur=maybe",
        "[layout]\ndock_w=99999",
        "[workspaces]\ncount=0",
        "[meta]\nschema_version=999",
        "[theme]\nclose=\"#112233",
        "[unknown]\nx=1",
        "broken",
        "",
    ] {
        assert!(validate(text).is_err(), "{text}");
    }
    let e = validate("# comment\n[effects]\nblur = maybe").unwrap_err();
    assert_eq!(e.line, 3);
    assert_eq!(e.field, "effects.blur");
}
#[test]
fn registry_rejects_missing_ids_bad_arrays_and_duplicates() {
    for text in [
        "[application.a]\nname=\"A\"\n[dock]\nentries=[\"missing\"]",
        "[dock]\nentries=gui_calc",
        "[startup]\nself_test=maybe",
        "[application.a]\nexec=42",
        "[theme]\nclose=\"#112233\"\nclose=\"#445566\"",
        "[theme]\n[theme]\nclose=\"#112233\"",
    ] {
        assert!(validate(text).is_err(), "{text}");
    }
    validate("[application.a]\nname=\"A\"\n[dock]\nentries=[\"a\"]").unwrap();
    validate("[startup]\nentries=[]").unwrap();
}

#[test]
fn standalone_terminal_and_files_reject_atomically() {
    use dwm_settings::{TerminalCfg, FilesCfg};
    let mut terminal = TerminalCfg::baseline();
    let before = terminal;
    for text in ["[terminal]\nfg=\"#123456\"\nbg_alpha=300", "[terminal]\ntheme=\"missing\"", "[terminal]\nunknown=true"] {
        assert!(terminal.apply(text).is_err());
        assert!(terminal == before);
    }
    terminal.apply("[terminal]\ntheme=\"catppuccin_mocha\"\nbg_alpha=210").unwrap();
    assert_eq!(terminal.bg_alpha, 210);
    let mut files = FilesCfg::baseline();
    let before = files;
    assert!(files.apply("[files]\nbg=\"#123456\"\nbg_alpha=-1").is_err());
    assert!(files == before);
    let root = std::path::Path::new(env!("DUNIT_ROOT"));
    for path in ["assets/dwm/apps/gui_terminal.toml"] {
        terminal.apply(&std::fs::read_to_string(root.join(path)).unwrap()).unwrap();
    }
    for path in ["assets/dwm/apps/gui_files.toml", "assets/dwm/apps/test/gui_files.toml"] {
        files.apply(&std::fs::read_to_string(root.join(path)).unwrap()).unwrap();
    }
}
