//! Anonymous pipes: a bounded in-kernel byte buffer with a read end and a write
//! end. Kernel = mechanism; `pipe()` and fd wiring are the policy in the syscall
//! layer. Each end is a process fd (`FdTarget::Pipe`), ref-counted so a dup of an
//! end counts and the buffer is freed only when both ends are fully closed.
//!
//! Blocking is cooperative on this UP, non-preemptible-in-kernel design: a read
//! on an empty pipe with writers still open reports `WouldBlock`, and the
//! syscall parks the caller on `WaitKey::Pipe(id)` with a syscall-restart so the
//! read re-runs on wake. EOF (all writers gone) yields 0; a write with no
//! readers yields `Broken` (EPIPE).

use alloc::collections::{BTreeMap, VecDeque};

use crate::sync::IrqSafeSpinLock;

/// Buffer high-water mark: a writer blocks (or short-writes) once this many
/// bytes are queued. 64 KiB matches the per-syscall copy cap.
const PIPE_CAPACITY: usize = 64 * 1024;

struct Pipe {
    buf: VecDeque<u8>,
    read_refs: u32,
    write_refs: u32,
}

struct Registry {
    pipes: BTreeMap<u64, Pipe>,
    next_id: u64,
}

static REGISTRY: IrqSafeSpinLock<Registry> = IrqSafeSpinLock::new(Registry {
    pipes: BTreeMap::new(),
    next_id: 1,
});

pub enum PipeRead {
    Data(usize),
    WouldBlock,
    Eof,
}

pub enum PipeWrite {
    Wrote(usize),
    WouldBlock,
    Broken,
}

/// Create a pipe with one read end and one write end. Returns its id.
pub fn create() -> u64 {
    let mut reg = REGISTRY.lock();
    let id = reg.next_id;
    reg.next_id += 1;
    reg.pipes.insert(
        id,
        Pipe {
            buf: VecDeque::new(),
            read_refs: 1,
            write_refs: 1,
        },
    );
    id
}

pub fn retain(id: u64, write_end: bool) {
    let mut reg = REGISTRY.lock();
    if let Some(pipe) = reg.pipes.get_mut(&id) {
        if write_end {
            pipe.write_refs += 1;
        } else {
            pipe.read_refs += 1;
        }
    }
}

/// Drop one reference to an end. Returns `true` if the other end now has waiters
/// that must be woken (the caller wakes `WaitKey::Pipe(id)`): closing the last
/// writer unblocks readers (they see EOF); closing the last reader unblocks
/// writers (they get EPIPE). The pipe object is freed once both ends hit zero.
pub fn release(id: u64, write_end: bool) -> bool {
    let mut reg = REGISTRY.lock();
    let drop_pipe;
    let wake;
    if let Some(pipe) = reg.pipes.get_mut(&id) {
        if write_end {
            pipe.write_refs = pipe.write_refs.saturating_sub(1);
            wake = pipe.write_refs == 0; // readers should see EOF
        } else {
            pipe.read_refs = pipe.read_refs.saturating_sub(1);
            wake = pipe.read_refs == 0; // writers should see EPIPE
        }
        drop_pipe = pipe.read_refs == 0 && pipe.write_refs == 0;
    } else {
        return false;
    }
    if drop_pipe {
        reg.pipes.remove(&id);
    }
    wake
}

/// POLLIN readiness for a read end: data buffered, or all write ends closed
/// (EOF is a readable event). A vanished pipe reads as EOF, so also ready.
pub fn is_readable(id: u64) -> bool {
    let reg = REGISTRY.lock();
    match reg.pipes.get(&id) {
        Some(pipe) => !pipe.buf.is_empty() || pipe.write_refs == 0,
        None => true,
    }
}

/// POLLOUT readiness for a write end: buffer has space, or all read ends closed
/// (a write would return EPIPE immediately — i.e. it will not block).
pub fn is_writable(id: u64) -> bool {
    let reg = REGISTRY.lock();
    match reg.pipes.get(&id) {
        Some(pipe) => pipe.read_refs == 0 || pipe.buf.len() < PIPE_CAPACITY,
        None => true,
    }
}

/// True once all write ends are closed (POLLHUP on the read end).
pub fn write_closed(id: u64) -> bool {
    let reg = REGISTRY.lock();
    match reg.pipes.get(&id) {
        Some(pipe) => pipe.write_refs == 0,
        None => true,
    }
}

/// Read into `out`. Drains up to `out.len()` bytes. Empty + writers open =>
/// WouldBlock; empty + no writers => Eof.
pub fn read(id: u64, out: &mut [u8]) -> PipeRead {
    let mut reg = REGISTRY.lock();
    let Some(pipe) = reg.pipes.get_mut(&id) else {
        return PipeRead::Eof;
    };
    if pipe.buf.is_empty() {
        return if pipe.write_refs == 0 {
            PipeRead::Eof
        } else {
            PipeRead::WouldBlock
        };
    }
    let n = core::cmp::min(out.len(), pipe.buf.len());
    for slot in out.iter_mut().take(n) {
        *slot = pipe.buf.pop_front().unwrap();
    }
    PipeRead::Data(n)
}

/// Append `data`. No readers => Broken (EPIPE). Full => WouldBlock. Otherwise
/// writes as much as fits (at least one byte) and reports the count.
pub fn write(id: u64, data: &[u8]) -> PipeWrite {
    let mut reg = REGISTRY.lock();
    let Some(pipe) = reg.pipes.get_mut(&id) else {
        return PipeWrite::Broken;
    };
    if pipe.read_refs == 0 {
        return PipeWrite::Broken;
    }
    let space = PIPE_CAPACITY.saturating_sub(pipe.buf.len());
    if space == 0 {
        return PipeWrite::WouldBlock;
    }
    let n = core::cmp::min(space, data.len());
    pipe.buf.extend(data.iter().take(n).copied());
    PipeWrite::Wrote(n)
}
