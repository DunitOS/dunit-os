//! Call-scoped wait-set. Snapshots survive syscall restart and never retain a
//! userspace pointer to the interest array or a refcount to an open pipe end.
use super::*;
use dunit_wait_abi::*;

pub(crate) struct Pending {
    interests: Vec<(Interest, u64)>,
    output: u64,
    capacity: usize,
    deadline: u64,
}

fn begin(
    input: u64,
    count: usize,
    output: u64,
    capacity: usize,
    timeout: u64,
    flags: u64,
) -> Result<Pending, i64> {
    if flags != 0 || count > MAX_INTERESTS || capacity < count || capacity > MAX_INTERESTS {
        return Err(EINVAL);
    }
    if count != 0 {
        validate_user_mapping(output, count * 24, true)?;
    }
    let bytes = if count == 0 {
        Vec::new()
    } else {
        copy_buffer_from_user(input as *const u8, count * 32)?
    };
    let mut interests = Vec::with_capacity(count);
    for bytes in bytes.chunks_exact(32) {
        let interest = Interest::decode(bytes.try_into().unwrap());
        if !interest.valid() {
            return Err(EINVAL);
        }
        let identity = match interest.kind {
            DESCRIPTOR => {
                crate::process::get_fd(interest.object as u32)
                    .ok_or(EBADF)?
                    .identity
            }
            PROCESS => {
                crate::process::child_exit_snapshot(crate::process::ProcessId(interest.object))
                    .map_err(process_error_to_errno)?;
                0
            }
            _ => 0,
        };
        interests.push((interest, identity));
    }
    let now = crate::clock::monotonic_ns();
    let deadline = if timeout == INFINITE {
        INFINITE
    } else {
        now.saturating_add(timeout).min(INFINITE - 1)
    };
    Ok(Pending {
        interests,
        output,
        capacity,
        deadline,
    })
}

pub(super) fn wait(
    input: u64,
    count: usize,
    output: u64,
    capacity: usize,
    timeout: u64,
    flags: u64,
) -> i64 {
    // UP kernel: IRQ masking spans readiness checks, snapshot install and park.
    // A producer cannot run in the check/register gap. Restart retains absolute
    // deadlines and fd slot identities, including across spurious notifications.
    let _irq = crate::sync::InterruptGuard::new();
    let saved = match crate::process::current_thread_mut() {
        Some(thread) => thread.wait_set.take(),
        None => return EINVAL,
    };
    let pending = match saved {
        Some(pending) => pending,
        None => match begin(input, count, output, capacity, timeout, flags) {
            Ok(pending) => alloc::boxed::Box::new(pending),
            Err(error) => return error,
        },
    };
    if !pending.interests.is_empty() {
        if let Err(error) =
            validate_user_mapping(pending.output, pending.interests.len() * 24, true)
        {
            return error;
        }
    }
    let now = crate::clock::monotonic_ns();
    let mut wake_deadline = pending.deadline;
    let mut results = Vec::with_capacity(pending.capacity * 24);
    for (interest, identity) in &pending.interests {
        let mut data = 0;
        let events = match interest.kind {
            IPC => {
                let pid = crate::process::current_process().unwrap().pid;
                if crate::ipc::get_ipc_manager()
                    .map(|ipc| ipc.has_messages(pid))
                    .unwrap_or(false)
                {
                    READ
                } else {
                    0
                }
            }
            DESCRIPTOR => match crate::process::get_fd(interest.object as u32) {
                Some(entry) if entry.identity == *identity => {
                    native_fd::readiness(interest.object as i32, interest.interest as u16) as u32
                }
                _ => HANGUP | ERROR,
            },
            TIMER => {
                wake_deadline = wake_deadline.min(interest.deadline_ns);
                if now >= interest.deadline_ns {
                    ELAPSED
                } else {
                    0
                }
            }
            PROCESS => match crate::process::child_exit_snapshot(crate::process::ProcessId(
                interest.object,
            )) {
                Ok(Some(status)) => {
                    data = ((status.kind as u32 as u64) << 32) | status.code as u32 as u64;
                    EXITED
                }
                Ok(None) => 0,
                // A sibling may reap after registration. Report closure rather
                // than switch to another child or consume a new exit record.
                Err(_) => HANGUP | ERROR,
            },
            _ => unreachable!(),
        };
        if events != 0 {
            results.extend_from_slice(
                &Event {
                    token: interest.token,
                    events,
                    reserved: 0,
                    data,
                }
                .encode(),
            );
        }
    }
    if !results.is_empty() || now >= pending.deadline {
        return match copy_buffer_to_user(pending.output as *mut u8, &results) {
            Ok(()) => (results.len() / 24) as i64,
            Err(error) => error,
        };
    }
    crate::process::current_thread_mut().unwrap().wait_set = Some(pending);
    match crate::process::wait_set_block_current(wake_deadline) {
        Ok(()) => USER_CONTEXT_RETURN_MAGIC,
        Err(error) => {
            crate::process::current_thread_mut().unwrap().wait_set = None;
            process_error_to_errno(error)
        }
    }
}
