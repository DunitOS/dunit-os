//! Installed application manifests. TOML parsing works with core + alloc only.
#![no_std]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use serde::Deserialize;
include!(concat!(env!("OUT_DIR"), "/rights.rs"));

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Cli,
    Tui,
    Gui,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub binary: String,
    pub icon: Option<String>,
    pub capabilities: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    application: Application,
}

pub fn identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
}
pub fn parse(text: &str) -> Result<Application, String> {
    if text.len() > 64 * 1024 {
        return Err("manifest exceeds 64 KiB".into());
    }
    let document: Document = toml::from_str(text).map_err(|e| format!("{}", e))?;
    if document.schema_version != 1 {
        return Err("schema_version: expected version 1".into());
    }
    let app = document.application;
    if !identifier(&app.id) {
        return Err("application.id: invalid identifier".into());
    }
    if app.name.trim().is_empty() || app.name.chars().any(char::is_control) {
        return Err("application.name: empty name or control character".into());
    }
    if app.binary.is_empty()
        || app.binary == "."
        || app.binary == ".."
        || !app
            .binary
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
    {
        return Err("application.binary: invalid executable basename".into());
    }
    if let Some(icon) = &app.icon {
        if !icon.starts_with('/')
            || icon == "/"
            || icon.chars().any(char::is_control)
            || icon[1..]
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err("application.icon: expected normalized absolute VFS path".into());
        }
    }
    if app.kind == Kind::Gui && app.icon.is_none() {
        return Err("application.icon: required for GUI".into());
    }
    for (index, cap) in app.capabilities.iter().enumerate() {
        if !RIGHTS.contains(&cap.as_str()) || app.capabilities[..index].contains(cap) {
            return Err("application.capabilities: unknown or duplicate native right".into());
        }
    }
    Ok(app)
}
