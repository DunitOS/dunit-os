//! Bootstrap and parent-to-child publication of packet capabilities.
use super::*;
use crate::handle::{RIGHT_NET_CONTROL, RIGHT_NET_RAW};

pub(crate) fn bootstrap_net_root(pid: ProcessId) -> Result<Handle, ProcessError> {
    with_process_mut(pid, |process| {
        Ok(process.handle_table.insert(HandleObject::NetRoot, RIGHT_NET_CONTROL))
    })
}

pub(crate) fn grant_net_device(
    root: Handle,
    index: u32,
    child: ProcessId,
    rights: u32,
) -> Result<Handle, i64> {
    use crate::syscall::{EINVAL, ENOENT, EPERM};
    let parent = with_current_process(|process| {
        process.handle_table.require(root, RIGHT_NET_CONTROL)
            .ok()
            .filter(|entry| matches!(entry.object, HandleObject::NetRoot))
            .map(|_| process.pid)
    })
    .flatten()
    .ok_or(EPERM)?;
    let allowed = RIGHT_NET_CONTROL | RIGHT_NET_RAW | RIGHT_READ | RIGHT_WRITE;
    if rights == 0 || rights & !allowed != 0
        || (rights & RIGHT_NET_RAW != 0 && rights & (RIGHT_READ | RIGHT_WRITE) == 0)
    {
        return Err(EINVAL);
    }
    crate::drivers::net::packet::supports(index as usize)?;
    let device = Arc::new(crate::handle::NetDevice::new(index));
    with_task_record(child, |process, record| {
        if record.parent != Some(parent) || process.is_kernel {
            return Ok(Err(EPERM));
        }
        if matches!(record.state, ProcessState::Dead | ProcessState::Reaped) {
            return Ok(Err(ENOENT));
        }
        Ok(Ok(process.handle_table.insert(HandleObject::NetDevice(device), rights)))
    })
    .map_err(|_| ENOENT)?
}

impl Process {
    pub(crate) fn net_device(
        &self,
        handle: Handle,
    ) -> Result<Option<(Arc<crate::handle::NetDevice>, u32)>, HandleError> {
        let entry = self.handle_table.get(handle)?;
        Ok(match &entry.object {
            HandleObject::NetDevice(device) => Some((device.clone(), entry.rights)),
            HandleObject::File(_) | HandleObject::NetRoot | HandleObject::Memory(_)
            | HandleObject::Endpoint(_) | HandleObject::Display | HandleObject::Input
            | HandleObject::SharedFrames { .. } => None,
        })
    }

    /// Move the reference to the syscall before running hardware retirement.
    pub(crate) fn take_net_device(&mut self, handle: Handle) -> Option<Arc<crate::handle::NetDevice>> {
        if !matches!(self.handle_table.get(handle).ok()?.object, HandleObject::NetDevice(_)) {
            return None;
        }
        match self.handle_table.remove(handle).ok()?.object {
            HandleObject::NetDevice(device) => Some(device),
            _ => unreachable!("packet handle was checked while the task was locked"),
        }
    }
}
