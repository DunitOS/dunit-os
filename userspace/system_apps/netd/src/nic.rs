//! Per-NIC runtime owning Ethernet queues, IPv4 DHCP/Static/ACD, and IPv6 NDP/SLAAC.
use alloc::{boxed::Box, vec::Vec};
use dunit::{errno, Error, Result};
use dunit_net_core::{
    acd, dhcp,
    ipv6_peer::Ip6Peer,
    router::{InterfaceView, Route4},
    Ip, Peer,
};
use dunit_packet_abi::{DeviceInfo, Records, MAX_BATCH_LEN};
use netd::{wire_error, Device, Queue};

use crate::{
    config::{NicSpec, V4ModeSpec, V6ModeSpec},
    dhcp_runtime::Dynamic,
    interface::{parse_ip, Interface},
    outbound::Outbound,
    static_runtime::Static,
};

const DHCP_ADDRESS_PATH: &str = "/system/boot-services/netd-dhcp-address.conf";
const LINK_PERIOD_MS: u64 = 1_000;

pub enum V4Mode {
    Dhcp(Dynamic),
    Static(Static),
    Disabled,
}

pub struct Nic {
    pub id: usize,
    pub device: Device,
    pub queue: Queue,
    pub info: DeviceInfo,
    pub metric: u32,
    pub link: bool,
    pub link_poll: u64,
    pub rx: Box<[u8; MAX_BATCH_LEN]>,
    pub tx: Outbound,
    pub v4_mode: V4Mode,
    pub v4_iface: Interface,
    pub v6: Option<Ip6Peer>,
    pub v6_spec: V6ModeSpec,
}

fn remembered_address(nic_id: usize) -> Result<Option<Ip>> {
    if nic_id != 0 {
        return Ok(None);
    }
    match dunit::fs::read_bounded(DHCP_ADDRESS_PATH, 32) {
        Ok(bytes) => Ok(core::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| parse_ip(text.trim()))),
        Err(Error(errno::ENOENT)) | Err(_) => Ok(None),
    }
}

impl Nic {
    pub fn open(id: usize, device: Device, spec: &NicSpec, seed: u32, now: u64) -> Result<Box<Self>> {
        let queue = device.open()?;
        let info = device.info()?;
        let mtu = info.mtu as usize;
        let mut tx = Outbound::new();
        let mut v4_iface = Interface::new(info.mac, mtu);
        let nic_seed = seed ^ ((id as u32 + 1).wrapping_mul(0x9e37_79b9));

        let v4_mode = match spec.v4 {
            V4ModeSpec::Static(config) => {
                v4_iface
                    .prepare(config, false)
                    .map_err(|_| Error(errno::EINVAL))?;
                let mut fixed = Static::new(config, info.mac, nic_seed);
                fixed.reset(info.link_up, now, &mut v4_iface, &mut tx)?;
                dunit::println!("[NETD] nic{} IPv4 static configured", id)?;
                V4Mode::Static(fixed)
            }
            V4ModeSpec::Dhcp => {
                let dynamic = Dynamic::new(info.mac, nic_seed, remembered_address(id)?)?;
                dunit::println!("[NETD] nic{} DHCPv4 enabled", id)?;
                V4Mode::Dhcp(dynamic)
            }
            V4ModeSpec::Disabled => V4Mode::Disabled,
        };

        let mut v6 = match &spec.v6 {
            V6ModeSpec::Disabled => None,
            V6ModeSpec::Slaac => {
                let peer6 = Ip6Peer::new(info.mac, mtu, spec.dad_probes, now)
                    .map_err(|_| Error(errno::EINVAL))?;
                dunit::println!("[NETD] nic{} IPv6 SLAAC/NDP enabled", id)?;
                Some(peer6)
            }
            V6ModeSpec::Static {
                address,
                prefix_len,
                gateway,
                dns,
            } => {
                let mut peer6 = Ip6Peer::new(info.mac, mtu, spec.dad_probes, now)
                    .map_err(|_| Error(errno::EINVAL))?;
                peer6.set_slaac(false);
                peer6
                    .configure_static(*address, *prefix_len, *gateway, dns, now)
                    .map_err(|_| Error(errno::EINVAL))?;
                dunit::println!("[NETD] nic{} IPv6 static configured", id)?;
                Some(peer6)
            }
        };
        let _ = &mut v6;

        Ok(Box::new(Self {
            id,
            device,
            queue,
            info,
            metric: spec.metric,
            link: info.link_up,
            link_poll: now.saturating_add(LINK_PERIOD_MS),
            rx: Box::new([0; MAX_BATCH_LEN]),
            tx,
            v4_mode,
            v4_iface,
            v6,
            v6_spec: spec.v6.clone(),
        }))
    }

