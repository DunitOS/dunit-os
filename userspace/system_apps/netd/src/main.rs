#![no_std]
#![no_main]

extern crate alloc;

mod address_probe;
mod config;
mod dhcp_runtime;
mod dns_runtime;
mod interface;
mod lease_hint;
mod nic;
mod outbound;
mod socket_service;
mod static_runtime;
mod telemetry;

use alloc::vec::Vec;
use dunit::{errno, process, time::Instant, Error, Result};
const MAX_NICS: usize = 4;
use libdunit::wait_set;
use netd::Device;
use nic::Nic;
use socket_service::SocketService;

fn clock_ms(started: Instant) -> Result<u64> {
    Ok(started.elapsed()?.as_millis() as u64)
}

pub(crate) fn network_error(_: dunit_net_core::Error) -> Error {
    Error(errno::EINVAL)
}

fn wait_ready(nics: &[alloc::boxed::Box<Nic>], sockets: Option<&SocketService>, timeout: u64) -> Result<()> {
    let mut input = [wait_set::Interest::default(); MAX_NICS + 1];
    let mut count = 0;
    for (idx, nic) in nics.iter().enumerate() {
        let interests = wait_set::READ
            | if nic.needs_write_wait() {
                wait_set::WRITE
            } else {
                0
            };
        input[count] = wait_set::Interest::descriptor(nic.queue.descriptor(), interests, idx as u64);
        count += 1;
    }
    if let Some(svc) = sockets {
        input[count] =
            wait_set::Interest::descriptor(svc.descriptor(), wait_set::READ, MAX_NICS as u64);
        count += 1;
    }
    let mut output = [wait_set::Event::default(); MAX_NICS + 1];
    let n = wait_set::wait(&input[..count], &mut output[..count], timeout).map_err(Error)?;
    for event in &output[..n] {
        if event.events & wait_set::ERROR != 0 {
            return Err(Error(errno::EIO));
        }
        if event.events & wait_set::HANGUP != 0 {
            return Err(Error(errno::EPIPE));
        }
    }
    Ok(())
}

#[dunit::main]
fn main() -> Result<()> {
    let devices = Device::receive_grants()?;
    let specs = config::load_nic_specs(devices.len())?;
    let allow_test_fault =
        dunit::fs::read_to_string("/system/boot-services/netd-allow-test-fault").is_ok();
    let started = Instant::now()?;
    let seed = match dunit::rand::bytes::<4>() {
        Ok(b) => u32::from_le_bytes(b),
        Err(_) => {
            let ticks = dunit::system::stats().map_or(0, |stats| stats.uptime_ticks as u32);
            process::id() ^ ticks.rotate_left(13)
        }
    };

    let mut nics = Vec::with_capacity(devices.len());
    let now = clock_ms(started)?;
    for (id, (device, spec)) in devices.into_iter().zip(specs.iter()).enumerate() {
        nics.push(Nic::open(id, device, spec, seed, now)?);
    }
    let socket_mtu = nics.first().map_or(1500, |n| n.info.mtu as usize);
    let mut sockets = nics
        .first()
        .map(|n| SocketService::open(&n.device, socket_mtu, seed))
        .transpose()?
        .flatten();
    if sockets.is_some() {
        dunit::println!("[NETD] dual-stack v2 socket bridge ready ({} nic)", nics.len())?;
    }

    loop {
        if allow_test_fault && dunit::fs::read_to_string("/run/netd/crash-once").is_ok() {
            let _ = dunit::fs::remove_file("/run/netd/crash-once");
            dunit::println!("[NETD] injected fault exit (test-only)")?;
            process::exit(1);
        }
        let now = clock_ms(started)?;
        for nic in &mut nics {
            let _ = nic.poll_link_and_timers(now)?;
            let id = nic.id;
            let nic_ref: &mut Nic = nic;
            let mut frames = Vec::new();
            nic_ref.drain_rx_frames(|_, f| {
                frames.push(f.to_vec());
                Ok(())
            })?;
            let _ = id;
            if let Some(svc) = &mut sockets {
                for frame in frames {
                    svc.ingress(nic_ref, &frame, now)?;
                }
            } else {
                for frame in frames {
                    if !nic_ref.handle_v4_probe_or_control(&frame, now)? {
                        if let Some(peer) = nic_ref.v4_peer() {
                            if let Ok(Some(resp)) = peer.receive(&frame, now) {
                                let _ = nic_ref.tx.try_push(&resp);
                            }
                        }
                    }
                }
            }
            nic_ref.transmit_probes(now)?;
        }

        let now = clock_ms(started)?;
        if let Some(service) = &mut sockets {
            service.step(&mut nics, now)?;
        }
        for nic in &mut nics {
            nic.tx.flush(&nic.queue)?;
        }

        let mut next = u64::MAX;
        for nic in &nics {
            next = next.min(nic.deadline());
        }
        if let Some(svc) = &sockets {
            if let Some(d) = svc.deadline() {
                next = next.min(d);
            }
        }
        let timeout = next
            .saturating_sub(clock_ms(started)?)
            .saturating_mul(1_000_000);
        match wait_ready(&nics, sockets.as_ref(), timeout) {
            Ok(()) | Err(Error(errno::EAGAIN)) => {}
            Err(error) => return Err(error),
        }
    }
}
