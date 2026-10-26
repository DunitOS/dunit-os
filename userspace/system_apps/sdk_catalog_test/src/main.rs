#![no_std]
#![no_main]
const PATH: &str = "/system/share/dunit/apps/example.monitor/dunit.toml";
#[dunit::main]
fn main() -> dunit::Result<()> {
    if dunit::args::args()?.get(1).map(|value| value.as_str()) == Some("--absent") {
        let mut entries = [libdunit::DirEntry::empty(); 1];
        assert_eq!(
            libdunit::readdir("/system/share/dunit/apps", &mut entries),
            dunit::errno::ENOENT
        );
        dunit::fs::write("sdk-catalog-not-directory", b"file")?;
        assert_eq!(
            libdunit::readdir("sdk-catalog-not-directory", &mut entries),
            dunit::errno::ENOTDIR
        );
        dunit::fs::remove_file("sdk-catalog-not-directory")?;
        assert!(dwm_settings::load_config().valid);
        dunit::println!("[SDKCAT] absent catalog legacy fallback and directory errno OK")?;
        return Ok(());
    }
    let original = dunit::fs::read_to_string(PATH)?;
    let config = dwm_settings::load_config();
    assert!(config.valid);
    let index = config
        .apps
        .apps
        .iter()
        .position(|app| app.id == "example.monitor")
        .unwrap();
    let app = &config.apps.apps[index];
    assert_eq!(app.name, "Manifest Monitor");
    assert_eq!(app.exec, "/app/gui_stat");
    assert_eq!(app.icon, "/assets/icons/monitor.rgba");
    assert_eq!(config.apps.dock, [index]);
    assert_eq!(config.apps.launcher, [index]);
    assert_eq!(config.apps.startup, [index]);
    dunit::println!("[SDKCAT] manifest resolved dock launcher startup OK")?;
    let renamed = original.replace("Manifest Monitor", "Renamed Monitor");
    dunit::fs::write(PATH, renamed.as_bytes())?;
    let changed = dwm_settings::load_config();
    assert!(changed.valid);
    assert_eq!(changed.apps.apps[index].name, "Renamed Monitor");
    dunit::println!("[SDKCAT] metadata reread OK")?;
    dunit::fs::write(
        PATH,
        original
            .replace("schema_version = 1", "schema_version = 999")
            .as_bytes(),
    )?;
    assert!(!dwm_settings::load_config().valid);
    assert_eq!(config.apps.apps[index].name, "Manifest Monitor");
    dunit::fs::write(PATH, original.as_bytes())?;
    assert!(dwm_settings::load_config().valid);
    dunit::println!("[SDKCAT] invalid candidate rejected and recovery OK")?;
    Ok(())
}
