//! M7 migration guard: declared SDK applications own no entry/ABI plumbing.
#[test]
fn sdk_applications_use_the_generated_entry_and_sdk_dependency() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../userspace/system_apps");
    for name in ["calc", "sdk_api_test"] {
        let app = root.join(name);
        let source = std::fs::read_to_string(app.join("src/main.rs")).unwrap();
        let manifest = std::fs::read_to_string(app.join("Cargo.toml")).unwrap();
        assert!(source.contains("#[dunit::main]"), "{name}");
        assert!(!manifest.contains("libdunit"), "{name}");
        for forbidden in [
            "unsafe",
            "extern \"C\"",
            "libdunit",
            "_start",
            "#[panic_handler]",
            "syscall",
        ] {
            assert!(!source.contains(forbidden), "{name}: {forbidden}");
        }
    }
}
