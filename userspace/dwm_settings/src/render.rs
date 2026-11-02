//! Validated compositor cadence; missing configuration keeps the legacy target.
use crate::{strip_comment, ConfigError};
use alloc::{format, string::String};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Render {
    pub frame_interval_ms: u32,
}
impl Default for Render {
    fn default() -> Self {
        Self {
            frame_interval_ms: 16,
        }
    }
}
impl Render {
    pub(crate) fn field(key: &str, value: &str) -> Result<u32, &'static str> {
        if key != "frame_interval_ms" {
            return Err("unknown render field");
        }
        let interval = value
            .parse::<u32>()
            .map_err(|_| "expected integer milliseconds")?;
        (1..=1000)
            .contains(&interval)
            .then_some(interval)
            .ok_or("frame interval must be 1..1000 ms")
    }
    pub(crate) fn resolve(text: &str) -> Result<Self, ConfigError> {
        let mut result = Self::default();
        let mut section = "";
        for (index, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = name.trim();
            } else if section == "render" && !line.is_empty() {
                let (key, value) = line.split_once('=').ok_or_else(|| ConfigError {
                    line: index + 1,
                    field: String::from("render"),
                    reason: "expected key = value",
                })?;
                result.frame_interval_ms =
                    Self::field(key.trim(), value.trim()).map_err(|reason| ConfigError {
                        line: index + 1,
                        field: format!("render.{}", key.trim()),
                        reason,
                    })?;
            }
        }
        Ok(result)
    }
}
