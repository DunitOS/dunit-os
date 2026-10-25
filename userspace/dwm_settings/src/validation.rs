//! Validate the supported document before a loader mutates live settings.
use crate::{Settings, MAX_APPS, MAX_WS};
use alloc::{format, string::String, vec::Vec};

#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub line: usize,
    pub field: String,
    pub reason: &'static str,
}

pub(crate) fn locate(text:&str,mut error:ConfigError)->ConfigError{
    if error.line!=0{return error;}
    let (wanted,key)=error.field.rsplit_once('.').unwrap_or((&error.field,""));
    let mut section="";let mut fallback=1;
    for(index,raw)in text.lines().enumerate(){let line=crate::strip_comment(raw).trim();
        if let Some(name)=line.strip_prefix('[').and_then(|s|s.strip_suffix(']')){section=name.trim();if section==error.field{error.line=index+1;return error;}if section==wanted{fallback=index+1;}}
        else if section==wanted&&line.split_once('=').is_some_and(|(name,_)|name.trim()==key){error.line=index+1;return error;}
    }
    error.line=fallback;error
}

fn string(input: &str) -> Result<&str, &'static str> {
    let text = input
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or("expected a quoted string")?;
    if text.contains('"') || text.chars().any(char::is_control) {
        return Err("unsupported string escape or control character");
    }
    Ok(text)
}

pub(crate) fn list(input: &str) -> Result<Vec<&str>, &'static str> {
    let text = input
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or("expected a single-line string array")?
        .trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for part in text.split(',') {
        let id = string(part.trim())?;
        if id.is_empty() {
            return Err("application id must not be empty");
        }
        ids.push(id);
    }
    Ok(ids)
}

/// The serializer defines the scalar schema; edit() verifies values by a
/// canonical round trip. Registry data is validated separately, before its
/// tolerant low-level parser can clamp or silently drop it.
pub fn validate(text: &str) -> Result<(), ConfigError> {
    if text.len() > 64 * 1024 { return Err(ConfigError { line: 0, field: String::new(), reason: "configuration exceeds 64 KiB" }); }
    let baseline = Settings::defaults();
    let schema = crate::to_toml(&baseline);
    let mut section = "";
    let mut sections = Vec::new();
    let mut fields = Vec::new();
    let mut applications = Vec::new();
    let mut references = Vec::new();
    let mut supplied = false;
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = crate::strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let error = |field: String, reason| ConfigError {
            line: line_no,
            field,
            reason,
        };
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim();
            if sections.contains(&section) {
                return Err(error(section.into(), "duplicate section"));
            }
            sections.push(section);
            if let Some(id) = section.strip_prefix("application.") {
                if id.is_empty()
                    || id
                        .chars()
                        .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                {
                    return Err(error(section.into(), "invalid application id"));
                }
                applications.push(id);
                if applications.len() > MAX_APPS {
                    return Err(error(section.into(), "too many applications"));
                }
            } else if !crate::shell::section(section) && !crate::policy::section(section) && section != "shell" && !matches!(section, "dock" | "launcher" | "startup" | "workspaces")
                && !schema.lines().any(|s| s == format!("[{}]", section))
            {
                return Err(error(section.into(), "unknown section"));
            }
            continue;
        }
        let (key, input) = line
            .split_once('=')
            .ok_or_else(|| error(section.into(), "expected key = value"))?;
        let key = key.trim();
        let input = input.trim();
        let field = format!("{}.{}", section, key);
        if fields.contains(&field) {
            return Err(error(field, "duplicate key"));
        }
        fields.push(field.clone());
        let checked = if crate::shell::section(section) || crate::policy::section(section) || section == "shell" {
            Ok(()) // Typed shell parsing and complete tree validation below.
        } else if section.starts_with("application.") {
            if matches!(key, "name" | "exec" | "icon" | "label") {
                string(input).map(|_| ())
            } else {
                Err("unknown application field")
            }
        } else if matches!(section, "dock" | "launcher" | "startup") && key == "entries" {
            match list(input) {
                Ok(ids) => {
                    references.push((line_no, field.clone(), ids));
                    Ok(())
                }
                Err(reason) => Err(reason),
            }
        } else if section == "startup" && key == "self_test" {
            crate::parse_bool(input)
                .map(|_| ())
                .ok_or("expected a boolean")
        } else if section == "workspaces" && matches!(key, "count" | "n") {
            crate::parse_uint(input)
                .filter(|n| *n >= 1 && *n as usize <= MAX_WS)
                .map(|_| ())
                .ok_or("workspace count out of range")
        } else if section == "scripts" && matches!(key, "runner" | "layout" | "events") {
            config_string(input).and_then(|path| {
                if (path.is_empty() && key == "events") || (path.starts_with('/') && !path.split('/').any(|part| part == "..")) { Ok(()) }
                else { Err("script path must be absolute without traversal") }
            })
        } else if field == "meta.schema_version" {
            input
                .parse::<u32>()
                .ok()
                .filter(|v| crate::version_supported(*v))
                .map(|_| ())
                .ok_or("unsupported schema version")
        } else {
            crate::edit(&baseline, &field, input).map(|_| ())
        };
        checked.map_err(|reason| error(field, reason))?;
        supplied = true;
    }
    if !supplied && applications.is_empty() {
        return Err(ConfigError {
            line: 0,
            field: String::new(),
            reason: "empty configuration",
        });
    }
    // A document without a registry retains the documented baseline registry.
    let defaults = crate::Applications::baseline();
    for (line, field, ids) in references {
        for id in ids {
            let exists = if applications.is_empty() {
                defaults.apps.iter().any(|a| a.id == id)
            } else {
                applications.contains(&id)
            };
            if !exists {
                return Err(ConfigError {
                    line,
                    field,
                    reason: "unknown application reference",
                });
            }
        }
    }
    let mut settings = Settings::defaults();
    crate::parse_into(text, &mut settings);
    let (apps, _) = crate::parse_apps(text);
    crate::shell::resolve(text, &settings, &apps).map_err(|error|locate(text,error))?;
    crate::policy::resolve(text, &apps).map_err(|error|locate(text,error))?;
    Ok(())
}