    pub fn poll_link_and_timers(&mut self, now: u64) -> Result<bool> {
        let mut changed = false;
        if now >= self.link_poll {
            let current = self.device.info()?.link_up;
            if current != self.link {
                self.link = current;
                changed = true;
                match &mut self.v4_mode {
                    V4Mode::Dhcp(client) => client.reset(now, &mut self.v4_iface, &mut self.tx)?,
                    V4Mode::Static(fixed) => {
                        fixed.reset(current, now, &mut self.v4_iface, &mut self.tx)?;
                    }
                    V4Mode::Disabled => {}
                }
                if let Some(peer6) = &mut self.v6 {
                    peer6.reset(now);
                    if let V6ModeSpec::Static {
                        address,
                        prefix_len,
                        gateway,
                        dns,
                    } = &self.v6_spec
                    {
                        let _ = peer6.configure_static(*address, *prefix_len, *gateway, dns, now);
                    }
                }
                dunit::println!(
                    "[NETD] nic{} link {}",
                    self.id,
                    if current { "up" } else { "down" }
                )?;
            }
            self.link_poll = now.saturating_add(LINK_PERIOD_MS);
        }
        if self.link {
            if let V4Mode::Dhcp(client) = &mut self.v4_mode {
                client.tick(now, &mut self.v4_iface, &mut self.tx)?;
            }
            if let Some(peer6) = &mut self.v6 {
                if let Ok(frames) = peer6.poll(now) {
                    for frame in frames {
                        let _ = self.tx.try_push(&frame);
                    }
                }
            }
        }
        Ok(changed)
    }

    pub fn transmit_probes(&mut self, now: u64) -> Result<()> {
        if !self.link {
            return Ok(());
        }
        match &mut self.v4_mode {
            V4Mode::Dhcp(client) => {
                client.drive_probe(now, &self.queue, &mut self.v4_iface, &mut self.tx)?;
                client.transmit(now, &mut self.v4_iface, &mut self.tx)?;
            }
            V4Mode::Static(fixed) => {
                fixed.transmit(now, &self.queue, &mut self.v4_iface)?;
            }
            V4Mode::Disabled => {}
        }
        Ok(())
    }

    pub fn handle_v4_probe_or_control(&mut self, frame: &[u8], now: u64) -> Result<bool> {
        let handled = match &mut self.v4_mode {
            V4Mode::Dhcp(client) if dhcp::is_candidate(frame) => {
                client.receive(frame, now, &mut self.v4_iface, &mut self.tx)?;
                true
            }
            V4Mode::Dhcp(client) => {
                client.observe(frame, now, &mut self.v4_iface, &mut self.tx)?
            }
            V4Mode::Static(fixed) => fixed.observe(frame, &mut self.v4_iface, &mut self.tx)?,
            V4Mode::Disabled => false,
        };
        if handled {
            return Ok(true);
        }
        if let Some(config) = self.v4_iface.config() {
            if let Ok(Some(resp)) = acd::reply_to_probe(self.info.mac, config.address, frame) {
                self.tx.push(&resp)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn v4_peer(&mut self) -> Option<&mut Peer> {
        self.v4_iface.peer()
    }

    pub fn is_v6_ethertype(frame: &[u8]) -> bool {
        frame.len() >= 14 && u16::from_be_bytes([frame[12], frame[13]]) == 0x86dd
    }

    pub fn deadline(&self) -> u64 {
        let v4 = if self.link {
            match &self.v4_mode {
                V4Mode::Dhcp(c) => c.deadline(),
                V4Mode::Static(s) => s.deadline(),
                V4Mode::Disabled => u64::MAX,
            }
        } else {
            u64::MAX
        };
        let v6 = if self.link {
            self.v6.as_ref().and_then(Ip6Peer::deadline).unwrap_or(u64::MAX)
        } else {
            u64::MAX
        };
        self.link_poll.min(v4).min(v6)
    }

    pub fn needs_write_wait(&self) -> bool {
        if !self.tx.is_empty() {
            return true;
        }
        if !self.link {
            return false;
        }
        match &self.v4_mode {
            V4Mode::Dhcp(c) => c.waiting_for_probe_tx(),
            V4Mode::Static(s) => s.waiting_for_tx(),
            V4Mode::Disabled => false,
        }
    }

    pub fn can_ever_v4(&self) -> bool {
        self.link
            && match &self.v4_mode {
                V4Mode::Dhcp(_) => true,
                V4Mode::Static(fixed) => !fixed.is_conflicted(),
                V4Mode::Disabled => false,
            }
    }

    pub fn can_ever_v6(&self) -> bool {
        self.link && self.v6.is_some()
    }

    pub fn view(&self) -> InterfaceView {
        let mut view = InterfaceView::empty(self.id, self.metric);
        view.link_up = self.link;
        if let Some(cfg) = self.v4_iface.config() {
            view.ipv4 = Some((cfg.address, cfg.mask));
            if let Some(routes) = cfg.routes {
                for r in routes.into_iter().flatten() {
                    view.ipv4_routes.push(Route4 {
                        network: r.network,
                        mask: r.mask,
                        gateway: r.gateway,
                    });
                }
            } else if let Some(gw) = cfg.router {
                view.ipv4_routes.push(Route4 {
                    network: [0; 4],
                    mask: [0; 4],
                    gateway: Some(gw),
                });
            }
            for s in cfg.dns.into_iter().flatten() {
                view.ipv4_dns.push(s);
            }
        }
        if let Some(peer6) = &self.v6 {
            peer6.fill_interface_view(&mut view);
        }
        view
    }

    pub fn drain_rx_frames<F>(&mut self, mut handler: F) -> Result<()>
    where
        F: FnMut(&mut Nic, &[u8]) -> Result<()>,
    {
        match self.queue.try_receive(self.rx.as_mut()) {
            Ok(size) => {
                // Split borrow: copy slice cursor over `self.rx` safely by scanning lengths first.
                let mut temp = Vec::new();
                for frame in Records::new(&self.rx[..size]).map_err(wire_error)? {
                    if self.link {
                        temp.push(frame.to_vec());
                    }
                }
                for frame in temp {
                    handler(self, &frame)?;
                }
                Ok(())
            }
            Err(Error(errno::EAGAIN)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

