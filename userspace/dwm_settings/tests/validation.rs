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

#[test]
fn shell_resolves_multiple_panels_and_application_groups() {
    use dwm_settings::shell::{resolve, Align, ComponentKind, Direction};
    let text = r##"
[application.editor]
name = "Editor"
[panels.bottom]
edge = "bottom"
align = "center"
children = ["group", "nested"]
[panels.top]
edge = "top"
children = ["clock"]
[components.group]
kind = "app_group"
entries = ["editor"]
presentation = "fan"
[components.nested]
kind = "container"
direction = "vertical"
children = ["button"]
[components.button]
kind = "button"
action = "launch:editor"
[components.clock]
kind = "clock"
format = "%H:%M:%S"
font_px = 18
color = "#AABBCC"
interval_ms = 100
[shell]
stylesheet = "/assets/dwm/shell.dss"
"##;
    validate(text).unwrap();
    // Resolve uses registry IDs, so registry order has no effect on groups.
    let mut apps = dwm_settings::Applications::baseline();
    apps.apps.push(dwm_settings::AppEntry { id: "editor".into(), name: "Editor".into(), exec: "editor".into(), icon: "editor".into(), label: "Edit".into() });
    let shell = resolve(text, &Settings::defaults(), &apps).unwrap();
    assert!(shell.explicit);
    assert_eq!(shell.panels.len(), 2);
    assert!(shell.panels[0].align == Align::Center);
    assert!(shell.component("group").unwrap().kind == ComponentKind::AppGroup);
    assert_eq!(shell.component("group").unwrap().entries, ["editor"]);
    assert!(shell.component("nested").unwrap().direction == Direction::Vertical);
    assert_eq!(shell.component("clock").unwrap().font_px, 18);
    assert_eq!(shell.stylesheet, "/assets/dwm/shell.dss");
}

#[test]
fn shell_rejects_broken_trees_and_out_of_range_configuration() {
    let cases = [
        "[panels.a]\nchildren=[\"missing\"]",
        "[panels.a]\nchildren=[\"clock\",\"clock\"]\n[components.clock]\nkind=\"clock\"",
        "[panels.a]\nchildren=[\"clock\"]\n[components.clock]\nkind=\"clock\"\nchildren=[\"nested\"]\n[components.nested]\nkind=\"spacer\"",
        "[panels.a]\nchildren=[]\n[components.a]\nkind=\"container\"\nchildren=[\"b\"]\n[components.b]\nkind=\"container\"\nchildren=[\"a\"]",
        "[panels.a]\nchildren=[\"a\"]\n[components.a]\nsize=32",
        "[panels.a]\nchildren=[\"a\"]\n[components.a]\nkind=\"clock\"\ninterval_ms=0",
        "[panels.a]\nedge=\"diagonal\"",
        "[panels.a]\nsize=0",
        "[panels.a]\nchildren=[\"a\"]\n[components.a]\nkind=\"app_group\"\nentries=[\"missing\"]",
        "[shell]\nstylesheet=\"relative.dss\"",
        "[scripts]\nevents=\"/assets/../bad.lua\"",
    ];
    for text in cases { assert!(validate(text).is_err(), "{text}"); }
}

#[test]
fn legacy_shell_migration_preserves_placement_and_remains_implicit() {
    use dwm_settings::{shell::resolve, Edge};
    let mut settings = Settings::defaults();
    settings.panel.pos = Edge::Bottom;
    settings.tray.pos = Edge::Right;
    settings.layout.show_workspaces = false;
    let shell = resolve("", &settings, &dwm_settings::Applications::baseline()).unwrap();
    assert!(!shell.explicit);
    assert!(shell.panels[0].edge == Edge::Bottom);
    assert!(shell.panels[1].edge == Edge::Right);
    assert!(shell.component("legacy-workspaces").is_none());
    assert!(shell.component("legacy-title").unwrap().grow == 1);
}

#[test]
fn shell_resource_limits_and_depth_are_enforced() {
    let mut panels = String::new();
    for id in 0..=dwm_settings::shell::MAX_PANELS { panels.push_str(&format!("[panels.p{id}]\nsize=32\n")); }
    assert!(validate(&panels).is_err());
    let mut tree = "[panels.p]\nchildren=[\"c0\"]\n".to_string();
    for id in 0..=dwm_settings::shell::MAX_DEPTH {
        tree.push_str(&format!("[components.c{id}]\nkind=\"container\"\n"));
        if id < dwm_settings::shell::MAX_DEPTH { tree.push_str(&format!("children=[\"c{}\"]\n", id+1)); }
    }
    assert!(validate(&tree).is_err());
}

