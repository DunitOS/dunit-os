#![no_std]
#![no_main]

extern crate alloc;

mod dns_tcp;
mod tcp_listener;

use alloc::{format, vec};
use dunit::{
    errno,
    process::{self, Child},
    time::{self, Duration, Instant},
    Error, Result,
};
use dunit_net_core::{
    dns::{AddressData, Kind, Poll, Resolver},
    udp::Address,
    Echo, Ethernet, Ip, Ipv4, Peer,
};
use dunit_packet_abi::{encode_record, DeviceControl, QueueOptions, Records, MAX_BATCH_LEN};
use libdunit::{wait_set, RIGHT_READ, RIGHT_TRANSFER};
use netd::{wire_error, Device, Queue};

const GUEST: Ip = [192, 0, 2, 2];
const HOST: Ip = [192, 0, 2, 1];
const NAME: &[u8] = b"du.net.test";
const ECHO_DATA: &[u8] = b"dunit-netd-live";
const RX_TIMEOUT_NS: u64 = 15_000_000_000;
const PATH: &str = "/system/libexec/netd_fixture";
static READ_ONLY: [u8; 128] = [0xa5; 128];

fn net_error(_: dunit_net_core::Error) -> Error {
    Error(errno::EIO)
}

fn require(ok: bool, check: &str) -> Result<()> {
    if ok {
        return Ok(());
    }
    dunit::println!("[NETD-FIXTURE] FAIL {}", check)?;
    Err(Error(errno::EIO))
}

fn clock(started: Instant) -> Result<u64> {
    Ok(started.elapsed()?.as_millis() as u64)
}

fn send_frame(queue: &Queue, frame: &[u8], tx: &mut [u8]) -> Result<()> {
    let count = encode_record(frame, tx).map_err(wire_error)?;
    queue.send(&tx[..count])
}

fn check_no_authority(fd: u32, device: u32) -> Result<()> {
    let mut bytes = [0; 80];
    require(
        libdunit::read(fd as usize, &mut bytes) == errno::EBADF,
        "inherited packet fd",
    )?;
    require(
        libdunit::net_queue_open(device, &QueueOptions.encode()) < 0,
        "inherited device handle",
    )?;
    require(
        libdunit::net_device_grant(1, 0, process::id(), RIGHT_READ) < 0,
        "ordinary grant",
    )?;
    dunit::println!("[NETD-FIXTURE] child cannot inherit packet authority")
}

fn authority(device: &Device, queue: &Queue) -> Result<()> {
    let handle = device.handle();
    require(
        libdunit::handle_rights(handle) & RIGHT_TRANSFER as isize == 0,
        "transfer right",
    )?;
    require(
        libdunit::handle_dup(handle, RIGHT_TRANSFER) < 0,
        "widening handle rights",
    )?;
    require(
        libdunit::ipc_send_handle(process::id(), handle, RIGHT_READ, &[0; 4], 0) == errno::EACCES,
        "device handle passed through IPC",
    )?;
    require(
        libdunit::handle_write(handle, &DeviceControl::Reset.encode()) == errno::EPERM,
        "reset without control right",
    )?;
    let attenuated = libdunit::handle_dup(handle, RIGHT_READ);
    require(attenuated > 0, "attenuated handle")?;
    let denied = libdunit::net_queue_open(attenuated as u32, &QueueOptions.encode());
    libdunit::handle_close(attenuated as u32);
    require(denied == errno::EPERM, "open without raw right")?;
    let mut invalid = QueueOptions.encode();
    invalid[15] = 1;
    require(
        libdunit::net_queue_open(handle, &invalid) == errno::EINVAL,
        "reserved option",
    )?;
    require(
        libdunit::syscall3(libdunit::SYSCALL_FCNTL, queue.descriptor() as usize, 1, 0) == 0,
        "clear descriptor close-on-exec",
    )?;
    let fd = format!("{}", queue.descriptor());
    let handle = format!("{handle}");
    let mut child = Child::spawn_with_env(PATH, &[PATH, "deny", &fd, &handle], &[])?;
    require(child.wait()?.success(), "child authority isolation")
}

