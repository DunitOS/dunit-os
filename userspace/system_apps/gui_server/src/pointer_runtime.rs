//! Authenticated pointer-mode requests and compositor-owned revocation.
use super::*;
use gui_protocol_v1::{
    control::{PointerMode, PointerRequest, IN_POINTER_MODE},
    wire::FEATURE_POINTER_MODE,
};
#[derive(Default)]
pub(super) struct Runtime {
    pub state: pointer_mode::State,
    cancelled: bool,
}
impl Runtime {
    pub fn take_cancelled(&mut self) -> bool {
        core::mem::take(&mut self.cancelled)
    }
    pub fn hidden(&self) -> bool {
        self.state.mode == PointerMode::Locked
    }
    pub fn owner(&self) -> Option<u32> {
        self.state.owner
    }
    pub fn bounds(&self, wins: &[Win], screen: (usize, usize)) -> (i32, i32, i32, i32) {
        self.owner()
            .and_then(|pid| {
                wins.iter()
                    .find(|w| w.pid == pid && w.alive && !w.minimized)
            })
            .map(|w| {
                let x = w.cx.max(0).min(screen.0.saturating_sub(1) as i32);
                let y = w.cy.max(0).min(screen.1.saturating_sub(1) as i32);
                (
                    x,
                    y,
                    (w.cx.saturating_add(w.sw).min(screen.0 as i32) - x).max(1),
                    (w.cy.saturating_add(w.sh).min(screen.1 as i32) - y).max(1),
                )
            })
            .unwrap_or((0, 0, screen.0 as i32, screen.1 as i32))
    }
    pub fn revoke(&mut self, clients: &mut [ClientState]) {
        if let Some(pid) = self.state.release() {
            self.cancelled = true;
            notify(clients, pid, 0, PointerMode::Normal, libdunit::EINTR);
        }
    }
    pub fn reconcile(&mut self, clients: &mut [ClientState], focus: Option<u32>) {
        if let Some(pid) = self.state.reconcile(focus) {
            self.cancelled = true;
            notify(clients, pid, 0, PointerMode::Normal, libdunit::EINTR);
        }
    }
    pub fn service(&mut self, clients: &mut [ClientState], focus: Option<u32>) {
        self.reconcile(clients, focus);
        // A single bounded pending request per authenticated client.
        for i in 0..clients.len() {
            let Some(r) = clients[i].pointer_request.take() else {
                continue;
            };
            let pid = clients[i].pid;
            let previous = self.owner();
            let status = self.state.request(pid, focus, r.mode).err().unwrap_or(0);
            if previous.is_some() && self.owner().is_none() {
                self.cancelled = true;
            }
            let effective = if self.owner() == Some(pid) {
                self.state.mode
            } else {
                PointerMode::Normal
            };
            notify(clients, pid, r.request, effective, status);
        }
    }
}
fn notify(clients: &mut [ClientState], pid: u32, request: u32, mode: PointerMode, status: isize) {
    if let Some(c) = clients.iter_mut().find(|c| c.pid == pid) {
        if c.pointer_effective != PointerMode::Normal && mode == PointerMode::Normal {
            let leave = gui_protocol_v1::control::Input {
                kind: IN_LEAVE,
                x: 0,
                y: 0,
                value: 0,
            };
            libdunit::ipc_send(pid, &leave.encode());
        }
        c.pointer_effective = mode;
    }
    send_input(
        clients,
        pid,
        IN_POINTER_MODE,
        request as i32,
        status as i32,
        mode as u32,
    );
}
pub(super) fn receive(c: &mut ClientState, payload: &[u8]) -> bool {
    if !PointerRequest::is_message(payload) {
        return false;
    }
    let Some(r) = PointerRequest::decode(payload) else {
        return true;
    };
    if c.features & FEATURE_POINTER_MODE == 0 {
        return true;
    }
    if c.pointer_request.is_some() {
        let input = gui_protocol_v1::control::Input {
            kind: IN_POINTER_MODE,
            x: r.request as i32,
            y: libdunit::EBUSY as i32,
            value: c.pointer_effective as u32,
        };
        libdunit::ipc_send(c.pid, &input.encode());
    } else {
        c.pointer_request = Some(r);
    }
    true
}
