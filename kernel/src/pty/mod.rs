//! Bounded bidirectional terminal streams. Master lifetime and ring state share
//! one lock. Slave operations borrow the owning fd endpoint. Table guards never
//! span task lookup, readiness notification or retired-channel destruction.
use crate::process::ProcessId;
use crate::sync::{IrqSafeSpinLock, IrqSafeSpinLockGuard};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

pub const PTY_RING_CAP: usize = 8192;
pub enum SlaveRead { Data(usize), WouldBlock, Eof }
struct Pty {
    master: ProcessId,
    slave: Option<Weak<Endpoint>>,
    slave_generation: u64,
    master_closed: bool,
    to_slave: VecDeque<u8>,
    to_master: VecDeque<u8>,
    // Zero means geometry has not been supplied by the terminal/client.
    dimensions: [u8; 8],
}
static PTY_TABLE: IrqSafeSpinLock<BTreeMap<u32, Pty>> = IrqSafeSpinLock::new(BTreeMap::new());
static NEXT_PTY_ID: AtomicU64 = AtomicU64::new(1);
fn table() -> IrqSafeSpinLockGuard<'static, BTreeMap<u32, Pty>> { PTY_TABLE.lock() }
#[cfg(any(test, feature = "boot-smoke-tests"))]
pub(crate) fn channel_count() -> usize { table().len() }
#[cfg(test)]
pub(crate) fn channel_exists(id: u32) -> bool { table().contains_key(&id) }
fn write_ring<E>(
    data: &[u8],
    mut lookup: impl for<'a> FnMut(&'a mut BTreeMap<u32, Pty>) -> Result<&'a mut VecDeque<u8>, E>,
) -> Result<usize, E> {
    let mut spare = VecDeque::new();
    let written = loop {
        let capacity = {
            let mut table = table();
            let ring = lookup(&mut table)?;
            match crate::byte_ring::push(ring, data, PTY_RING_CAP, &mut spare) {
                Ok(n) => break n,
                Err(capacity) => capacity,
            }
        };
        // Allocate before reentering the table and checking the channel again.
        // A close or a competing write can change the result during this gap.
        spare = VecDeque::with_capacity(capacity);
    };
    drop(spare);
    if written > 0 {
        crate::process::wake_poll_waiters();
    }
    Ok(written)
}
/// Shared by fd entries and in-flight I/O/ioctl; final drop signals slave hangup.
#[derive(Debug)]
pub struct Endpoint { id: u32, generation: u64 }
impl Drop for Endpoint {
    fn drop(&mut self) {
        let retired = {
            let mut table = table();
            // A new slave may attach after our strong count reaches zero but
            // before this destructor acquires the table. Retire only our lease.
            if table.get(&self.id).map(|pty| pty.master_closed &&
                pty.slave_generation == self.generation).unwrap_or(false) {
                table.remove(&self.id)
            } else { None }
        };
        drop(retired);
        crate::process::wake_poll_waiters();
    }
}
fn slave_alive(pty: &Pty) -> bool {
    pty.slave.as_ref().map(|endpoint| endpoint.strong_count() != 0).unwrap_or(false)
}
/// Create a channel owned by master; an endpoint can be attached once at a time.
pub fn create(master: ProcessId) -> u32 {
    let next = NEXT_PTY_ID.fetch_add(1, Ordering::SeqCst);
    assert!(next <= u32::MAX as u64, "PTY id exhausted");
    let id = next as u32;
    let old = table().insert(id, Pty {
        master, slave: None, slave_generation: 0, master_closed: false,
        to_slave: VecDeque::new(), to_master: VecDeque::new(), dimensions: [0; 8],
    });
    assert!(old.is_none(), "PTY id exhausted");
    id
}
pub fn attach_slave(id: u32, master: ProcessId) -> Option<Arc<Endpoint>> {
    let endpoint = {
        let mut table = table();
        let pty = table.get_mut(&id)?;
        if pty.master != master || pty.master_closed || slave_alive(pty) { return None; }
        pty.slave_generation = pty.slave_generation.checked_add(1)
            .expect("PTY attachment generation exhausted");
        let endpoint = Arc::new(Endpoint { id, generation: pty.slave_generation });
        pty.slave = Some(Arc::downgrade(&endpoint));
        endpoint
    };
    crate::process::wake_poll_waiters();
    Some(endpoint)
}
pub fn master_write(id: u32, master: ProcessId, data: &[u8]) -> Option<usize> {
    write_ring(data, |table| {
        let pty = table.get_mut(&id).ok_or(())?;
        if pty.master != master || pty.master_closed { return Err(()); }
        Ok(&mut pty.to_slave)
    }).ok()
}
/// Hangup is readable once the last slave owner is gone and output is drained.
pub fn master_read(id: u32, master: ProcessId, out: &mut [u8]) -> Option<(usize, bool)> {
    let (n, hangup) = {
        let mut table = table();
        let pty = table.get_mut(&id)?;
        if pty.master != master || pty.master_closed { return None; }
        let n = crate::byte_ring::pop(&mut pty.to_master, out);
        let slave_gone = pty.slave.is_some() && !slave_alive(pty);
        (n, slave_gone && pty.to_master.is_empty())
    };
    if n > 0 { crate::process::wake_poll_waiters(); }
    Some((n, hangup))
}
pub fn close(id: u32, master: ProcessId) -> bool {
    let retired = {
        let mut table = table();
        let Some(pty) = table.get_mut(&id) else { return false; };
        if pty.master != master || pty.master_closed { return false; }
        pty.master_closed = true;
        if !slave_alive(pty) { table.remove(&id) } else { None }
    };
    drop(retired);
    crate::process::wake_poll_waiters();
    true
}
pub fn slave_read_stdin(endpoint: &Endpoint, out: &mut [u8]) -> Option<SlaveRead> {
    let master_alive = master_alive(endpoint)?;
    let result = {
        let mut table = table();
        let pty = table.get_mut(&endpoint.id)?;
        let n = crate::byte_ring::pop(&mut pty.to_slave, out);
        if n > 0 { SlaveRead::Data(n) }
        else if pty.master_closed || !master_alive { SlaveRead::Eof }
        else { SlaveRead::WouldBlock }
    };
    if matches!(result, SlaveRead::Data(_)) { crate::process::wake_poll_waiters(); }
    Some(result)
}
pub fn slave_write_stdout(endpoint: &Endpoint, data: &[u8]) -> Option<Result<usize, ()>> {
    let master_alive = master_alive(endpoint)?;
    match write_ring(data, |table| {
        let pty = table.get_mut(&endpoint.id).ok_or(None)?;
        if pty.master_closed || !master_alive { return Err(Some(())); }
        Ok(&mut pty.to_master)
    }) {
        Ok(n) => Some(Ok(n)),
        Err(closed) => closed.map(Err),
    }
}
pub fn slave_output_readiness(endpoint: &Endpoint) -> Option<(bool, bool)> {
    let master_alive = master_alive(endpoint)?;
    let table = table();
    let pty = table.get(&endpoint.id)?;
    let closed = pty.master_closed || !master_alive;
    Some((!closed && pty.to_master.len() < PTY_RING_CAP, closed))
}
/// Resource cleanup closes masters. Metadata lookup also detects a detached
/// master whose last task pin has delayed cleanup. Never nest registry under PTY.
fn master_alive(endpoint: &Endpoint) -> Option<bool> {
    let master = table().get(&endpoint.id)?.master;
    Some(crate::process::process_exists(master))
}
/// Endpoint owners retain buffered input/EOF through operations racing fd close.
pub fn on_process_exit(pid: ProcessId) {
    let retired = {
        let mut table = table();
        let ids: Vec<u32> = table.keys().copied().collect();
        let mut retired = Vec::new();
        for id in ids {
            let pty = table.get_mut(&id).unwrap();
            if pty.master == pid { pty.master_closed = true; }
            if pty.master_closed && !slave_alive(pty) {
                retired.push(table.remove(&id).unwrap());
            }
        }
        retired
    };
    drop(retired);
    crate::process::wake_poll_waiters();
}
pub fn slave_dimensions(endpoint: &Endpoint) -> Option<[u8; 8]> {
    Some(table().get(&endpoint.id)?.dimensions)
}
pub fn set_slave_dimensions(endpoint: &Endpoint, dimensions: [u8; 8]) -> bool {
    let mut table = table();
    let Some(pty) = table.get_mut(&endpoint.id) else { return false; };
    pty.dimensions = dimensions;
    true
}
pub fn slave_pending_input(endpoint: &Endpoint) -> Option<usize> {
    Some(table().get(&endpoint.id)?.to_slave.len())
}
pub fn slave_readiness(endpoint: &Endpoint) -> Option<(bool, bool)> {
    let master_alive = master_alive(endpoint)?;
    let table = table();
    let pty = table.get(&endpoint.id)?;
    let closed = pty.master_closed || !master_alive;
    Some((!pty.to_slave.is_empty() || closed, closed))
}
