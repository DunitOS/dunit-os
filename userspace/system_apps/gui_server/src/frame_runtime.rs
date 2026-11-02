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
    server: &mut Server,
    clients: &mut Vec<ClientState>,
    reload: &mut bool,
    notifications: &mut Vec<alloc::string::String>,
    performance: &mut performance::Profiler,
) -> Result<(), isize> {
    use libdunit::wait_set::{self, Event, Interest, INFINITE, READ};
    let interests = [Interest::ipc(0), Interest::timer(deadline.ns, 1)];
    let mut events = [Event::default(); 2];
    while !deadline.due(monotonic_ns()?) {
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
                Some(deadline),
            )?;
            performance.finish(performance::Stage::Pump, stamp);
        }
    }
    Ok(())
}
