//! Multi-interface dual-stack routing, metrics, DNS selection, and link failover.
use crate::{ipv6, valid_ip, Error, Ip, Ip6, IpAddr};
use alloc::vec::Vec;

pub const MAX_IFACES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route4 {
    pub network: Ip,
    pub mask: Ip,
    pub gateway: Option<Ip>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route6 {
    pub prefix: Ip6,
    pub prefix_len: u8,
    pub gateway: Option<Ip6>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceView {
    pub id: usize,
    pub link_up: bool,
    pub metric: u32,
    pub ipv4: Option<(Ip, Ip)>,
    pub ipv4_routes: Vec<Route4>,
    pub ipv4_dns: Vec<Ip>,
    pub ipv6_ll: Option<Ip6>,
    pub ipv6_globals: Vec<(Ip6, u8)>,
    pub ipv6_routes: Vec<Route6>,
    pub ipv6_dns: Vec<Ip6>,
}

impl InterfaceView {
    pub fn empty(id: usize, metric: u32) -> Self {
        Self {
            id,
            link_up: true,
            metric,
            ipv4: None,
            ipv4_routes: Vec::new(),
            ipv4_dns: Vec::new(),
            ipv6_ll: None,
            ipv6_globals: Vec::new(),
            ipv6_routes: Vec::new(),
            ipv6_dns: Vec::new(),
        }
    }

    pub fn owns_ip(&self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => self.ipv4.is_some_and(|(a, _)| a == v4),
            IpAddr::V6(v6) => {
                self.ipv6_ll == Some(v6) || self.ipv6_globals.iter().any(|(a, _)| *a == v6)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteChoice {
    pub iface_id: usize,
    pub source: IpAddr,
    pub next_hop: IpAddr,
    pub metric: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DnsChoice {
    pub iface_id: usize,
    pub server: IpAddr,
}

pub struct MultiRouter {
    ifaces: Vec<InterfaceView>,
}

impl Default for MultiRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiRouter {
    pub fn new() -> Self {
        Self { ifaces: Vec::new() }
    }

    pub fn interfaces(&self) -> &[InterfaceView] {
        &self.ifaces
    }

    pub fn set_interface(&mut self, view: InterfaceView) {
        if let Some(slot) = self.ifaces.iter_mut().find(|i| i.id == view.id) {
            *slot = view;
        } else if self.ifaces.len() < MAX_IFACES {
            self.ifaces.push(view);
        }
    }

    pub fn has_usable_v4(&self) -> bool {
        self.ifaces.iter().any(|i| i.link_up && i.ipv4.is_some())
    }

    pub fn has_usable_v6(&self) -> bool {
        self.ifaces
            .iter()
            .any(|i| i.link_up && (i.ipv6_ll.is_some() || !i.ipv6_globals.is_empty()))
    }

    pub fn has_routable_v6(&self) -> bool {
        self.ifaces.iter().any(|i| {
            i.link_up
                && (!i.ipv6_globals.is_empty()
                    || !i.ipv6_dns.is_empty()
                    || !i.ipv6_routes.is_empty())
        })
    }

    pub fn primary_ip(&self, ipv6_preferred: bool) -> Option<(usize, IpAddr)> {
        let mut sorted: Vec<&InterfaceView> = self.ifaces.iter().filter(|i| i.link_up).collect();
        sorted.sort_by_key(|i| (i.metric, i.id));
        if ipv6_preferred {
            for i in &sorted {
                if let Some((ip, _)) = i.ipv6_globals.first() {
                    return Some((i.id, IpAddr::V6(*ip)));
                }
            }
            for i in &sorted {
                if let Some(ll) = i.ipv6_ll {
                    return Some((i.id, IpAddr::V6(ll)));
                }
            }
        }
        for i in &sorted {
            if let Some((v4, _)) = i.ipv4 {
                return Some((i.id, IpAddr::V4(v4)));
            }
        }
        for i in &sorted {
            if let Some((ip, _)) = i.ipv6_globals.first() {
                return Some((i.id, IpAddr::V6(*ip)));
            }
        }
        None
    }

    pub fn select(&self, destination: IpAddr, scope_id: u32) -> Result<RouteChoice, Error> {
        match destination {
            IpAddr::V4(dst) => self.select_v4(dst),
            IpAddr::V6(dst) => self.select_v6(dst, scope_id),
        }
    }

    pub fn select_v4(&self, dst: Ip) -> Result<RouteChoice, Error> {
        if !valid_ip(dst) {
            return Err(Error::Address);
        }
        let dst_u32 = u32::from_be_bytes(dst);
        let mut best: Option<(u32, u32, usize, RouteChoice)> = None;

        for iface in self.ifaces.iter().filter(|i| i.link_up) {
            let Some((local, mask)) = iface.ipv4 else {
                continue;
            };
            if dst == local {
                return Err(Error::Address);
            }
            let mask_u32 = u32::from_be_bytes(mask);
            let local_net = u32::from_be_bytes(local) & mask_u32;
            if dst_u32 & mask_u32 == local_net {
                let bcast = local_net | !mask_u32;
                if mask_u32.count_ones() < 31 && (dst_u32 == local_net || dst_u32 == bcast) {
                    continue;
                }
                let prefix_len = mask_u32.count_ones();
                let candidate = RouteChoice {
                    iface_id: iface.id,
                    source: IpAddr::V4(local),
                    next_hop: IpAddr::V4(dst),
                    metric: iface.metric,
                };
                update_best(&mut best, prefix_len, iface.metric, iface.id, candidate);
            }
            for r in &iface.ipv4_routes {
                let rm = u32::from_be_bytes(r.mask);
                let rn = u32::from_be_bytes(r.network);
                if rm.leading_ones() != rm.count_ones() || rn & !rm != 0 {
                    continue;
                }
                if dst_u32 & rm == rn {
                    let prefix_len = rm.count_ones();
                    let next = r.gateway.unwrap_or(dst);
                    let candidate = RouteChoice {
                        iface_id: iface.id,
                        source: IpAddr::V4(local),
                        next_hop: IpAddr::V4(next),
                        metric: iface.metric,
                    };
                    update_best(&mut best, prefix_len, iface.metric, iface.id, candidate);
                }
            }
        }
        best.map(|(_, _, _, c)| c).ok_or(Error::NoRoute)
    }

    pub fn select_v6(&self, dst: Ip6, scope_id: u32) -> Result<RouteChoice, Error> {
        if !ipv6::valid_unicast(dst) {
            return Err(Error::Address);
        }
        if ipv6::is_link_local(dst) {
            let mut candidates: Vec<&InterfaceView> = self
                .ifaces
                .iter()
                .filter(|i| i.link_up && i.ipv6_ll.is_some())
                .filter(|i| scope_id == 0 || (i.id + 1) as u32 == scope_id)
                .collect();
            candidates.sort_by_key(|i| (i.metric, i.id));
            let iface = candidates.first().ok_or(Error::NoRoute)?;
            let src = iface.ipv6_ll.ok_or(Error::NoRoute)?;
            if dst == src {
                return Err(Error::Address);
            }
            return Ok(RouteChoice {
                iface_id: iface.id,
                source: IpAddr::V6(src),
                next_hop: IpAddr::V6(dst),
                metric: iface.metric,
            });
        }

        let mut best: Option<(u32, u32, usize, RouteChoice)> = None;
        for iface in self.ifaces.iter().filter(|i| i.link_up) {
            if scope_id != 0 && (iface.id + 1) as u32 != scope_id {
                continue;
            }
            if iface.owns_ip(IpAddr::V6(dst)) {
                return Err(Error::Address);
            }
            for &(addr, plen) in &iface.ipv6_globals {
                if ipv6::prefix_contains(addr, plen, dst) {
                    let candidate = RouteChoice {
                        iface_id: iface.id,
                        source: IpAddr::V6(addr),
                        next_hop: IpAddr::V6(dst),
                        metric: iface.metric,
                    };
                    update_best(&mut best, u32::from(plen), iface.metric, iface.id, candidate);
                }
            }
            let src_for_global = iface
                .ipv6_globals
                .first()
                .map(|(a, _)| *a)
                .or(iface.ipv6_ll);
            let Some(src) = src_for_global else {
                continue;
            };
            for r in &iface.ipv6_routes {
                if ipv6::prefix_contains(r.prefix, r.prefix_len, dst) {
                    if r.gateway.is_some() && iface.ipv6_globals.is_empty() {
                        continue;
                    }
                    let next = r.gateway.unwrap_or(dst);
                    let candidate = RouteChoice {
                        iface_id: iface.id,
                        source: IpAddr::V6(src),
                        next_hop: IpAddr::V6(next),
                        metric: iface.metric,
                    };
                    update_best(
                        &mut best,
                        u32::from(r.prefix_len),
                        iface.metric,
                        iface.id,
                        candidate,
                    );
                }
            }
        }
        best.map(|(_, _, _, c)| c).ok_or(Error::NoRoute)
    }

    pub fn dns_servers(&self) -> Vec<DnsChoice> {
        let mut sorted: Vec<&InterfaceView> = self.ifaces.iter().filter(|i| i.link_up).collect();
        sorted.sort_by_key(|i| (i.metric, i.id));
        let mut out = Vec::new();
        for iface in sorted {
            if !iface.ipv6_globals.is_empty() || iface.ipv6_ll.is_some() {
                for &v6 in &iface.ipv6_dns {
                    let item = DnsChoice {
                        iface_id: iface.id,
                        server: IpAddr::V6(v6),
                    };
                    if !out.iter().any(|existing: &DnsChoice| existing.server == item.server) {
                        out.push(item);
                    }
                }
            }
            if iface.ipv4.is_some() {
                for &v4 in &iface.ipv4_dns {
                    let item = DnsChoice {
                        iface_id: iface.id,
                        server: IpAddr::V4(v4),
                    };
                    if !out.iter().any(|existing: &DnsChoice| existing.server == item.server) {
                        out.push(item);
                    }
                }
            }
        }
        out
    }
}

fn update_best(
    best: &mut Option<(u32, u32, usize, RouteChoice)>,
    prefix_len: u32,
    metric: u32,
    id: usize,
    candidate: RouteChoice,
) {
    let better = match best {
        None => true,
        Some((best_plen, best_metric, best_id, _)) => {
            (prefix_len > *best_plen)
                || (prefix_len == *best_plen && (metric, id) < (*best_metric, *best_id))
        }
    };
    if better {
        *best = Some((prefix_len, metric, id, candidate));
    }
}
