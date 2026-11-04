//! Sparse scalar edits preserve DSS inheritance and independent desktop data.
use crate::Settings;
use alloc::string::String;

/// A staged snapshot determines intent; current unrelated source changes survive.
pub fn scalar_changes(before: &Settings, after: &Settings) -> Result<String, &'static str> {
    if !crate::version_supported(after.meta.schema_version) {
        return Err("unsupported settings schema");
    }
    if before.meta.schema_version != after.meta.schema_version {
        return Err("schema metadata is read-only");
    }
    if !crate::roundtrip_ok(after) {
        return Err("settings failed serialization validation");
    }
    Ok(crate::preserve::changes(
        &crate::to_toml(before),
        &crate::to_toml(after),
    ))
}

/// Apply checked scalar fields to source without expanding omitted settings.
/// This pure function checks the overlay; save APIs additionally validate the
/// complete source/result against installed manifests before writing.
pub fn apply_scalar_overlay(document: &str, overlay: &str) -> Result<String, &'static str> {
    if document.len() > 64 * 1024 || overlay.len() > 64 * 1024 {
        return Err("desktop document exceeds limit");
    }
    let mut values = Settings::defaults();
    crate::parse_into(document, &mut values);
    let patch = crate::normalize_overlay(&values, overlay)?;
    let result = crate::preserve::patch(document, &patch);
    if result.len() > 64 * 1024 {
        return Err("desktop document exceeds limit");
    }
    Ok(result)
}

pub(super) fn save_to(source: &str, patch: &str) -> bool {
    if !crate::validate_installed_document(source).is_ok_and(|config| config.valid) {
        return false;
    }
    if patch.is_empty() {
        return true;
    }
    let Ok(result) = apply_scalar_overlay(source, patch) else {
        return false;
    };
    crate::save_document(&result).is_ok()
}

/// Persist only fields the caller changed, not a complete Settings snapshot.
/// Writes to the selected fields retain existing last-writer-wins semantics;
/// unrelated changes made since the snapshot are retained.
pub fn save_changes(before: &Settings, after: &Settings) -> bool {
    let Ok(patch) = scalar_changes(before, after) else {
        return false;
    };
    let Ok(source) = crate::load_document() else {
        return false;
    };
    save_to(&source, &patch)
}

/// Save explicitly declared fields, even when equal to a fallback/default.
/// This is the writer for staged CLI commands and partial theme imports.
pub fn save_overlay(overlay: &str) -> bool {
    let Ok(source) = crate::load_document() else {
        return false;
    };
    if overlay.is_empty() {
        return false;
    }
    save_to(&source, overlay)
}
