//! PTY-like endpoint: a bidirectional byte channel between a userspace terminal
//! emulator (the *master*) and a spawned child process whose stdin/stdout it
//! drives (the *slave*).
//!
//! The kernel had no stream/pipe primitive — only bounded 256-byte datagram IPC
//! and shared-memory regions — and interactive stdin only worked for the single
//! foreground process fed by a kernel-side `ExecInput` provider (the GUI path
//! passed `NoExecInput`, so exec'd interactive programs could never read input
//! from a userspace terminal). A `Pty` fills that gap with two byte rings:
//!
//! * `to_slave`  — master writes keystrokes, the slave reads them as stdin.
//! * `to_master` — the slave writes stdout/stderr, the master reads them to paint.
//!
//! Both sides poll cooperatively (read returns "would block" when empty and the
//! caller yields), matching the OS's cooperative scheduler; no wait/wake wiring
//! is needed. Routing is keyed by pid: `sys_read`/`sys_write` consult the table
//! before the legacy terminal-stdin / console paths (see `syscall::sys_read`,
//! `sys_write`). A master owns its ptys; slave attachment records the child pid.

use crate::process::ProcessId;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU64, Ordering};

/// Per-direction ring capacity. Large enough for a screenful of scrollback plus
/// a typed line; full rings produce short writes/EAGAIN; callers can await readiness.
pub const PTY_RING_CAP: usize = 8192;

/// Result of a slave-side stdin read.
pub enum SlaveRead {
    /// Bytes were copied into the caller buffer.
    Data(usize),
    /// The ring is empty but the master is alive — caller should retry (EAGAIN).
    WouldBlock,
    /// The ring is empty and the master is gone — end of input (EOF).
    Eof,
}

struct Pty {
    master: ProcessId,
    slave: Option<Weak<Endpoint>>,
    master_closed: bool,
    to_slave: VecDeque<u8>,
    to_master: VecDeque<u8>,
    // Zero means geometry has not been supplied by the terminal/client.
    dimensions: [u8; 8],
}

/// Синглтон таблицы PTY. Тот же паттерн, что у `IpcManager`/`PROCESS_TABLE`:
/// `UnsafeCell`-newtype без `static mut`, доступ кооперативный на одном CPU.
struct PtyTableCell(UnsafeCell<Option<BTreeMap<u32, Pty>>>);
unsafe impl Sync for PtyTableCell {}
static PTY_TABLE: PtyTableCell = PtyTableCell(UnsafeCell::new(None));
static NEXT_PTY_ID: AtomicU64 = AtomicU64::new(1);

fn table() -> &'static mut BTreeMap<u32, Pty> {
    unsafe {
        let slot = PTY_TABLE.0.get();
        if (*slot).is_none() {
            *slot = Some(BTreeMap::new());
        }
        (*slot).as_mut().unwrap()
    }
}

fn ring_push(ring: &mut VecDeque<u8>, data: &[u8]) -> usize {
    let space = PTY_RING_CAP.saturating_sub(ring.len());
    let n = space.min(data.len());
    ring.extend(data[..n].iter().copied());
    n
}

fn ring_pop(ring: &mut VecDeque<u8>, out: &mut [u8]) -> usize {
    let n = ring.len().min(out.len());
    for slot in out.iter_mut().take(n) {
        *slot = ring.pop_front().unwrap();
    }
    n
}

/// Shared terminal endpoint held by fd entries, including inherited/duplicated
/// descriptors. Process identity is not terminal ownership.
#[derive(Debug)]
pub struct Endpoint {
    pub id: u32,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        if table()
            .get(&self.id)
            .map(|pty| pty.master_closed)
            .unwrap_or(false)
        {
            table().remove(&self.id);
        }
        crate::process::wake_poll_waiters();
    }
}
fn slave_alive(pty: &Pty) -> bool {
    pty.slave
        .as_ref()
        .map(|endpoint| endpoint.strong_count() != 0)
        .unwrap_or(false)
}

/// Create a new PTY owned by `master`. Returns its id (always >= 1).
pub fn create(master: ProcessId) -> u32 {
    let id = NEXT_PTY_ID.fetch_add(1, Ordering::SeqCst) as u32;
    table().insert(
        id,
        Pty {
            master,
            slave: None,
            master_closed: false,
            to_slave: VecDeque::new(),
            to_master: VecDeque::new(),
            dimensions: [0; 8],
        },
    );
    id
}

/// Record `slave` as the child attached to `id`. Fails if the pty is unknown or
/// `master` is not its owner.
pub fn attach_slave(id: u32, master: ProcessId) -> Option<Arc<Endpoint>> {
    let pty = table().get_mut(&id)?;
    if pty.master != master || pty.master_closed || slave_alive(pty) {
        return None;
    }
    let endpoint = Arc::new(Endpoint { id });
    pty.slave = Some(Arc::downgrade(&endpoint));
    crate::process::wake_poll_waiters();
    Some(endpoint)
}

