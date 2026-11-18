//! Capability checks and bounded copies for raw Ethernet queues.
use super::*;
use crate::drivers::net::packet as driver;
use crate::handle::{NetDevice, RIGHT_NET_CONTROL, RIGHT_NET_RAW, RIGHT_READ, RIGHT_WRITE};
use crate::process::{self, FdEntry, FdTarget, ProcessId};
use core::sync::atomic::Ordering;
use dunit_packet_abi::{
    DeviceControl, DeviceInfo, QueueOptions, Records, WireError, DEVICE_CONTROL_LEN,
    DEVICE_INFO_LEN, MAX_BATCH_LEN, QUEUE_OPTIONS_LEN,
};

pub(super) fn grant(root: u64, interface: u64, recipient: u64, rights: u64) -> i64 {
    let (Ok(root), Ok(interface), Ok(rights)) = (
        u32::try_from(root), u32::try_from(interface), u32::try_from(rights),
    ) else {
        return EINVAL;
    };
    process::grant_net_device(root, interface, ProcessId(recipient), rights)
        .map(i64::from)
        .unwrap_or_else(|error| error)
}

pub(super) fn open(handle: u64, options: *const u8, length: usize) -> i64 {
    let result = (|| {
        let handle = u32::try_from(handle).map_err(|_| EBADF)?;
        let (device, rights) = process::with_current_process(|task| task.net_device(handle))
            .ok_or(EPERM)?
            .map_err(handle_error_to_errno)?
            .ok_or(EPERM)?;
        if rights & RIGHT_NET_RAW == 0 {
            return Err(EPERM);
        }
        if length != QUEUE_OPTIONS_LEN {
            return Err(EINVAL);
        }
        QueueOptions::decode(&copy_buffer_from_user(options, length)?).map_err(wire_errno)?;
        let access = rights & (RIGHT_READ | RIGHT_WRITE);
        if access == 0 {
            return Err(EPERM);
        }
        let generation = driver::open(device.index as usize)?;
        device.queue_generation.store(generation, Ordering::Release);
        let mut entry = FdEntry::new(FdTarget::Packet { generation, access }, access | 64);
        entry.cloexec = true;
        match process::allocate_fd(entry) {
            Ok(fd) => Ok(i64::from(fd)),
            Err(error) => {
                driver::close(generation);
                Err(process_error_to_errno(error))
            }
        }
    })();
    result.unwrap_or_else(|error| error)
}

pub(super) fn read_info(device: &NetDevice, rights: u32, out: *mut u8, len: usize) -> i64 {
    let result = (|| {
        require(rights, RIGHT_READ)?;
        if len < DEVICE_INFO_LEN {
            return Err(EFBIG);
        }
        let generation = device.queue_generation.load(Ordering::Acquire);
        let info = driver::info(device.index as usize, generation)?;
        let entry = DeviceInfo {
            link_up: info.link_up,
            mtu: 1500,
            next_rx_record_len: info.next_record as u16,
            mac: info.mac,
            rx_frames: info.rx_packets,
            rx_bytes: info.rx_bytes,
            rx_dropped: info.rx_dropped,
            tx_frames: info.tx_packets,
            tx_bytes: info.tx_bytes,
            tx_errors: info.tx_errors,
        };
        copy_buffer_to_user(out, &entry.encode())?;
        Ok(DEVICE_INFO_LEN as i64)
    })();
    result.unwrap_or_else(|error| error)
}

pub(super) fn control(device: &NetDevice, rights: u32, data: *const u8, len: usize) -> i64 {
    let result = (|| {
        require(rights, RIGHT_NET_CONTROL | RIGHT_WRITE)?;
        if len != DEVICE_CONTROL_LEN {
            return Err(EINVAL);
        }
        let bytes = copy_buffer_from_user(data, len)?;
        match DeviceControl::decode(&bytes).map_err(wire_errno)? {
            DeviceControl::Reset => driver::reset(device.index as usize)?,
        }
        Ok(len as i64)
    })();
    result.unwrap_or_else(|error| error)
}

fn require(rights: u32, required: u32) -> Result<(), i64> {
    if rights & required != required { Err(EPERM) } else { Ok(()) }
}

fn wire_errno(error: WireError) -> i64 {
    match error {
        WireError::Unsupported => EOPNOTSUPP,
        WireError::Invalid | WireError::BufferTooSmall => EINVAL,
    }
}

pub(super) fn read(generation: u64, access: u32, out: *mut u8, len: usize) -> i64 {
    let result = (|| {
        require(access, RIGHT_READ)?;
        let mut data = alloc::vec![0; len.min(MAX_BATCH_LEN)];
        let reservation = driver::reserve(generation, &mut data)?;
        let bytes = reservation.bytes;
        let copy = copy_pages(out, &data[..bytes]);
        let consume = driver::finish(generation, reservation, copy.is_ok());
        copy?;
        consume?;
        Ok(bytes as i64)
    })();
    result.unwrap_or_else(|error| error)
}

fn copy_pages(out: *mut u8, data: &[u8]) -> Result<(), i64> {
    let mut offset = 0;
    while offset < data.len() {
        let address = out as usize + offset;
        let len = (4096 - (address & 4095)).min(data.len() - offset);
        copy_buffer_to_user(address as *mut u8, &data[offset..offset + len])?;
        offset += len;
    }
    Ok(())
}

pub(super) fn write(generation: u64, access: u32, data: *const u8, len: usize) -> i64 {
    let result = (|| {
        require(access, RIGHT_WRITE)?;
        if len > MAX_BATCH_LEN {
            return Err(EINVAL);
        }
        let bytes = copy_buffer_from_user(data, len)?;
        let records = Records::new(&bytes).map_err(wire_errno)?;
        driver::transmit(generation, records).map(|bytes| bytes as i64)
    })();
    result.unwrap_or_else(|error| error)
}
