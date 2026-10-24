use crate::{safe_path, Error, Result};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
/// Config manifests deliberately accept string-valued TOML tables only.
/// Unknown/duplicate keys are handled by callers, never partially applied.
pub fn strings(source: &str) -> Result<Vec<(String, String, String)>> {
    let mut section = "";
    let mut out = Vec::new();
    for line in source
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        let (k, v) = line.split_once('=').ok_or(Error::Invalid)?;
        let k = k.trim();
        let v = v.trim();
        if !v.starts_with('"') || !v.ends_with('"') || v.len() < 2 || k.is_empty() {
            return Err(Error::Invalid);
        }
        let v = &v[1..v.len() - 1];
        if v.contains(['"', '\\', '\0']) || out.iter().any(|(s, key, _)| s == section && key == k) {
            return Err(Error::Invalid);
        }
        out.push((section.to_string(), k.to_string(), v.to_string()));
        if out.len() > 256 {
            return Err(Error::Limit);
        }
    }
    Ok(out)
}
#[derive(Default)]
struct Directory {
    name: String,
    size: u32,
    scale: u32,
    min: u32,
    max: u32,
    threshold: u32,
    kind: String,
}
/// Source order breaks equal-distance ties, as in the freedesktop specification.
pub struct Index {
    dirs: Vec<Directory>,
}
impl Index {
    pub fn parse(source: &str) -> Result<Self> {
        let mut dirs = Vec::new();
        let mut sections = Vec::new();
        let mut section = "";
        for line in source
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
        {
            if line.starts_with('[') && line.ends_with(']') {
                section = &line[1..line.len() - 1];
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                return Err(Error::Invalid);
            };
            sections.push((section, k.trim(), v.trim()));
        }
        for (_, _, list) in sections.iter().filter(|(s, k, _)| {
            *s == "Icon Theme" && matches!(*k, "Directories" | "ScaledDirectories")
        }) {
            for name in list.split(',').filter(|s| !s.is_empty()) {
                if !safe_path(name) || dirs.len() >= 1024 {
                    return Err(Error::Invalid);
                }
                let get = |key| {
                    sections
                        .iter()
                        .find(|(s, k, _)| *s == name && *k == key)
                        .map(|(_, _, v)| *v)
                };
                let num = |key, default| -> Result<u32> {
                    get(key).map_or(Ok(default), |v| v.parse().map_err(|_| Error::Invalid))
                };
                let size = num("Size", 0)?;
                let scale = num("Scale", 1)?;
                let min = num("MinSize", size)?;
                let max = num("MaxSize", size)?;
                let threshold = num("Threshold", 2)?;
                let kind = get("Type").unwrap_or("Threshold");
                if size == 0
                    || size > 8192
                    || scale == 0
                    || scale > 16
                    || min > max
                    || max > 8192
                    || threshold > 8192
                    || !matches!(kind, "Fixed" | "Scalable" | "Threshold")
                {
                    return Err(Error::Invalid);
                }
                dirs.push(Directory {
                    name: name.to_string(),
                    size,
                    scale,
                    min,
                    max,
                    threshold,
                    kind: kind.to_string(),
                });
            }
        }
        if dirs.is_empty() {
            return Err(Error::Invalid);
        }
        Ok(Self { dirs })
    }
    pub fn candidates(&self, side: u32) -> Vec<&str> {
        let mut dirs: Vec<_> = self.dirs.iter().collect();
        dirs.sort_by_key(|d| {
            let (lo, hi) = match d.kind.as_str() {
                "Fixed" => (d.size, d.size),
                "Scalable" => (d.min, d.max),
                _ => (d.size.saturating_sub(d.threshold), d.size + d.threshold),
            };
            let (lo, hi) = (lo * d.scale, hi * d.scale);
            lo.saturating_sub(side).max(side.saturating_sub(hi))
        });
        dirs.iter().map(|d| d.name.as_str()).collect()
    }
}