#[test]
fn policy_resolves_bindings_and_partial_rule_precedence() {
    use dwm_settings::policy::{resolve, Action};
    let text = r#"
[application.editor]
name="Editor"
[workspaces]
count=3
[bindings.edit]
scancode=18
modifiers="super+shift"
action="launch:editor"
[bindings.workspace]
scancode=3
modifiers="super"
action="workspace:2"
[rules.general]
app="*"
workspace=1
width=800
height=600
opacity=200
[rules.editor]
app="editor"
workspace=3
x=-10
floating=true
animation="genie"
[rules.editor-size]
app="editor"
width=1000
"#;
    validate(text).unwrap();
    let mut apps = dwm_settings::Applications::baseline();
    apps.workspaces = 3;
    apps.apps.push(dwm_settings::AppEntry { id: "editor".into(), name: "Editor".into(), exec: "editor".into(), icon: "editor".into(), label: "Edit".into() });
    let config = resolve(text, &apps).unwrap();
    assert_eq!(config.bindings[0].modifiers, 9);
    assert!(config.bindings[0].action == Action::Launch("editor".into()));
    assert!(config.bindings[1].action == Action::Workspace(1));
    let policy = config.for_app("editor");
    assert_eq!(policy.workspace, Some(2));
    assert_eq!(policy.width, Some(1000));
    assert_eq!(policy.height, Some(600));
    assert_eq!(policy.opacity, Some(200));
    assert_eq!(policy.x, Some(-10));
    assert_eq!(policy.floating, Some(true));
    assert_eq!(policy.animation.as_deref(), Some("genie"));
    assert_eq!(config.for_app("other").width, Some(800));
}

#[test]
fn policy_rejects_missing_fields_invalid_actions_and_duplicate_chords() {
    for text in [
        "[bindings.a]\nscancode=18",
        "[bindings.a]\nscancode=128\naction=\"launcher\"",
        "[bindings.a]\nscancode=18\nmodifiers=\"super+super\"\naction=\"launcher\"",
        "[bindings.a]\nscancode=18\naction=\"launch:missing\"",
        "[bindings.a]\nscancode=18\naction=\"workspace:2\"",
        "[bindings.a]\nscancode=18\naction=\"launcher\"\n[bindings.b]\nscancode=18\naction=\"close\"",
        "[rules.a]\napp=\"missing\"\nwidth=300",
        "[rules.a]\napp=\"*\"\nwidth=0",
        "[rules.a]\napp=\"*\"\nopacity=256",
        "[rules.a]\napp=\"*\"\nworkspace=0",
        "[rules.a]\nwidth=300",
        "[rules.a]\napp=\"*\"\nanimation=\"broken\"",
        "[panels.p]\nchildren=[\"button\"]\n[components.button]\nkind=\"button\"\naction=\"shell:rm\"",
    ] { assert!(validate(text).is_err(), "{text}"); }
}

#[test]
fn widget_clock_style_and_dimensions_are_atomic() {
    use dwm_settings::{ClockStyle, WidgetCfg};
    let mut config = WidgetCfg::baseline();
    config.apply("[widget]\nstyle=\"analog\"\nsize=144\nfont_px=16\ninterval_ms=100").unwrap();
    assert!(config.style == ClockStyle::Analog);
    assert_eq!(config.size, 144);
    let before = config;
    for text in ["[widget]\nstyle=\"digital\"\nsize=0", "[widget]\ninterval_ms=0", "[widget]\nstyle=\"unknown\"", "[widget]\nfont_px=200"] {
        assert!(config.apply(text).is_err());
        assert!(config == before);
    }
    let root = std::path::Path::new(env!("DUNIT_ROOT"));
    config.apply(&std::fs::read_to_string(root.join("assets/dwm/widgets/clock.toml")).unwrap()).unwrap();
}

