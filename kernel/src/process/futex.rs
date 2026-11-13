//! Futex admission and clear-child-tid; task/mapping before wait queue locking.
use super::user_word::UserWord;
use super::*;

/// Outcome of a futex-style compare-and-block attempt.
pub enum FutexWaitOutcome {
    /// The word matched `expected`; the caller was parked and must return the
    /// user-context-return sentinel (it will resume on wake).
    Parked,
    /// The word did not match `expected`; the caller should report EAGAIN.
    ValueMismatch,
}

/// Record the calling thread's clear-child-tid word (0 clears it).
pub fn set_current_clear_child_tid(addr: u64) {
    if let Some(thread) = current_thread_mut() {
        thread.clear_child_tid = addr;
    }
}

/// On thread exit: if a clear-child-tid word was registered, zero it in the
/// owner's address space and wake futex waiters on it. Called from sys_exit
/// before teardown while the exiting thread is still current.
pub fn run_clear_child_tid_on_exit() {
    let addr = match current_thread() {
        Some(thread) => thread.clear_child_tid,
        None => return,
    };
    if addr == 0 {
        return;
    }
    let owner = match current_pid() {
        Some(owner) => owner,
        None => return,
    };
    let cleared = process_address_space(owner)
        .ok()
        .map(|space| UserWord::clear(&space.lock(), addr))
        .unwrap_or(false);
    if cleared {
        futex_wake(addr, 0); // wake all waiters on the word
    }
}

/// Acquire process state and the mapping before WAIT_QUEUE. Keep comparison
/// and registration under that queue lock; no task lookup or AddressSpace lock
/// acquisition is allowed while it is held. The outer IRQ boundary still makes
/// task state transitions UP-only; this is not an SMP block/cancel protocol.
/// Returns Parked when the caller was blocked,
/// ValueMismatch when the word differs from `expected` (map to EAGAIN), or an
/// error (an unmapped word yields InvalidMemoryRange -> EFAULT at the syscall).
pub fn futex_wait(
    addr: u64,
    expected: u32,
    deadline: Option<u64>,
) -> Result<FutexWaitOutcome, ProcessError> {
    let _irq = InterruptGuard::new();
    let entity = current_entity_id().ok_or(ProcessError::NoCurrentProcess)?;
    let owner = owner_of(entity).ok_or(ProcessError::NoSuchProcess)?;
    let space = process_address_space(owner)?;
    let table = process_table_mut();
    let index = process_record_index(table, entity).ok_or(ProcessError::NoSuchProcess)?;
    let record = &mut table[index];
    let process = record
        .process
        .as_mut()
        .ok_or(ProcessError::ProcessNotPrepared)?;
    let address_space = space.lock();
    let word = UserWord::new(&address_space, addr).ok_or(ProcessError::InvalidMemoryRange)?;
    // Lock order: task state -> AddressSpace -> wait registrations. The mapping
    // guard protects translation/backing through the final compare and enqueue.
    let mut queue = WAIT_QUEUE.lock();
    if word.load() != expected {
        return Ok(FutexWaitOutcome::ValueMismatch);
    }
    if record.state != ProcessState::Running {
        return Err(ProcessError::NotRunnable);
    }
    if process.is_kernel {
        return Err(ProcessError::InvalidUserContext);
    }
    unsafe {
        crate::hal::syscall_capture_user_context(&mut process.context, 0);
    }
    record.state = ProcessState::Blocked;
    process.state = ProcessState::Blocked;
    queue.push(WaitEntry {
        entity,
        key: WaitKey::Word { owner, addr },
        deadline,
    });
    crate::cpu::runtime()
        .yield_requested
        .store(true, Ordering::SeqCst);
    crate::cpu::runtime()
        .schedule_hint
        .store(0, Ordering::SeqCst);
    Ok(FutexWaitOutcome::Parked)
}

/// Wake up to `max` waiters parked on the futex word at `addr` in the current
/// address space. `max == 0` means wake all waiters. Returns the count woken.
pub fn futex_wake(addr: u64, max: usize) -> usize {
    let _irq = InterruptGuard::new();
    let Some(owner) = current_pid() else {
        return 0;
    };
    let target = WaitKey::Word { owner, addr };
    wake_matching(&WAIT_QUEUE, |entry| entry.key == target, max, wake_entry)
}
