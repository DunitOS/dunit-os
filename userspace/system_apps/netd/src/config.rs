//! Versioned network configuration (`version = 1`), staging, atomic apply, and rollback.
use alloc::{format, string::String, vec::Vec};
use dunit::{errno, fs, Error, Result};
use dunit_net_core::{ipv6, Ip6};

use crate::interface::Config as V4Config;

pub const DEFAULT_CONFIG_PATH: &str = "/system/share/network/defaults.toml";
pub const MULTI_CONFIG_PATH: &str = "/system/boot-services/netd-nics.conf";
pub const LEGACY_V4_CONFIG_PATH: &str = "/system/boot-services/netd-ipv4.conf";
pub const PERSIST_CONFIG_PATH: &str = "/persist/config/network.conf";
pub const STAGED_CONFIG_PATH: &str = "/run/netd/staged.conf";
pub const LAST_GOOD_CONFIG_PATH: &str = "/var/lib/netd/last-good.conf";
pub const EVENTS_LOG_PATH: &str = "/var/log/netd/events.log";
pub const SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: usize = 2048;
const MAX_EVENT_LOG_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V4ModeSpec {
    Dhcp,
    Static(V4Config),
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum V6ModeSpec {
    Slaac,
    Static {
        address: Ip6,
        prefix_len: u8,
        gateway: Option<Ip6>,
        dns: Vec<Ip6>,
    },
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NicSpec {
    pub metric: u32,
    pub dad_probes: u8,
    pub v4: V4ModeSpec,
    pub v6: V6ModeSpec,
}

impl NicSpec {
    pub fn default_for(index: usize) -> Self {
        Self {
            metric: ((index as u32) + 1) * 10,
            dad_probes: 1,
            v4: V4ModeSpec::Dhcp,
            v6: V6ModeSpec::Disabled,
        }
    }
}

pub fn load_nic_specs(count: usize) -> Result<Vec<NicSpec>> {
    ensure_runtime_dirs();

    // Candidate order: staged override -> persistent config -> boot service multi-NIC -> legacy IPv4 -> defaults.toml.
    for (label, path, require_version) in [
        ("staged", STAGED_CONFIG_PATH, true),
        ("persistent", PERSIST_CONFIG_PATH, true),
        ("boot-nics", MULTI_CONFIG_PATH, false),
    ] {
        match fs::read_to_string(path) {
            Ok(text) => {
                let mut specs: Vec<NicSpec> = (0..count).map(NicSpec::default_for).collect();
                if parse_multi_config(&text, &mut specs, require_version).is_some() {
                    let _ = record_last_good(&text);
                    append_event(&format!("applied {} config ({})", label, path));
                    return Ok(specs);
                }
                append_event(&format!("rejected invalid {} config ({}); rolling back", label, path));
                dunit::println!("[NETD] invalid {} config; rolling back", label)?;
                if label == "staged" {
                    let _ = fs::remove_file(STAGED_CONFIG_PATH);
                }
                if let Some(specs) = try_rollback(count) {
                    dunit::println!("[NETD] config rolled back")?;
                    return Ok(specs);
                }
            }
            Err(Error(errno::ENOENT)) => {}
            Err(e) => return Err(e),
        }
    }

    let mut specs: Vec<NicSpec> = (0..count).map(NicSpec::default_for).collect();
    match fs::read_to_string(LEGACY_V4_CONFIG_PATH) {
        Ok(text) => {
            let v4 = V4Config::static_address(&text).ok_or(Error(errno::EINVAL))?;
            if let Some(first) = specs.get_mut(0) {
                first.v4 = V4ModeSpec::Static(v4);
            }
            append_event("applied legacy netd-ipv4.conf");
            return Ok(specs);
        }
        Err(Error(errno::ENOENT)) => {}
        Err(e) => return Err(e),
    }

    if let Ok(text) = fs::read_to_string(DEFAULT_CONFIG_PATH) {
        if parse_multi_config(&text, &mut specs, true).is_some() {
            let _ = record_last_good(&text);
            append_event("applied /system/share/network/defaults.toml");
            return Ok(specs);
        }
        append_event("ignored invalid defaults.toml");
    }

    Ok(specs)
}

fn ensure_runtime_dirs() {
    for dir in ["/run", "/run/netd", "/var", "/var/lib", "/var/lib/netd", "/var/log", "/var/log/netd"] {
        let _ = fs::create_dir(dir);
    }
}

fn try_rollback(count: usize) -> Option<Vec<NicSpec>> {
    for (source, require_version) in [
        (LAST_GOOD_CONFIG_PATH, false),
        (MULTI_CONFIG_PATH, false),
        (DEFAULT_CONFIG_PATH, true),
    ] {
        if let Ok(text) = fs::read_to_string(source) {
            let mut specs: Vec<NicSpec> = (0..count).map(NicSpec::default_for).collect();
            if parse_multi_config(&text, &mut specs, require_version).is_some() {
                if source != LAST_GOOD_CONFIG_PATH {
                    let _ = record_last_good(&text);
                }
                append_event(&format!("rolled back to {}", source));
                return Some(specs);
            }
        }
    }
    None
}

fn record_last_good(text: &str) -> Result<()> {
    let tmp = "/var/lib/netd/last-good.tmp";
    let _ = fs::remove_file(tmp);
    fs::write(tmp, text.as_bytes())?;
    let _ = fs::remove_file(LAST_GOOD_CONFIG_PATH);
    fs::rename(tmp, LAST_GOOD_CONFIG_PATH)
}

pub fn append_event(msg: &str) {
    let mut existing = fs::read_to_string(EVENTS_LOG_PATH).unwrap_or_default();
    existing.push_str(msg);
    existing.push('\n');
    if existing.len() > MAX_EVENT_LOG_BYTES {
        let cut = existing.len() - MAX_EVENT_LOG_BYTES;
        let trimmed = existing[cut..]
            .find('\n')
            .map_or(&existing[cut..], |i| &existing[cut + i + 1..]);
        let owned = String::from(trimmed);
        existing = owned;
    }
    let _ = fs::write(EVENTS_LOG_PATH, existing.as_bytes());
}

#[derive(Default)]
struct RawSection {
    metric: Option<u32>,
    dad_probes: Option<u8>,
    ipv4_mode: Option<V4ModeSpec>,
    gateway4: Option<[u8; 4]>,
    dns4: Vec<[u8; 4]>,
    ipv6_mode: Option<(bool, Option<(Ip6, u8)>)>,
    gateway6: Option<Ip6>,
    dns6: Vec<Ip6>,
}

pub fn parse_multi_config(text: &str, specs: &mut [NicSpec], require_version: bool) -> Option<u32> {
    if text.len() > MAX_CONFIG_BYTES {
        return None;
    }
    let mut current: Option<usize> = None;
    let mut raw: [Option<RawSection>; 4] = [None, None, None, None];
    let mut version: Option<u32> = None;

    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let idx = section.strip_prefix("nic")?.parse::<usize>().ok()?;
            if idx >= 4 {
                return None;
            }
            if raw[idx].is_none() {
                raw[idx] = Some(RawSection::default());
            }
            current = Some(idx);
            continue;
        }
        let (key, val) = line.split_once('=')?;
        let key = key.trim();
        let val = val.trim().trim_matches('"');
        if current.is_none() {
            if key == "version" {
                let v = val.parse::<u32>().ok()?;
                if v != SCHEMA_VERSION {
                    return None;
                }
                version = Some(v);
                continue;
            }
            return None;
        }
        let idx = current?;
        let sec = raw[idx].as_mut()?;
        match key {
            "metric" => sec.metric = Some(val.parse().ok()?),
            "dad_probes" => sec.dad_probes = Some(val.parse().ok()?),
            "ipv4" => {
                sec.ipv4_mode = Some(match val {
                    "dhcp" => V4ModeSpec::Dhcp,
                    "none" => V4ModeSpec::Disabled,
                    _ => V4ModeSpec::Static(V4Config::static_address(val)?),
                });
            }
            "gateway4" => sec.gateway4 = Some(crate::interface::parse_ip(val)?),
            "dns4" => {
                for part in val.split(',').map(str::trim) {
                    sec.dns4.push(crate::interface::parse_ip(part)?);
                }
            }
            "ipv6" => {
                sec.ipv6_mode = Some(match val {
                    "slaac" => (true, None),
                    "none" => (false, None),
                    _ => {
                        let (addr_s, plen_s) = val.split_once('/')?;
                        let addr = parse_ipv6(addr_s.trim())?;
                        let plen = plen_s.trim().parse::<u8>().ok()?;
                        if !ipv6::valid_unicast(addr) || plen == 0 || plen > 128 {
                            return None;
                        }
                        (false, Some((addr, plen)))
                    }
                });
            }
            "gateway6" => sec.gateway6 = Some(parse_ipv6(val)?),
            "dns6" => {
                for part in val.split(',').map(str::trim) {
                    let ip = parse_ipv6(part)?;
                    if !ipv6::valid_unicast(ip) {
                        return None;
                    }
                    sec.dns6.push(ip);
                }
            }
            _ => return None,
        }
    }

    if require_version && version != Some(SCHEMA_VERSION) {
        return None;
    }

    for (idx, spec) in specs.iter_mut().enumerate() {
        let Some(sec) = raw.get_mut(idx).and_then(Option::take) else {
            continue;
        };
        if let Some(m) = sec.metric {
            spec.metric = m;
        }
        if let Some(d) = sec.dad_probes {
            spec.dad_probes = d;
        }
        if let Some(mut v4) = sec.ipv4_mode {
            if let V4ModeSpec::Static(cfg) = &mut v4 {
                if let Some(gw) = sec.gateway4 {
                    cfg.router = Some(gw);
                }
                if !sec.dns4.is_empty() {
                    cfg.dns[0] = sec.dns4.first().copied();
                    cfg.dns[1] = sec.dns4.get(1).copied();
                }
            }
            spec.v4 = v4;
        }
        if let Some((slaac, static_cidr)) = sec.ipv6_mode {
            spec.v6 = match (slaac, static_cidr) {
                (true, _) => V6ModeSpec::Slaac,
                (false, Some((address, prefix_len))) => V6ModeSpec::Static {
                    address,
                    prefix_len,
                    gateway: sec.gateway6,
                    dns: sec.dns6,
                },
                (false, None) => V6ModeSpec::Disabled,
            };
        }
    }
    Some(version.unwrap_or(SCHEMA_VERSION))
}