#[test]
fn complete_document_validation_resolves_schema_and_rejects_before_io() {
    let root = std::path::Path::new(env!("DUNIT_ROOT"));
    let source = std::fs::read_to_string(root.join("assets/dwm/default.toml")).unwrap();
    let config = dwm_settings::validate_document(&source).unwrap();
    assert!(config.valid);
    assert_eq!(config.settings.meta.schema_version, dwm_settings::SCHEMA_VERSION);
    assert!(!config.shell.panels.is_empty());
    // Test doubles panic on filesystem I/O, proving invalid saves never write.
    assert!(dwm_settings::save_document("[panels.a]\nchildren=[\"missing\"]").is_err());
    assert!(dwm_settings::validate_document("[meta]\nschema_version=999").is_err());
    for source in ["", "[meta]\nschema_version=2", "[theme]\npanel=\"#112233\"", "[meta]\nschema_version=999"] {
        assert!(dwm_settings::save_document(source).is_err());
    }
    let error=dwm_settings::save_document("[meta]\nschema_version=999").unwrap_err();
    assert_eq!(error.field,"meta.schema_version");
}

#[test]
fn full_desktop_examples_resolve_real_component_trees() {
    let root=env!("DUNIT_ROOT");
    for name in ["frost-desktop.toml","multi-panel-desktop.toml"] {
        let source=std::fs::read_to_string(format!("{root}/assets/dwm/{name}")).unwrap();
        let config=dwm_settings::validate_document(&source).unwrap();
        assert!(config.valid && config.shell.explicit);
        assert!(!config.policy.bindings.is_empty());
        assert!(config.shell.components.iter().any(|c|c.kind==dwm_settings::shell::ComponentKind::AppGroup));
    }
}

#[test]
fn running_app_sources_deduplicate_and_preserve_pinned_order() {
    use dwm_settings::shell::AppsSource;
    assert_eq!(AppsSource::Pinned.resolve_entries(&[3,1,3],&[2,1]),[3,1]);
    assert_eq!(AppsSource::Running.resolve_entries(&[3,1],&[2,1,2]),[2,1]);
    assert_eq!(AppsSource::Both.resolve_entries(&[3,1],&[2,1,2]),[3,1,2]);
    assert!(AppsSource::Running.resolve_entries(&[3,1],&[]).is_empty());
    for text in [
        "[panels.p]\nchildren=[\"a\"]\n[components.a]\nkind=\"apps\"\nsource=\"broken\"",
        "[panels.p]\nchildren=[\"a\"]\n[components.a]\nkind=\"clock\"\nsource=\"running\"",
        "[panels.p]\nchildren=[\"a\"]\n[components.a]\nkind=\"launcher\"\nicon=\"relative.svg\"",
        "[panels.p]\nchildren=[\"a\"]\n[components.a]\nkind=\"launcher\"\nicon=\"/assets/../image.svg\"",
    ] {assert!(validate(text).is_err(),"{text}");}
    validate("[panels.p]\nchildren=[\"a\"]\n[components.a]\nkind=\"app_group\"\nicon=\"/assets/icons/aqua/mime_folder.svg\"").unwrap();
}

#[test]
fn lua_widget_data_targets_are_typed_bounded_and_transactional() {
    use dwm_settings::shell::resolve;
    let source="[panels.p]\nchildren=[\"indicator\",\"button\",\"clock\"]\n[components.indicator]\nkind=\"indicator\"\nlabel=\"Original\"\n[components.button]\nkind=\"button\"\naction=\"launcher\"\n[components.clock]\nkind=\"clock\"";
    validate(source).unwrap();
    let shell=resolve(source,&Settings::defaults(),&dwm_settings::Applications::baseline()).unwrap();
    let candidate=shell.with_widget_updates([(1,"Привет",Some(0xFFAABBCC)),(2,"Open",None)]).unwrap();
    assert_eq!(candidate.components[0].label,"Привет");
    assert_eq!(candidate.components[0].color,Some(0xFFAABBCC));
    assert_eq!(candidate.components[1].label,"Open");
    for updates in [vec![(1,"Changed",None),(3,"Wrong kind",None)],vec![(0,"Zero",None)],vec![(99,"Missing",None)],vec![(1,"bad\nline",None)]] {
        assert!(shell.with_widget_updates(updates).is_err());
        assert_eq!(shell.components[0].label,"Original");
    }
    let oversized="x".repeat(257);
    assert!(shell.with_widget_updates([(1,oversized.as_str(),None)]).is_err());
}

#[test]
fn cross_reference_errors_identify_the_original_field_line(){
    let source="[panels.main]\nchildren=[\"missing\"]\n";
    let error=dwm_settings::validate(source).unwrap_err();assert_eq!(error.line,2);assert!(error.field.contains("children"));
    assert!(dwm_settings::save_user_document("[meta]\nschema_version=2").is_err());
}