/// Validate one standalone app/widget table and apply fields to a local
/// candidate. The caller commits the candidate only after this succeeds.
pub(crate) fn table(mut text: &str, wanted: &str,
    mut apply: impl FnMut(&str, &str) -> Result<(), &'static str>) -> Result<(), ConfigError> {
    if text.len() > 64 * 1024 {
        return Err(ConfigError { line: 0, field: wanted.into(), reason: "configuration exceeds 64 KiB" });
    }
    text = text.trim_end();
    let mut seen_section = false;
    let mut keys = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = crate::strip_comment(raw).trim();
        if line.is_empty() { continue; }
        let error = |field: String, reason| ConfigError { line: index + 1, field, reason };
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if section.trim() != wanted { return Err(error(section.into(), "unknown section")); }
            if seen_section { return Err(error(section.into(), "duplicate section")); }
            seen_section = true;
            continue;
        }
        let (key, input) = line.split_once('=').ok_or_else(|| error(wanted.into(), "expected key = value"))?;
        let key = key.trim();
        let field = format!("{}.{}", wanted, key);
        if !seen_section { return Err(error(field, "setting outside its table")); }
        if keys.contains(&key) { return Err(error(field, "duplicate key")); }
        keys.push(key);
        apply(key, input.trim()).map_err(|reason| error(field, reason))?;
    }
    if keys.is_empty() {
        return Err(ConfigError { line: 0, field: wanted.into(), reason: "configuration contains no settings" });
    }
    Ok(())
}

pub(crate) fn config_string(input: &str) -> Result<&str, &'static str> {
    let value = string(input)?;
    if value.len() > crate::ConfigStr::CAP { return Err("string exceeds 128 bytes"); }
    Ok(value)
}

pub(crate) fn color(input: &str) -> Result<u32, &'static str> {
    crate::parse_color(string(input)?).ok_or("expected #RRGGBB or #AARRGGBB color")
}

pub(crate) fn uint(input: &str, maximum: u32) -> Result<u32, &'static str> {
    crate::parse_uint(input).filter(|n| *n <= maximum).ok_or("integer outside accepted range")
}

pub(crate) fn colors(input: &str) -> Result<[u32; 8], &'static str> {
    let values = list(input)?;
    if values.len() != 8 { return Err("ANSI palette must contain exactly eight colors"); }
    let mut result = [0; 8];
    for (slot, text) in result.iter_mut().zip(values) {
        *slot = crate::parse_color(text).ok_or("invalid ANSI color")?;
    }
    Ok(result)
}