fn first_receive(device: &Device, queue: &Queue, rx: &mut [u8]) -> Result<usize> {
    queue.wait(wait_set::READ, RX_TIMEOUT_NS)?;
    let before = device.info()?.next_rx_record_len;
    require(before > 8, "queued RX metadata")?;
    let fd = queue.descriptor() as usize;
    require(
        libdunit::read(fd, &mut [0; 8]) == errno::EFBIG,
        "short RX buffer",
    )?;
    require(
        libdunit::syscall3(
            libdunit::SYSCALL_READ,
            fd,
            READ_ONLY.as_ptr() as usize,
            before as usize,
        ) == errno::EFAULT,
        "RX invalid destination",
    )?;
    require(
        device.info()?.next_rx_record_len == before,
        "RX consumed on failed copy",
    )?;
    queue.receive(rx)
}

fn exchange() -> Result<()> {
    let device = Device::receive_grant()?;
    let info = device.info()?;
    let original = device.open()?;
    let queue = original.try_clone()?;
    drop(original);
    authority(&device, &queue)?;
    // QEMU holds receives for one second after the reference NIC programs RCTL.
    // Give it that settling time before starting the logical ARP deadline.
    time::sleep(Duration::from_millis(1250))?;
    let mut peer =
        Peer::new(info.mac, GUEST, [255, 255, 255, 0], info.mtu as usize).map_err(net_error)?;
    let mut rx = vec![0; MAX_BATCH_LEN];
    let mut tx = vec![0; MAX_BATCH_LEN];
    let started = Instant::now()?;
    let id = process::id() as u16;
    let arp = peer
        .echo(HOST, id, 1, ECHO_DATA, clock(started)?)
        .map_err(net_error)?
        .ok_or(Error(errno::EIO))?;
    let size = encode_record(&arp, &mut tx).map_err(wire_error)?;
    tx[size..size + 8].fill(0);
    let fd = queue.descriptor() as usize;
    require(
        libdunit::write(fd, &tx[..size + 8]) == errno::EINVAL,
        "invalid suffix publishes TX prefix",
    )?;
    require(
        libdunit::syscall3(libdunit::SYSCALL_WRITE, fd, 1, size) == errno::EFAULT,
        "TX invalid source",
    )?;
    queue.send(&tx[..size])?;
    let size = first_receive(&device, &queue, &mut rx)?;
    for frame in Records::new(&rx[..size]).map_err(wire_error)? {
        peer.receive(frame, clock(started)?).map_err(net_error)?;
    }
    let ping = peer
        .echo(HOST, id, 1, ECHO_DATA, clock(started)?)
        .map_err(net_error)?
        .ok_or(Error(errno::EIO))?;
    send_frame(&queue, &ping, &mut tx)?;
    queue.wait(wait_set::READ, RX_TIMEOUT_NS)?;
    let count = queue.receive(&mut rx)?;
    let mut replies = Records::new(&rx[..count]).map_err(wire_error)?;
    let frame = Ethernet::parse(replies.next().ok_or(Error(errno::EIO))?).map_err(net_error)?;
    let ip = Ipv4::parse(frame.payload).map_err(net_error)?;
    let echo = Echo::parse(ip.payload).map_err(net_error)?;
    require(
        frame.destination == info.mac
            && frame.kind == 0x0800
            && ip.source == HOST
            && ip.destination == GUEST
            && ip.protocol == 1
            && echo.reply
            && echo.identifier == id
            && echo.sequence == 1
            && echo.data == ECHO_DATA
            && replies.next().is_none(),
        "live echo reply",
    )?;

    let mut resolver = Resolver::with_secondary(
        &mut peer,
        Address { ip: HOST, port: 53 },
        Address { ip: [192, 0, 2, 53], port: 53 },
        false,
    ).map_err(net_error)?;
    require(
        resolver
            .start(NAME, Kind::A, clock(started)?)
            .map_err(net_error)?
            .is_none(),
        "fresh DNS transaction",
    )?;
    let Poll::Frame(query) = resolver
        .poll(&mut peer, clock(started)?)
        .map_err(net_error)?
    else {
        return Err(Error(errno::EIO));
    };
    send_frame(&queue, &query, &mut tx)?;
    queue.wait(wait_set::READ, RX_TIMEOUT_NS)?;
    let count = queue.receive(&mut rx)?;
    for frame in Records::new(&rx[..count]).map_err(wire_error)? {
        peer.receive(frame, clock(started)?).map_err(net_error)?;
    }
    let Poll::Answer(answer) = resolver
        .poll(&mut peer, clock(started)?)
        .map_err(net_error)?
    else {
        return Err(Error(errno::EIO));
    };
    require(answer.ttl == 30, "live DNS TTL")?;
    let missing = b"missing.du.net.test";
    require(
        resolver.start(missing, Kind::A, clock(started)?).map_err(net_error)?.is_none(),
        "fresh negative DNS query",
    )?;
    let Poll::Frame(query) = resolver.poll(&mut peer, clock(started)?).map_err(net_error)? else {
        return Err(Error(errno::EIO));
    };
    send_frame(&queue, &query, &mut tx)?;
    queue.wait(wait_set::READ, RX_TIMEOUT_NS)?;
    let count = queue.receive(&mut rx)?;
    for frame in Records::new(&rx[..count]).map_err(wire_error)? {
        peer.receive(frame, clock(started)?).map_err(net_error)?;
    }
    require(
        resolver.poll(&mut peer, clock(started)?) == Err(dunit_net_core::Error::NameNotFound)
            && resolver.start(missing, Kind::Aaaa, clock(started)?)
                == Err(dunit_net_core::Error::NameNotFound),
        "NXDOMAIN cached across record types",
    )?;
    dunit::println!("[NETD-FIXTURE] DNS NXDOMAIN cached from SOA across record types")?;
    resolver.start(b"failover.du.net.test", Kind::A, clock(started)?).map_err(net_error)?;
    // A primary query, ARP for the secondary, then a fresh secondary query.
    for _ in 0..3 {
        let Poll::Frame(frame) = resolver.poll(&mut peer, clock(started)?).map_err(net_error)? else {
            return Err(Error(errno::EIO));
        };
        send_frame(&queue, &frame, &mut tx)?;
        queue.wait(wait_set::READ, RX_TIMEOUT_NS)?;
        let count = queue.receive(&mut rx)?;
        for frame in Records::new(&rx[..count]).map_err(wire_error)? {
            peer.receive(frame, clock(started)?).map_err(net_error)?;
        }
    }
    require(
        resolver.poll(&mut peer, clock(started)?).map_err(net_error)? == Poll::Answer(answer.clone()),
        "secondary DNS answer after SERVFAIL",
    )?;
    dunit::println!("[NETD-FIXTURE] DNS secondary answered after primary SERVFAIL")?;
    resolver.close(&mut peer).map_err(net_error)?;
    let fallback = dns_tcp::exchange(
        &queue, &mut peer, info.mtu as usize, started, &mut rx, &mut tx,
    )?;
    require(fallback == answer, "split TCP DNS response")?;
    dunit::println!("[NETD-FIXTURE] TCP DNS fallback after truncated UDP and SYN loss passed")?;
    dunit::println!("[NETD-FIXTURE] native authority, faults, ARP, ICMP and UDP DNS passed")?;
    match answer.address {
        AddressData::V4([192, 0, 2, 3]) => process::exit(7), // kernel closes live fd/session
        AddressData::V4([192, 0, 2, 4]) => {
            drop(device);
            require(
                libdunit::read(queue.descriptor() as usize, &mut rx) == errno::EPIPE,
                "revoked descriptor read",
            )?;
            require(
                matches!(queue.wait(wait_set::READ, RX_TIMEOUT_NS), Err(Error(errno::EPIPE))),
                "revoked descriptor readiness",
            )?;
            dunit::println!("[NETD-FIXTURE] stale queue revoked with EPIPE and HANGUP")?;
            dunit::println!("[NETD-FIXTURE] fresh session after supervised restart")?;
            Ok(())
        }
        _ => Err(Error(errno::EIO)),
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let args = dunit::args::args()?;
    if let [_, mode, fd, device] = args.as_slice() {
        if mode == "deny" {
            return check_no_authority(
                fd.parse().map_err(|_| Error(errno::EINVAL))?,
                device.parse().map_err(|_| Error(errno::EINVAL))?,
            );
        }
    }
    exchange()
}
