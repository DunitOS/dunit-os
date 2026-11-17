//! Bounded IPC service shared by active frames and idle pacing.
use super::*;
/// Drain a bounded batch of messages routed by authenticated sender PID.
pub(super) fn pump_clients(
    server: &mut Server,
    clients: &mut Vec<ClientState>,
    reload: &mut bool,
    notify_out: &mut Vec<alloc::string::String>,
    performance: &mut performance::Profiler,
    transfers: &mut transfer_runtime::Runtime,
    deadline: Option<frame_pacing::FrameDeadline>,
) -> Result<(), isize> {
    let mut rx = [0u8; 256];
    let mut sender: u32 = 0;
    for _ in 0..64 {
        if let Some(deadline) = deadline {
            if deadline.due(frame_runtime::monotonic_ns()?) {
                break;
            }
        }
        let mut attached = 0;
        let n = libdunit::ipc_recv_handle(&mut rx, &mut sender, &mut attached);
        if n == libdunit::EAGAIN {
            break;
        }
        if n < 0 {
            return Err(n);
        }
        let n = n as usize;
        performance.ipc_packet();
        let attachment = if attached != 0 {
            Some(unsafe { dunit_transfer::OwnedHandle::adopt(attached) })
        } else {
            None
        };
        let idx = match clients.iter().position(|c| c.pid == sender) {
            Some(i) => i,
            None => continue, // message from an unknown pid — ignore
        };
        dispatch_client_payload(
            server,
            &mut clients[idx],
            &rx[..n],
            attachment,
            reload,
            notify_out,
            transfers,
        );
        transfers.flush();
    }
    Ok(())
}
