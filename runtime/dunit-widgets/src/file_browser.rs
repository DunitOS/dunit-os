//! Shared filesystem-neutral browser data and path operations.
use alloc::{string::String, vec::Vec};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Directory,
    Device,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub kind: FileKind,
    pub size: Option<u64>,
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name
            .chars()
            .any(|c| c == '/' || c == '\0' || c.is_control())
}
pub fn child_path(base: &str, name: &str) -> String {
    let mut path = String::from(base);
    if !path.ends_with('/') {
        path.push('/');
    }
    path.push_str(name);
    path
}
pub fn normalize(path: &str, cwd: &str) -> Option<String> {
    if path.contains('\0') || cwd.contains('\0') || !cwd.starts_with('/') {
        return None;
    }
    let joined = if path.starts_with('/') {
        String::from(path)
    } else {
        child_path(cwd, path)
    };
    let mut parts = Vec::new();
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => {
                if p.chars().any(char::is_control) {
                    return None;
                }
                parts.push(p);
            }
        }
    }
    let mut result = String::from("/");
    result.push_str(&parts.join("/"));
    Some(result)
}
pub fn parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => String::from("/"),
        Some(i) => String::from(&trimmed[..i]),
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFilter {
    pub key: String,
    pub label: String,
    pub extensions: Vec<String>,
}
impl FileFilter {
    pub fn new(key: impl Into<String>, label: impl Into<String>, extensions: Vec<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            extensions,
        }
    }
    pub fn matches(&self, entry: &FileEntry) -> bool {
        entry.kind == FileKind::Directory
            || self.extensions.is_empty()
            || entry
                .name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| self.extensions.iter().any(|v| v.eq_ignore_ascii_case(ext)))
    }
}
