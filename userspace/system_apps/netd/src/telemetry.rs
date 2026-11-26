//! Bounded, non-ambient status and route telemetry for authorized diagnostic clients.
use alloc::{boxed::Box, fmt::Write, string::String, vec::Vec};
use dunit_net_core::{router::MultiRouter, Ip, Ip6};
use dunit_socket_abi::MAX_PAYLOAD;

use crate::nic::Nic;

fn push_line_bounded(out: &mut String, line: &str) {
    if out.len() + line.len() <= MAX_PAYLOAD {
        out.push_str(line);
    }
}

fn fmt_v4(ip: Ip) -> String {
    alloc::format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

fn fmt_v6(ip: Ip6) -> String {
    alloc::format!(
        "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
        u16::from_be_bytes([ip[0], ip[1]]),
        u16::from_be_bytes([ip[2], ip[3]]),
        u16::from_be_bytes([ip[4], ip[5]]),
        u16::from_be_bytes([ip[6], ip[7]]),
        u16::from_be_bytes([ip[8], ip[9]]),
        u16::from_be_bytes([ip[10], ip[11]]),
        u16::from_be_bytes([ip[12], ip[13]]),
        u16::from_be_bytes([ip[14], ip[15]]),
    )
}

pub fn render_status(router: &MultiRouter, nics: &[Box<Nic>]) -> Vec<u8> {
    let mut out = String::new();
    for nic in nics {
        let info = nic.device.info().unwrap_or(nic.info);
        let view = router
            .interfaces()
            .iter()
            .find(|v| v.id == nic.id)
            .cloned()
            .unwrap_or_else(|| nic.view());
        let mut line = String::new();
        let _ = write!(
            line,
            "nic{} link={} metric={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            nic.id,
            if nic.link { "up" } else { "down" },
            nic.metric,
            info.mac[0],
            info.mac[1],
            info.mac[2],
            info.mac[3],
            info.mac[4],
            info.mac[5],
        );
        if let Some((ip4, mask4)) = view.ipv4 {
            let prefix = u32::from_be_bytes(mask4).count_ones();
            let _ = write!(line, " ipv4={}/{}", fmt_v4(ip4), prefix);
        }
        if let Some(ll) = view.ipv6_ll {
            let _ = write!(line, " ll6={}", fmt_v6(ll));
        }
        for (g, plen) in &view.ipv6_globals {
            let _ = write!(line, " ipv6={}/{}", fmt_v6(*g), plen);
        }
        for dns in &view.ipv4_dns {
            let _ = write!(line, " dns4={}", fmt_v4(*dns));
        }
        for dns in &view.ipv6_dns {
            let _ = write!(line, " dns6={}", fmt_v6(*dns));
        }
        let _ = writeln!(
            line,
            " rx_pkt={} rx_bytes={} rx_drop={} tx_pkt={} tx_bytes={} tx_err={}",
            info.rx_frames,
            info.rx_bytes,
            info.rx_dropped,
            info.tx_frames,
            info.tx_bytes,
            info.tx_errors,
        );
        push_line_bounded(&mut out, &line);
    }
    if out.is_empty() {
        out.push_str("none\n");
    }
    out.into_bytes()
}

pub fn render_routes(router: &MultiRouter) -> Vec<u8> {
    let mut out = String::new();
    let mut sorted: Vec<_> = router.interfaces().iter().collect();
    sorted.sort_by_key(|v| (v.metric, v.id));
    for v in sorted {
        let state = if v.link_up { "up" } else { "down" };
        if let Some((ip4, mask4)) = v.ipv4 {
            let net = [
                ip4[0] & mask4[0],
                ip4[1] & mask4[1],
                ip4[2] & mask4[2],
                ip4[3] & mask4[3],
            ];
            let pfx = u32::from_be_bytes(mask4).count_ones();
            let line = alloc::format!(
                "v4 {}/{} dev nic{} metric {} link {}\n",
                fmt_v4(net),
                pfx,
                v.id,
                v.metric,
                state
            );
            push_line_bounded(&mut out, &line);
        }
        for r4 in &v.ipv4_routes {
            let pfx = u32::from_be_bytes(r4.mask).count_ones();
            let via = r4.gateway.map_or_else(|| String::from("on-link"), fmt_v4);
            let line = alloc::format!(
                "v4 {}/{} via {} dev nic{} metric {} link {}\n",
                fmt_v4(r4.network),
                pfx,
                via,
                v.id,
                v.metric,
                state
            );
            push_line_bounded(&mut out, &line);
        }
        for r6 in &v.ipv6_routes {
            let via = r6.gateway.map_or_else(|| String::from("on-link"), fmt_v6);
            let line = alloc::format!(
                "v6 {}/{} via {} dev nic{} metric {} link {}\n",
                fmt_v6(r6.prefix),
                r6.prefix_len,
                via,
                v.id,
                v.metric,
                state
            );
            push_line_bounded(&mut out, &line);
        }
    }
    if out.is_empty() {
        out.push_str("none\n");
    }
    out.into_bytes()
}
