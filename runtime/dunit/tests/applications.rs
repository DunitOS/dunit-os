//! SDK application source guard. Service and explicit capability-mechanism
//! probes are outside the window-client inventory; they retain lower layers.
fn sources(folder: &std::path::Path, output: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, output)
        } else if path.extension().is_some_and(|e| e == "rs") {
            output.push(path);
        }
    }
}
#[test]
fn sdk_applications_use_the_generated_entry_and_sdk_dependency() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../userspace/system_apps");
    for name in [
        "calc",
        "sdk_api_test",
        "gui_stat",
        "gui_calc",
        "gui_demo",
        "gui_client",
        "gui_terminal",
        "gui_settings",
        "gui_files",
    ] {
        let app = root.join(name);
        let main = std::fs::read_to_string(app.join("src/main.rs")).unwrap();
        let manifest = std::fs::read_to_string(app.join("Cargo.toml")).unwrap();
        assert!(main.contains("#[dunit::main]"), "{name}");
        assert!(!manifest.contains("libdunit"), "{name}");
        let mut files = Vec::new();
        sources(&app.join("src"), &mut files);
        for path in files {
            let source = std::fs::read_to_string(&path).unwrap();
            for forbidden in [
                "libdunit::",
                "extern \"C\"",
                "#[panic_handler]",
                "gui_protocol_v1::",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{}: {forbidden}",
                    path.display()
                );
            }
            for word in source.split(|c: char| !c.is_alphanumeric() && c != '_') {
                assert!(
                    !["unsafe", "_start", "syscall"].contains(&word),
                    "{}: {word}",
                    path.display()
                );
            }
        }
    }
}
