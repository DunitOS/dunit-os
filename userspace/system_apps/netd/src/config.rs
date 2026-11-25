//! Per-interface configuration parsing (`/system/boot-services/netd-nics.conf`
//! and legacy `/system/boot-services/netd-ipv4.conf`).
use alloc::vec::Vec;
use dunit::{errno, Error, Result};
use dunit_net_core::{ipv6, Ip6};

use crate::interface::Config as V4Config;

const MULTI_CONFIG_PATH: &str = "/system/boot-services/netd-nics.conf";
const LEGACY_V4_CONFIG_PATH: &str = "/system/boot-services/netd-ipv4.conf";

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
    let mut specs: Vec<NicSpec> = (0..count).map(NicSpec::default_for).collect();
    match dunit::fs::read_to_string(MULTI_CONFIG_PATH) {
        Ok(text) => {
            parse_multi_config(&text, &mut specs).ok_or(Error(errno::EINVAL))?;
            return Ok(specs);
        }
        Err(Error(errno::ENOENT)) => {}
        Err(e) => return Err(e),
    }

    match dunit::fs::read_to_string(LEGACY_V4_CONFIG_PATH) {
        Ok(text) => {
            let v4 = V4Config::static_address(&text).ok_or(Error(errno::EINVAL))?;
            if let Some(first) = specs.get_mut(0) {
                first.v4 = V4ModeSpec::Static(v4);
            }
        }
        Err(Error(errno::ENOENT)) => {}
        Err(e) => return Err(e),
    }
    Ok(specs)
}

#[derive(Default)]
struct RawSection {
    metric: Option<u32>,
    dad_probes: Option<u8>,
    ipv4_mode: Option<V4ModeSpec>,
    gateway4: Option<[u8; 4]>,
    dns4: Vec<[u8; 4]>,
    ipv6_mode: Option<(bool, Option<(Ip6, u8)>)>, // (slaac_enabled, static_cidr)
    gateway6: Option<Ip6>,
    dns6: Vec<Ip6>,
}

fn parse_multi_config(text: &str, specs: &mut [NicSpec]) -> Option<()> {
    if text.len() > 2048 {
        return None;
    }
    let mut current: Option<usize> = None;
    let mut raw: [Option<RawSection>; 4] = [None, None, None, None];

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
        let idx = current?;
        let sec = raw[idx].as_mut()?;
        let (key, val) = line.split_once('=')?;
        let key = key.trim();
        let val = val.trim();
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
    Some(())
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

