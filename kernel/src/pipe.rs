//! Anonymous pipes: a bounded in-kernel byte buffer with a read end and a write
//! end. Kernel = mechanism; `pipe()` and fd wiring are the policy in the syscall
//! layer. Each end is a process fd (`FdTarget::Pipe`), ref-counted so a dup of an
//! end counts and the buffer is freed only when both ends are fully closed.
//!
//! A read on an empty pipe with writers still open reports `WouldBlock`, and the
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
    let Some(pipe) = reg.pipes.get_mut(&id) else {
        return false;
    };
    let refs = if write_end {
        &mut pipe.write_refs
    } else {
        &mut pipe.read_refs
    };
    *refs = refs.saturating_sub(1);
    let wake = *refs == 0;
    let retired = if pipe.read_refs == 0 && pipe.write_refs == 0 {
        reg.pipes.remove(&id)
    } else {
        None
    };
    drop(reg);
    drop(retired);
    wake
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
    PipeRead::Data(crate::byte_ring::pop(&mut pipe.buf, out))
}

/// Append `data`. No readers => Broken (EPIPE). Full => WouldBlock. Otherwise
/// writes as much as fits (at least one byte) and reports the count.
pub fn write(id: u64, data: &[u8]) -> PipeWrite {
    let mut spare = VecDeque::new();
    loop {
        let capacity = {
            let mut reg = REGISTRY.lock();
            let Some(pipe) = reg.pipes.get_mut(&id) else {
                return PipeWrite::Broken;
            };
            if pipe.read_refs == 0 {
                return PipeWrite::Broken;
            }
            if pipe.buf.len() == PIPE_CAPACITY {
                return PipeWrite::WouldBlock;
            }
            match crate::byte_ring::push(&mut pipe.buf, data, PIPE_CAPACITY, &mut spare) {
                Ok(n) => return PipeWrite::Wrote(n),
                Err(capacity) => capacity,
            }
        };
        // Recheck endpoint state and free space after allocating.
        spare = VecDeque::with_capacity(capacity);
    }
}

/// Non-consuming readiness under the same registry lock as I/O.
pub fn readiness(id: u64, write: bool) -> (bool, bool) {
    let registry = REGISTRY.lock();
    match registry.pipes.get(&id) {
        Some(pipe) if write => (pipe.read_refs != 0 && pipe.buf.len() < PIPE_CAPACITY, pipe.read_refs == 0),
        Some(pipe) => (!pipe.buf.is_empty() || pipe.write_refs == 0, pipe.write_refs == 0),
        None => (false, true),
    }
}