pub fn parse_ipv6(s: &str) -> Option<Ip6> {
    if s.is_empty() || s.len() > 45 {
        return None;
    }
    let (head_str, tail_str, compressed) = match s.split_once("::") {
        Some((h, t)) => {
            if t.contains("::") {
                return None;
            }
            (h, t, true)
        }
        None => (s, "", false),
    };
    let mut head = Vec::new();
    if !head_str.is_empty() {
        for g in head_str.split(':') {
            if g.is_empty() || g.len() > 4 {
                return None;
            }
            head.push(u16::from_str_radix(g, 16).ok()?);
        }
    }
    let mut tail = Vec::new();
    if !tail_str.is_empty() {
        for g in tail_str.split(':') {
            if g.is_empty() || g.len() > 4 {
                return None;
            }
            tail.push(u16::from_str_radix(g, 16).ok()?);
        }
    }
    if compressed {
        if head.len() + tail.len() >= 8 {
            return None;
        }
    } else if head.len() != 8 || !tail.is_empty() {
        return None;
    }
    let mut words = [0u16; 8];
    words[..head.len()].copy_from_slice(&head);
    let tail_start = 8 - tail.len();
    words[tail_start..].copy_from_slice(&tail);
    let mut out = [0u8; 16];
    for (i, w) in words.iter().enumerate() {
        let bytes = w.to_be_bytes();
        out[i * 2] = bytes[0];
        out[i * 2 + 1] = bytes[1];
    }
    Some(out)
}
