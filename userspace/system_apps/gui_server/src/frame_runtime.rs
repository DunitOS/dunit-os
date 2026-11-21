//! Serve protocol requests during frame pacing without completing an unpainted frame.
use super::*;
use crate::frame_pacing::FrameDeadline;
pub(super) fn monotonic_ns() -> Result<u64, isize> {
    let mut time = [0u64; 2];
    let status = libdunit::syscall2(
        libdunit::SYSCALL_CLOCK_GET_TIME,
        1,
        time.as_mut_ptr() as usize,
    );
    if status < 0 {
        return Err(status);
    }
    if time[1] >= 1_000_000_000 {
        return Err(libdunit::EINVAL);
    }
    time[0]
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(time[1]))
        .ok_or(libdunit::EINVAL)
}
pub(super) fn wait_for_frame(
    deadline: FrameDeadline,
    commit_wake_after: Option<FrameDeadline>,
    visible: impl Fn(u32) -> bool,
    server: &mut Server,
    clients: &mut Vec<ClientState>,
    reload: &mut bool,
    notifications: &mut Vec<alloc::string::String>,
    performance: &mut performance::Profiler,
    transfers: &mut transfer_runtime::Runtime,
) -> Result<(), isize> {
    use libdunit::wait_set::{self, Event, Interest, INFINITE, READ};
    let mut events = [Event::default(); 2];
    loop {
        let now = monotonic_ns()?;
        if deadline.due(now) {
            return Ok(());
        }
        let mut timer = deadline;
        if let Some(earliest) = commit_wake_after {
            if clients.iter().any(|c| {
                c.win_created && c.presented && c.dirty.is_some() && visible(c.pid)
            }) {
                if earliest.due(now) {
                    performance.commit_wake();
                    return Ok(());
                }
                timer.ns = timer.ns.min(earliest.ns);
            }
        }
        let interests = [Interest::ipc(0), Interest::timer(timer.ns, 1)];
        let stamp = performance.stamp();
        let result = wait_set::wait(&interests, &mut events, INFINITE);
        performance.finish(performance::Stage::Sleep, stamp);
        let count = result?;
        if events[..count]
            .iter()
            .any(|event| event.token == 0 && event.events & READ != 0)
        {
            let stamp = performance.stamp();
            pump_clients(
                server,
                clients,
                reload,
                notifications,
                performance,
                transfers,
                Some(deadline),
            )?;
            performance.finish(performance::Stage::Pump, stamp);
        }
    }
}
