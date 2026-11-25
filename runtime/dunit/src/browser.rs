//! Sandboxed client for the isolated `browser_netd` service using kernel IPC + shared frames.
use crate::{
    error::checked,
    fs,
    time::{self, Duration, Instant},
    Error, Result,
};
use alloc::{string::String, vec, vec::Vec};
use core::fmt;
pub use dunit_browser_net::{
    policy::ResourceIntent,
    wire::{
        ControlMessage, FetchRequest, FetchResponsePayload, ERR_DECOMPRESS_BOMB, ERR_FORBIDDEN,
        ERR_INVALID, ERR_POLICY, ERR_QUOTA, ERR_REDIRECT_LOOP, ERR_TLS_EXPIRED,
        ERR_TLS_HANDSHAKE, ERR_TLS_INVALID_CERT, ERR_TLS_UNTRUSTED, ERR_TRANSPORT,
        ERR_TRUNCATED, FLAG_BYPASS_CACHE, FLAG_CACHE_HIT, FLAG_CACHE_REPAIRED, FLAG_IS_DOWNLOAD,
        FLAG_TLS_VERIFIED, FLAG_VIA_PROXY, OP_FETCH_REQ, OP_FETCH_RESP, OP_PING, OP_STATS_REQ,
        OP_STATS_RESP, OP_TRIGGER_FAULT, SHM_HANDLE_OFFSET,
    },
};
pub use dunit_http_core::{Header, Method};
use libdunit::{self as sys, wait_set};

pub const ENDPOINT_PATH: &str = "/run/browser-netd/endpoint";
pub const CACHE_DIR: &str = "/var/lib/browser-netd/cache";
pub const COOKIES_PATH: &str = "/var/lib/browser-netd/cookies.dnck";
pub const FAULT_FLAG_PATH: &str = "/system/boot-services/browser-netd-fault.conf";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndpointInfo {
    pub pid: u32,
    pub generation: u32,
}

