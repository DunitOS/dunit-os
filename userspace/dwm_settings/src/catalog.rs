//! Installed metadata is read at startup/reload; desktop composition stays user policy.
use crate::{AppEntry, Applications, ConfigError, MAX_APPS};
use alloc::{format, string::String, vec, vec::Vec};
use dunit_app_manifest::{Application, Kind};
const ROOT: &str = "/system/share/dunit/apps";
const MAX_CATALOG: usize = 256;
fn error(path: &str, reason: &'static str) -> ConfigError {
    ConfigError {
        line: 0,
        field: String::from(path),
        reason,
    }
}
pub(crate) fn entries(manifests: &[Application]) -> Result<Applications, ConfigError> {
    let mut result = Applications::baseline();
    for (index, app) in manifests.iter().enumerate() {
        if manifests[..index].iter().any(|other| other.id == app.id) {
            return Err(error(&app.id, "duplicate installed application id"));
        }
        if app.kind != Kind::Gui {
            continue;
        }
        if result.apps.len() == MAX_APPS {
            return Err(error(ROOT, "too many desktop applications"));
        }
        result.apps.push(AppEntry {
            id: app.id.clone(),
            name: app.name.clone(),
            exec: format!("/app/{}", app.binary),
            icon: app.icon.clone().unwrap_or_default(),
            label: String::new(),
        });
    }
    result.dock = (0..result.apps.len()).collect();
    result.launcher = result.dock.clone();
    Ok(result)
}
pub(crate) fn load() -> Result<Applications, ConfigError> {
    let mut directories = vec![libdunit::DirEntry::empty(); MAX_CATALOG + 1];
    let count = libdunit::readdir(ROOT, &mut directories);
    if count == libdunit::ENOENT {
        return Ok(Applications::baseline());
    }
    if count < 0 {
        return Err(error(ROOT, "could not read application catalog"));
    }
    if count as usize > MAX_CATALOG {
        return Err(error(ROOT, "application catalog exceeds limit"));
    }
    directories.truncate(count as usize);
    directories.sort_by(|a, b| a.name().cmp(b.name()));
    let mut manifests = Vec::new();
    for directory in directories {
        let id = directory.name();
        if directory.file_type != libdunit::FILE_TYPE_DIRECTORY
            || !dunit_app_manifest::identifier(id)
        {
            return Err(error(ROOT, "invalid application catalog directory"));
        }
        let path = format!("{}/{}/dunit.toml", ROOT, id);
        let text = crate::read_file_checked(&path)
            .map_err(|_| error(&path, "could not read complete application manifest"))?;
        let app = dunit_app_manifest::parse(&text).map_err(|reason| {
            libdunit::println(&format!(
                "dwm_settings: manifest={} reason={}",
                path, reason
            ));
            error(&path, "invalid installed application manifest")
        })?;
        if app.id != id {
            return Err(error(&path, "manifest id differs from directory"));
        }
        let mut stat = libdunit::FileStat::default();
        if libdunit::stat(&format!("/app/{}", app.binary), &mut stat) < 0
            || stat.file_type != libdunit::FILE_TYPE_FILE
        {
            return Err(error(&path, "manifest executable is unavailable"));
        }
        manifests.push(app);
    }
    entries(&manifests)
}

/// Pure resolver used by host tests and tools which supply installed manifests.
pub fn validate_document_with_manifests(
    text: &str,
    manifests: &[Application],
) -> Result<crate::Config, ConfigError> {
    let catalog = entries(manifests)?;
    crate::validate_seeded_document(text, &catalog)
}
/// Resolve an editable full desktop document against the installed catalog.
pub fn validate_installed_document(text: &str) -> Result<crate::Config, ConfigError> {
    crate::validation::validate_syntax(text)?;
    crate::validate_seeded_document(text, &load()?)
}
