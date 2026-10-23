//! Checked settings edits shared by terminal setup and graphical theme import.
use alloc::format;
use crate::Settings;

/// Enumerate the serializer's actual fields, avoiding a second settings schema.
pub fn value<'a>(document: &'a str, field: &str) -> Option<&'a str> {
    let (wanted_section, wanted_key) = field.split_once('.')?;
    let mut section = "";
    for line in document.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = s;
        } else if section == wanted_section {
            if let Some((key, v)) = line.split_once('=') {
                if key.trim() == wanted_key {
                    return Some(v.trim());
                }
            }
        }
    }
    None
}

fn scalar(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

pub fn edit(current: &Settings, field: &str, input: &str) -> Result<Settings, &'static str> {
    if field.starts_with("meta.") {
        return Err("schema metadata is read-only");
    }
    if input.is_empty() || input.chars().any(|c| c.is_control()) {
        return Err("enter a non-empty, single-line TOML value");
    }
    let document = crate::to_toml(current);
    let old = value(&document, field).ok_or("unknown field; use show to list supported fields")?;
    // Strings use the same TOML representation displayed by show/get.
    if old.starts_with('"') && !(input.starts_with('"') && input.ends_with('"')) {
        return Err("string/color values must be enclosed in double quotes");
    }
    let (section, key) = field.split_once('.').ok_or("expected section.key")?;
    let patch = format!("[{}]\n{} = {}\n", section, key, input);
    let mut candidate = *current;
    crate::parse_into(&patch, &mut candidate);
    let encoded = crate::to_toml(&candidate);
    let actual = value(&encoded, field).ok_or("field is not serializable")?;
    let requested = scalar(input);
    let resolved = scalar(actual);
    // Reject silently ignored or clamped input. Canonical colors are case-insensitive.
    let matches = if requested.starts_with('#') {
        requested.eq_ignore_ascii_case(resolved)
            || (requested.len() == 7
                && resolved.len() == 9
                && resolved[1..3].eq_ignore_ascii_case("FF")
                && requested[1..].eq_ignore_ascii_case(&resolved[3..]))
    } else {
        requested == resolved
    };
    if !matches {
        return Err("invalid value or out of range; value was not changed");
    }
    if !crate::roundtrip_ok(&candidate) {
        return Err("settings failed serialization validation");
    }
    Ok(candidate)
}


/// Validate every supplied setting before returning a candidate. Unknown,
/// malformed and clamped values reject the entire overlay.
pub fn apply_overlay(current: &Settings, text: &str) -> Result<Settings, &'static str> {
    let mut candidate = *current;
    let mut section = "";
    let mut count = 0usize;
    for raw in text.lines() {
        // Match the shared TOML subset: # and // comments outside double quotes.
        let mut quoted = false;
        let bytes = raw.as_bytes();
        let mut end = bytes.len();
        for (i, byte) in bytes.iter().enumerate() {
            if *byte == b'"' {
                quoted = !quoted;
            }
            if !quoted
                && (*byte == b'#' || (*byte == b'/' && bytes.get(i + 1) == Some(&b'/')))
            {
                end = i;
                break;
            }
        }
        if quoted {
            return Err("overlay contains an unterminated string");
        }
        let line = raw[..end].trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim();
            continue;
        }
        let (key, input) = line
            .split_once('=')
            .ok_or("overlay contains a malformed line")?;
        let field = format!("{}.{}", section, key.trim());
        if field == "meta.schema_version" {
            let version = input
                .trim()
                .parse::<u32>()
                .map_err(|_| "invalid overlay schema")?;
            if !crate::version_supported(version) {
                return Err("unsupported overlay schema");
            }
            continue; // overlays cannot replace the running schema metadata
        }
        candidate = edit(&candidate, &field, input.trim())?;
        count += 1;
    }
    if count == 0 {
        return Err("overlay contains no supported settings");
    }
    Ok(candidate)
}

/// Bounded, checked I/O through the same reader as the desktop config.
pub fn load_overlay(current: &Settings, path: &str) -> Result<Settings, &'static str> {
    if !path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err("overlay path must be absolute without parent traversal");
    }
    let text = crate::read_file_checked(path).map_err(|_| "could not read complete UTF-8 overlay (64 KiB maximum)")?;
    apply_overlay(current, &text)
}
