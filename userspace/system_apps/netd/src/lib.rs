//! Bootstrap and owning native packet queue for the network service.
#![no_std]

use dunit::{
    errno,
    time::{Duration, Instant},
    Error, Result,
};
use dunit_packet_abi::{DeviceInfo, QueueOptions, DEVICE_INFO_LEN};
use libdunit::wait_set::{self, Event, Interest};

const GRANT_TIMEOUT: Duration = Duration::from_secs(10);

pub fn checked(result: isize) -> Result<usize> {
    usize::try_from(result).map_err(|_| Error(result))
}

pub fn wire_error(_: dunit_packet_abi::WireError) -> Error {
    Error(errno::EIO)
}

pub struct Device(u32);

impl Device {
    pub fn handle(&self) -> u32 {
        self.0
    }

    pub fn receive_grant() -> Result<Self> {
        // The launcher environment identifies the expected sender. Only an
        // actual kernel-installed device handle authorizes queue creation.
        let expected = dunit::env::var("DUNIT_NET_SUPERVISOR")?
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|pid| *pid != 0)
            .ok_or(Error(errno::EACCES))?;
        let started = Instant::now()?;
        let mut payload = [0; 256];
        loop {
            let remaining = GRANT_TIMEOUT
                .checked_sub(started.elapsed()?)
                .ok_or(Error(errno::EAGAIN))?;
            let mut sender = 0;
            let mut attachment = 0;
            let size = libdunit::ipc_recv_handle(&mut payload, &mut sender, &mut attachment);
            if size == errno::EAGAIN {
                wait_set::wait(
                    &[Interest::ipc(0)],
                    &mut [Event::default()],
                    remaining.as_nanos() as u64,
                )
                .map_err(Error)?;
                continue;
            }
            let size = checked(size)?;
            if attachment != 0 {
                libdunit::handle_close(attachment);
            }
            if sender != expected || attachment != 0 || size != 4 {
                continue;
            }
            let handle = u32::from_le_bytes(payload[..4].try_into().unwrap());
            if handle == 0 {
                return Err(Error(errno::EIO));
            }
            return Ok(Self(handle));
        }
    }

    pub fn info(&self) -> Result<DeviceInfo> {
        let mut bytes = [0; DEVICE_INFO_LEN];
        let size = checked(libdunit::handle_read(self.0, &mut bytes))?;
        if size != bytes.len() {
            return Err(Error(errno::EIO));
        }
        DeviceInfo::decode(&bytes).map_err(wire_error)
    }

    pub fn open(&self) -> Result<Queue> {
        let fd = checked(libdunit::net_queue_open(self.0, &QueueOptions.encode()))?;
        Ok(Queue(u32::try_from(fd).map_err(|_| Error(errno::EIO))?))
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        libdunit::handle_close(self.0);
    }
}

pub struct Queue(u32);

impl Queue {
    pub fn descriptor(&self) -> u32 {
        self.0
    }
    pub fn try_clone(&self) -> Result<Self> {
        let fd = checked(libdunit::syscall3(
            libdunit::SYSCALL_FCNTL,
            self.0 as usize,
            5,
            3,
        ))?;
        Ok(Self(u32::try_from(fd).map_err(|_| Error(errno::EIO))?))
    }

    pub fn wait(&self, interest: u32, timeout_ns: u64) -> Result<()> {
        let mut event = [Event::default()];
        let count = wait_set::wait(
            &[Interest::descriptor(self.0, interest, 0)],
            &mut event,
            timeout_ns,
        )
        .map_err(Error)?;
        if count == 0 {
            return Err(Error(errno::EAGAIN));
        }
        if event[0].events & wait_set::ERROR != 0 {
            return Err(Error(errno::EIO));
        }
        if event[0].events & wait_set::HANGUP != 0 {
            return Err(Error(errno::EPIPE));
        }
        Ok(())
    }

    pub fn receive(&self, buffer: &mut [u8]) -> Result<usize> {
        loop {
            let result = libdunit::read(self.0 as usize, buffer);
            if result != errno::EAGAIN {
                return checked(result);
            }
            self.wait(wait_set::READ, wait_set::INFINITE)?;
        }
    }

    pub fn send(&self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            let result = libdunit::write(self.0 as usize, bytes);
            if result == errno::EAGAIN {
                self.wait(wait_set::WRITE, wait_set::INFINITE)?;
                continue;
            }
            let count = checked(result)?;
            if count == 0 || count > bytes.len() {
                return Err(Error(errno::EIO));
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        libdunit::close(self.0 as usize);
    }
}