/// Master → slave: enqueue keystrokes for the child's stdin. Returns bytes
/// accepted, or `None` if the pty is unknown / not owned by `master`.
pub fn master_write(id: u32, master: ProcessId, data: &[u8]) -> Option<usize> {
    match table().get_mut(&id) {
        Some(pty) if pty.master == master && !pty.master_closed => {
            let written = ring_push(&mut pty.to_slave, data);
            if written > 0 {
                crate::process::wake_poll_waiters();
            }
            Some(written)
        }
        _ => None,
    }
}

/// Master ← slave: drain the child's stdout. Returns `(bytes, hangup)` where
/// `hangup` is true once the slave has exited and its output is fully drained.
/// `None` if the pty is unknown / not owned by `master`.
pub fn master_read(id: u32, master: ProcessId, out: &mut [u8]) -> Option<(usize, bool)> {
    match table().get_mut(&id) {
        Some(pty) if pty.master == master && !pty.master_closed => {
            let n = ring_pop(&mut pty.to_master, out);
            if n > 0 {
                crate::process::wake_poll_waiters();
            }
            let slave_gone = pty.slave.is_some() && !slave_alive(pty);
            Some((n, slave_gone && pty.to_master.is_empty()))
        }
        _ => None,
    }
}

/// Master closes its pty. Fails if unknown / not owned by `master`.
pub fn close(id: u32, master: ProcessId) -> bool {
    let Some(pty) = table().get_mut(&id) else {
        return false;
    };
    if pty.master != master || pty.master_closed {
        return false;
    }
    pty.master_closed = true;
    if !slave_alive(pty) {
        table().remove(&id);
    }
    crate::process::wake_poll_waiters();
    true
}

/// Slave-side stdin read, keyed by the reading process's pid. Returns `None`
/// when `pid` is not a pty slave (so `sys_read` falls back to the legacy
/// terminal-stdin path).
pub fn slave_read_stdin(id: u32, out: &mut [u8]) -> Option<SlaveRead> {
    let pty = table().get_mut(&id).unwrap();
    let n = ring_pop(&mut pty.to_slave, out);
    if n > 0 {
        return Some(SlaveRead::Data(n));
    }
    if !pty.master_closed && crate::process::process_exists(pty.master) {
        Some(SlaveRead::WouldBlock)
    } else {
        Some(SlaveRead::Eof)
    }
}

/// Slave-side stdout write, keyed by the writing process's pid. Returns true if
/// `pid` is a pty slave and the bytes were routed to its master (so `sys_write`
/// skips the console/foreground sink); false to fall through.
pub fn slave_write_stdout(id: u32, data: &[u8]) -> Option<Result<usize, ()>> {
    let pty = table().get_mut(&id).unwrap();
    if pty.master_closed || !crate::process::process_exists(pty.master) {
        return Some(Err(()));
    }
    Some(Ok(ring_push(&mut pty.to_master, data)))
}

/// Writable only when output space exists; hangup is never optimistic success.
pub fn slave_output_readiness(id: u32) -> Option<(bool, bool)> {
    let pty = table().get(&id)?;
    let closed = pty.master_closed || !crate::process::process_exists(pty.master);
    Some((!closed && pty.to_master.len() < PTY_RING_CAP, closed))
}

/// Exit hook: drop any ptys owned by `pid` (the master died). Slave death is
/// observed lazily by `master_read` via `process_exists`, so nothing to do there.
pub fn on_process_exit(pid: ProcessId) {
    let ids: Vec<u32> = table().keys().copied().collect();
    for id in ids {
        let pty = table().get_mut(&id).unwrap();
        if pty.master == pid {
            pty.master_closed = true;
        }
        if pty.master_closed && !slave_alive(pty) {
            table().remove(&id);
        }
    }
    crate::process::wake_poll_waiters();
}

pub fn slave_dimensions(id: u32) -> Option<[u8; 8]> {
    Some(table().get(&id)?.dimensions)
}
pub fn set_slave_dimensions(id: u32, dimensions: [u8; 8]) -> bool {
    if !table().contains_key(&id) {
        return false;
    }
    table().get_mut(&id).unwrap().dimensions = dimensions;
    true
}
pub fn slave_pending_input(id: u32) -> Option<usize> {
    Some(table().get(&id)?.to_slave.len())
}

/// Read readiness observes actual bytes or a closed master, never optimistic stdin.
pub fn slave_readiness(id: u32) -> Option<(bool, bool)> {
    let pty = table().get(&id)?;
    let hung_up = pty.master_closed || !crate::process::process_exists(pty.master);
    Some((!pty.to_slave.is_empty() || hung_up, hung_up))
}
