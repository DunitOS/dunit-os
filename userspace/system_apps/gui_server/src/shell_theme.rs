//! Shared SDK theme preparation. No theme parsing or file I/O in paint paths.
use super::*;
use alloc::string::String;

fn read(path: &str) -> Result<String, String> {
    let bytes = libdunit::read_binary(path, 64 * 1024)
        .ok_or_else(|| alloc::format!("{}: cannot read theme", path))?;
    String::from_utf8(bytes).map_err(|_| alloc::format!("{}: theme must be UTF-8", path))
}

pub fn prepare(config: &settings::Config) -> Result<dunit_component::Theme, String> {
    let profile =
        settings::appearance::profile(&read(config.settings.desktop.sdk_theme.as_str())?, config);
    dunit_component::Theme::parse_shell(
        &profile,
        &read(settings::appearance::SHELL_TOKENS)?,
        &read(settings::appearance::SDK_BASE)?,
        "",
    )
    .map_err(|e| alloc::format!("invalid shared theme: {:?}", e))
}

#[path = "shell_palette.rs"]
mod palette;
pub use palette::palette;
