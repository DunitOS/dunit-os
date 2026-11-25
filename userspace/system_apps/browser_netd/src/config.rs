//! Sealed `/system/boot-services/browser-netd.conf` policy parser.
use alloc::{string::String, vec::Vec};
use dunit::fs;
use dunit_browser_net::policy::{BrowserPolicy, ProxyRoute};

pub const CONFIG_PATH: &str = "/system/boot-services/browser-netd.conf";

pub fn load_policy() -> BrowserPolicy {
    let mut policy = BrowserPolicy::default();
    let Ok(text) = fs::read_to_string(CONFIG_PATH) else {
        return policy;
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim().trim_matches('"');
        match key {
            "allow_http" => policy.origins.allow_http = val == "true" || val == "1",
            "allow_private_ips" => policy.origins.allow_private_ips = val == "true" || val == "1",
            "allow_cross_origin_redirect" => {
                policy.origins.allow_cross_origin_redirect = val == "true" || val == "1";
            }
            "allow_host" => push_csv(&mut policy.origins.allow_hosts, val),
            "deny_host" => push_csv(&mut policy.origins.deny_hosts, val),
            "proxy" => {
                if let Some((h, p)) = val.split_once(':') {
                    if let Ok(port) = p.parse::<u16>() {
                        policy.proxy.route = ProxyRoute::HttpForward {
                            host: String::from(h),
                            port,
                        };
                    }
                }
            }
            "no_proxy" => push_csv(&mut policy.proxy.no_proxy, val),
            "max_body_bytes" => {
                if let Ok(v) = val.parse::<usize>() {
                    policy.quotas.max_body_bytes = v.max(1024);
                    policy.downloads.max_inline_bytes = v.max(1024);
                }
            }
            "max_decompressed_bytes" => {
                if let Ok(v) = val.parse::<usize>() {
                    policy.quotas.max_decompressed_bytes = v.max(1024);
                }
            }
            "max_redirects" => {
                if let Ok(v) = val.parse::<usize>() {
                    policy.quotas.max_redirects = v.min(16);
                }
            }
            _ => {}
        }
    }
    policy
}

fn push_csv(target: &mut Vec<String>, val: &str) {
    for part in val.split(',') {
        let item = part.trim();
        if !item.is_empty() {
            target.push(String::from(item));
        }
    }
}

