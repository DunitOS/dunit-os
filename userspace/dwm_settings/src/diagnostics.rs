//! Explicitly enabled technical diagnostics; absent configuration stays silent.
use crate::{strip_comment, ConfigError};
use alloc::{format, string::String};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub performance: bool,
}
impl Diagnostics {
    pub(crate) fn field(key: &str, value: &str) -> Result<bool, &'static str> {
        if key != "performance" {
            return Err("unknown diagnostics field");
        }
        crate::parse_bool(value).ok_or("expected a boolean")
    }
    pub(crate) fn resolve(text: &str) -> Result<Self, ConfigError> {
        let mut result = Self::default();
        let mut section = "";
        for (index, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = name.trim();
            } else if section == "diagnostics" && !line.is_empty() {
                let (key, value) = line.split_once('=').ok_or_else(|| ConfigError {
                    line: index + 1,
                    field: String::from("diagnostics"),
                    reason: "expected key = value",
                })?;
                result.performance =
                    Self::field(key.trim(), value.trim()).map_err(|reason| ConfigError {
                        line: index + 1,
                        field: format!("diagnostics.{}", key.trim()),
                        reason,
                    })?;
            }
        }
        Ok(result)
    }
}