impl EndpointInfo {
    pub fn parse(text: &str) -> Option<Self> {
        let mut pid = None;
        let mut generation = None;
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "pid" => pid = v.trim().parse::<u32>().ok().filter(|&p| p != 0),
                    "gen" => generation = v.trim().parse::<u32>().ok(),
                    _ => {}
                }
            }
        }
        Some(Self {
            pid: pid?,
            generation: generation?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceMeta {
    pub service_pid: u32,
    pub generation: u32,
    pub flags: u32,
    pub cache_hits: u16,
    pub cache_repaired: u16,
    pub cookie_count: u16,
    pub restarts: u16,
    pub summary: String,
}

impl ServiceMeta {
    fn from_ctrl(c: &ControlMessage) -> Self {
        Self {
            service_pid: c.service_pid,
            generation: c.generation,
            flags: c.flags,
            cache_hits: c.cache_hits,
            cache_repaired: c.cache_repaired,
            cookie_count: c.cookie_count,
            restarts: c.restarts,
            summary: c.summary.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedResponse {
    pub payload: FetchResponsePayload,
    pub meta: ServiceMeta,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteError {
    pub code: i32,
    pub meta: ServiceMeta,
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "browser-net error {}: {}", self.code, self.meta.summary)
    }
}

pub enum FetchOutcome {
    Ok(FetchedResponse),
    Remote(RemoteError),
}

pub struct BrowserClient {
    next_id: u64,
    endpoint: Option<EndpointInfo>,
}

impl BrowserClient {
    pub const fn new() -> Self {
        Self {
            next_id: 1,
            endpoint: None,
        }
    }

    pub fn current_endpoint(&self) -> Option<EndpointInfo> {
        self.endpoint
    }

    pub fn discover(&mut self) -> Result<EndpointInfo> {
        let text = fs::read_to_string(ENDPOINT_PATH)?;
        let info = EndpointInfo::parse(&text).ok_or(Error(sys::EINVAL))?;
        self.endpoint = Some(info);
        Ok(info)
    }

    pub fn wait_ready(&mut self, timeout: Duration) -> Result<EndpointInfo> {
        let start = Instant::now()?;
        loop {
            if let Ok(ep) = self.discover() {
                if self.ping_target(ep.pid, Duration::from_secs(2)).is_ok() {
                    return Ok(ep);
                }
            }
            if start.elapsed()? >= timeout {
                return Err(Error(sys::EAGAIN));
            }
            time::sleep(Duration::from_millis(25))?;
        }
    }

    pub fn wait_restart(&mut self, dead_pid: u32, timeout: Duration) -> Result<EndpointInfo> {
        let start = Instant::now()?;
        loop {
            if let Ok(ep) = self.discover() {
                if ep.pid != dead_pid && self.ping_target(ep.pid, Duration::from_secs(2)).is_ok() {
                    return Ok(ep);
                }
            }
            if start.elapsed()? >= timeout {
                return Err(Error(sys::EAGAIN));
            }
            time::sleep(Duration::from_millis(30))?;
        }
    }

    pub fn ping(&mut self, timeout: Duration) -> Result<ServiceMeta> {
        let ep = self.discover_or_cached()?;
        let ctrl = self.ping_target(ep.pid, timeout)?;
        Ok(ServiceMeta::from_ctrl(&ctrl))
    }

    pub fn stats(&mut self, timeout: Duration) -> Result<ServiceMeta> {
        let ep = self.discover_or_cached()?;
        let req_id = self.alloc_id();
        let msg = ControlMessage::empty(OP_STATS_REQ, req_id).encode();
        checked(sys::ipc_send(ep.pid, &msg))?;
        let (ctrl, attachment) = self.recv_reply(ep.pid, req_id, OP_STATS_RESP, timeout)?;
        if attachment != 0 {
            let _ = sys::handle_close(attachment);
        }
        Ok(ServiceMeta::from_ctrl(&ctrl))
    }

    pub fn trigger_fault_for_test(&mut self, timeout: Duration) -> Result<()> {
        let ep = self.discover_or_cached()?;
        let req_id = self.alloc_id();
        let msg = ControlMessage::empty(OP_TRIGGER_FAULT, req_id).encode();
        let _ = sys::ipc_send(ep.pid, &msg);
        let _ = time::sleep(timeout.min(Duration::from_millis(120)));
        Ok(())
    }

    pub fn fetch(&mut self, req: &FetchRequest, timeout: Duration) -> Result<FetchOutcome> {
        let ep = self.discover_or_cached()?;
        let req_id = self.alloc_id();
        let encoded_req = req.encode();
        let shm_size = encoded_req.len().max(4096);
        let shm = checked(sys::handle_create_shared(shm_size))? as u32;
        let mapped = match checked(sys::handle_map(shm, 0, shm_size)) {
            Ok(ptr) => ptr,
            Err(err) => {
                let _ = sys::handle_close(shm);
                return Err(err);
            }
        };
        unsafe {
            core::ptr::copy_nonoverlapping(encoded_req.as_ptr(), mapped as *mut u8, encoded_req.len());
        }
        let _ = sys::vm_unmap(mapped, (shm_size + 4095) & !4095);

        let mut ctrl = ControlMessage::empty(OP_FETCH_REQ, req_id);
        ctrl.payload_len = encoded_req.len() as u32;
        if req.bypass_cache {
            ctrl.flags |= FLAG_BYPASS_CACHE;
        }
        let send_res = sys::ipc_send_handle(
            ep.pid,
            shm,
            sys::RIGHT_READ | sys::RIGHT_MAP,
            &ctrl.encode(),
            SHM_HANDLE_OFFSET,
        );
        let _ = sys::handle_close(shm);
        checked(send_res)?;

        let (reply, attachment) = self.recv_reply(ep.pid, req_id, OP_FETCH_RESP, timeout)?;
        let meta = ServiceMeta::from_ctrl(&reply);
        if reply.status_or_error < 0 {
            if attachment != 0 {
                let _ = sys::handle_close(attachment);
            }
            return Ok(FetchOutcome::Remote(RemoteError {
                code: reply.status_or_error,
                meta,
            }));
        }
        if attachment == 0 || attachment != reply.shm_handle || reply.payload_len == 0 {
            if attachment != 0 {
                let _ = sys::handle_close(attachment);
            }
            return Err(Error(sys::EINVAL));
        }
        let real_len = match checked(sys::handle_shared_len(attachment)) {
            Ok(n) => n,
            Err(err) => {
                let _ = sys::handle_close(attachment);
                return Err(err);
            }
        };
        let payload_len = reply.payload_len as usize;
        if payload_len > real_len || payload_len > 2 * 1024 * 1024 {
            let _ = sys::handle_close(attachment);
            return Err(Error(sys::EFBIG));
        }
        let ptr = match checked(sys::handle_map(attachment, 0, payload_len)) {
            Ok(p) => p,
            Err(err) => {
                let _ = sys::handle_close(attachment);
                return Err(err);
            }
        };
        let mut copy = Vec::with_capacity(payload_len);
        for i in 0..payload_len {
            copy.push(unsafe { core::ptr::read_volatile((ptr as *const u8).add(i)) });
        }
        let _ = sys::shared_buffer::release(attachment, ptr, payload_len);
        let payload = FetchResponsePayload::decode(&copy).map_err(|()| Error(sys::EINVAL))?;
        Ok(FetchOutcome::Ok(FetchedResponse { payload, meta }))
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    fn discover_or_cached(&mut self) -> Result<EndpointInfo> {
        if let Some(ep) = self.endpoint {
            return Ok(ep);
        }
        self.discover()
    }

    fn ping_target(&mut self, pid: u32, timeout: Duration) -> Result<ControlMessage> {
        let req_id = self.alloc_id();
        let msg = ControlMessage::empty(OP_PING, req_id).encode();
        checked(sys::ipc_send(pid, &msg))?;
        let (ctrl, attachment) = self.recv_reply(pid, req_id, OP_PING, timeout)?;
        if attachment != 0 {
            let _ = sys::handle_close(attachment);
        }
        Ok(ctrl)
    }

    fn recv_reply(
        &self,
        expected_pid: u32,
        expected_req_id: u64,
        expected_op: u16,
        timeout: Duration,
    ) -> Result<(ControlMessage, u32)> {
        let start = Instant::now()?;
        let mut buf = [0u8; 256];
        loop {
            let mut sender = 0u32;
            let mut attachment = 0u32;
            let got = sys::ipc_recv_handle(&mut buf, &mut sender, &mut attachment);
            if got == sys::EAGAIN {
                let elapsed = start.elapsed()?;
                if elapsed >= timeout {
                    return Err(Error(sys::EAGAIN));
                }
                let rem = timeout - elapsed;
                let step = rem.min(Duration::from_millis(50));
                let _ = wait_set::wait(
                    &[wait_set::Interest::ipc(0)],
                    &mut [wait_set::Event::default()],
                    step.as_nanos() as u64,
                );
                if fs::read_to_string(ENDPOINT_PATH)
                    .ok()
                    .and_then(|t| EndpointInfo::parse(&t))
                    .is_some_and(|ep| ep.pid != expected_pid)
                {
                    return Err(Error(sys::EPIPE));
                }
                continue;
            }
            if got < 0 {
                return Err(Error(got));
            }
            if sender != expected_pid {
                if attachment != 0 {
                    let _ = sys::handle_close(attachment);
                }
                continue;
            }
            match ControlMessage::decode(&buf[..got as usize]) {
                Ok(ctrl) if ctrl.request_id == expected_req_id && ctrl.op == expected_op => {
                    return Ok((ctrl, attachment));
                }
                _ => {
                    if attachment != 0 {
                        let _ = sys::handle_close(attachment);
                    }
                }
            }
        }
    }
}

pub fn write_pipe_frame(fd: usize, data: &[u8]) -> Result<()> {
    let hdr = (data.len() as u32).to_le_bytes();
    write_fd_all(fd, &hdr)?;
    write_fd_all(fd, data)
}

pub fn read_pipe_frame(fd: usize, max_len: usize) -> Result<Vec<u8>> {
    let mut hdr = [0u8; 4];
    read_fd_exact(fd, &mut hdr)?;
    let len = u32::from_le_bytes(hdr) as usize;
    if len == 0 || len > max_len {
        return Err(Error(sys::EINVAL));
    }
    let mut buf = vec![0u8; len];
    read_fd_exact(fd, &mut buf)?;
    Ok(buf)
}

fn write_fd_all(fd: usize, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        let n = sys::write(fd, &data[..data.len().min(4096)]);
        if n == sys::EAGAIN {
            time::sleep(Duration::from_millis(1))?;
            continue;
        }
        let wrote = checked(n)?;
        if wrote == 0 {
            return Err(Error(sys::EPIPE));
        }
        data = &data[wrote..];
    }
    Ok(())
}

fn read_fd_exact(fd: usize, mut out: &mut [u8]) -> Result<()> {
    while !out.is_empty() {
        let n = sys::read(fd, out);
        if n == sys::EAGAIN {
            let _ = wait_set::wait(
                &[wait_set::Interest::descriptor(fd as u32, wait_set::READ, 0)],
                &mut [wait_set::Event::default()],
                Duration::from_millis(50).as_nanos() as u64,
            );
            continue;
        }
        let got = checked(n)?;
        if got == 0 {
            return Err(Error(sys::EPIPE));
        }
        out = &mut out[got..];
    }
    Ok(())
}

