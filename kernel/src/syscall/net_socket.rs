//! Native dual-stack socket proxy. Endpoint and TCP/UDP/DNS behavior belong to
//! the registered network service; the kernel attests ownership and bounds copies.
use super::*;
use crate::handle::RIGHT_NET_SERVICE;
use crate::process::{self, FdEntry, FdTarget, ProcessId};
use crate::sync::IrqSafeSpinLock;
use dunit_socket_abi::*;

mod state;
use state::{AcceptStep6, Bridge, ReadPlan, ERROR, HANGUP, MAX_UDP};

static BRIDGE: IrqSafeSpinLock<Bridge> = IrqSafeSpinLock::new(Bridge::new());

fn wire_error(_: WireError) -> i64 {
    EINVAL
}

fn caller() -> Result<u64, i64> {
    process::with_current_process(|task| task.pid.0).ok_or(EPERM)
}

fn report(result: Result<i64, i64>) -> i64 {
    result.unwrap_or_else(|error| error)
}

fn options(source: *const u8, len: usize) -> Result<u16, i64> {
    if len != OPTIONS_LEN {
        return Err(EINVAL);
    }
    Options::decode_version(&copy_buffer_from_user(source, len)?).map_err(wire_error)
}

fn install(target: FdTarget) -> Result<i64, i64> {
    let mut entry = FdEntry::new(target, 1 | 2 | 64);
    entry.cloexec = true;
    match process::allocate_fd(entry) {
        Ok(fd) => {
            process::wake_poll_waiters();
            Ok(i64::from(fd))
        }
        Err(error) => {
            close(target);
            Err(process_error_to_errno(error))
        }
    }
}

pub(super) fn register(handle: u64, source: *const u8, len: usize) -> i64 {
    report((|| {
        let handle = u32::try_from(handle).map_err(|_| EBADF)?;
        let (rights, owner) = process::with_current_process(|task| {
            task.net_device(handle)
                .map(|device| (device.map(|(_, rights)| rights), task.pid.0))
        })
        .ok_or(EPERM)?
        .map_err(handle_error_to_errno)?;
        if rights.unwrap_or(0) & RIGHT_NET_SERVICE == 0 {
            return Err(EPERM);
        }
        let version = options(source, len)?;
        let token = BRIDGE.lock().register_version(owner, version)?;
        install(FdTarget::NetService { token })
    })())
}

pub(super) fn grant(root: u64, child: u64, scope: u64) -> i64 {
    report((|| {
        let root = u32::try_from(root).map_err(|_| EBADF)?;
        let scope = u32::try_from(scope).map_err(|_| EINVAL)?;
        let allowed = SCOPE_UDP | SCOPE_TCP | SCOPE_UDP_BIND | SCOPE_TCP_LISTEN | SCOPE_LOW_PORT;
        if scope == 0 || scope & !allowed != 0 {
            return Err(EINVAL);
        }
        process::grant_net_connect(root, ProcessId(child), scope).map(i64::from)
    })())
}

pub(super) fn open(handle: u64, protocol: u64, source: *const u8, len: usize) -> i64 {
    report((|| {
        let handle = u32::try_from(handle).map_err(|_| EBADF)?;
        let protocol = u16::try_from(protocol).map_err(|_| EINVAL)?;
        let expected = match protocol {
            PROTO_TCP => SCOPE_TCP | SCOPE_TCP_LISTEN,
            PROTO_UDP => SCOPE_UDP | SCOPE_UDP_BIND,
            PROTO_DNS => SCOPE_TCP | SCOPE_UDP,
            _ => return Err(EOPNOTSUPP),
        };
        let (scope, owner) = process::with_current_process(|task| {
            task.net_connect(handle).map(|scope| (scope, task.pid.0))
        })
        .ok_or(EPERM)?
        .map_err(handle_error_to_errno)?;
        if scope & expected == 0 {
            return Err(EPERM);
        }
        let version = options(source, len)?;
        let token = BRIDGE.lock().open_version(owner, protocol, scope, version)?;
        install(FdTarget::NetSocket { token })
    })())
}

pub(crate) fn close(target: FdTarget) {
    let changed = BRIDGE.lock().close(target);
    if changed {
        process::wake_poll_waiters();
    }
}

pub(super) fn readiness(target: FdTarget) -> u16 {
    caller().map_or(ERROR | HANGUP, |owner| BRIDGE.lock().ready(owner, target))
}

fn copy_read(plan: ReadPlan, out: *mut u8) -> Result<i64, i64> {
    let copy = copy_buffer_to_user(out, plan.bytes());
    let finish = BRIDGE.lock().finish_read(&plan, copy.is_ok());
    process::wake_poll_waiters();
    copy?;
    finish?;
    Ok(plan.bytes().len() as i64)
}

fn client_read(owner: u64, token: u64, out: *mut u8, len: usize, from: bool) -> Result<i64, i64> {
    let plan = BRIDGE.lock().reserve_client(owner, token, len, from)?;
    match plan {
        Some(plan) => copy_read(plan, out),
        None => Ok(0),
    }
}

pub(super) fn read(target: FdTarget, out: *mut u8, len: usize) -> i64 {
    report((|| {
        let owner = caller()?;
        match target {
            FdTarget::NetSocket { token } => client_read(owner, token, out, len, false),
            FdTarget::NetService { token } => {
                let plan = BRIDGE.lock().reserve_service(owner, token, len)?;
                copy_read(plan, out)
            }
            _ => Err(EBADF),
        }
    })())
}

pub(super) fn write(target: FdTarget, source: *const u8, len: usize) -> i64 {
    report((|| {
        let owner = caller()?;
        match target {
            FdTarget::NetSocket { token } => {
                let protocol = BRIDGE.lock().protocol(owner, token)?;
                if (protocol == PROTO_UDP && len > MAX_UDP)
                    || (protocol == PROTO_DNS && len > MAX_DNS_NAME)
                {
                    return Err(EMSGSIZE);
                }
                let payload = copy_buffer_from_user(source, len.min(MAX_PAYLOAD))?;
                BRIDGE.lock().send6(owner, token, Endpoint6::NONE, &payload)?;
                process::wake_poll_waiters();
                Ok(payload.len() as i64)
            }
            FdTarget::NetService { token } => {
                let version = BRIDGE.lock().service_version(owner, token)?;
                let (min_len, max_len) = if version == VERSION_V2 {
                    (BRIDGE_HEADER6_LEN, MAX_FRAME_V2)
                } else {
                    (BRIDGE_HEADER_LEN, MAX_FRAME)
                };
                if !(min_len..=max_len).contains(&len) {
                    return Err(EMSGSIZE);
                }
                let frame = copy_buffer_from_user(source, len)?;
                if version == VERSION_V2 {
                    let (header, payload) = BridgeHeader6::decode(&frame).map_err(wire_error)?;
                    BRIDGE.lock().service_write6(owner, token, header, payload)?;
                } else {
                    let (header, payload) = BridgeHeader::decode(&frame).map_err(wire_error)?;
                    BRIDGE.lock().service_write6(
                        owner,
                        token,
                        BridgeHeader6::from_v1(header),
                        payload,
                    )?;
                }
                process::wake_poll_waiters();
                Ok(len as i64)
            }
            _ => Err(EBADF),
        }
    })())
}

pub(super) fn operation(
    fd: u64,
    op: u64,
    input: *const u8,
    in_len: usize,
    output: *mut u8,
    out_len: usize,
) -> i64 {
    report((|| {
        let fd = u32::try_from(fd).map_err(|_| EBADF)?;
        let Some(FdEntry {
            target: FdTarget::NetSocket { token },
            ..
        }) = process::get_fd(fd)
        else {
            return Err(EBADF);
        };
        let owner = caller()?;
        let op = u16::try_from(op).map_err(|_| EOPNOTSUPP)?;
        let version = BRIDGE.lock().socket_version(owner, token)?;
        let v2 = version == VERSION_V2;
        let ep_len = if v2 { ENDPOINT6_LEN } else { ENDPOINT_LEN };
        let eps_len = if v2 { ENDPOINTS6_LEN } else { ENDPOINTS_LEN };
        match op {
            OP_CONNECT if in_len == ep_len && out_len == 0 => {
                let bytes = copy_buffer_from_user(input, ep_len)?;
                let endpoint = unicast_version(version, &bytes)?;
                let complete = BRIDGE.lock().connect6(owner, token, endpoint)?;
                if complete {
                    Ok(0)
                } else {
                    process::wake_poll_waiters();
                    Err(EAGAIN)
                }
            }
            OP_SEND_TO if out_len == 0 => {
                if !(ep_len..=ep_len + MAX_UDP).contains(&in_len) {
                    return Err(EMSGSIZE);
                }
                let bytes = copy_buffer_from_user(input, in_len)?;
                let endpoint = unicast_version(version, &bytes[..ep_len])?;
                let payload = &bytes[ep_len..];
                BRIDGE.lock().send6(owner, token, endpoint, payload)?;
                process::wake_poll_waiters();
                Ok(payload.len() as i64)
            }
            OP_RECV_FROM if in_len == 0 => client_read(owner, token, output, out_len, true),
            OP_SHUTDOWN_WRITE if in_len == 0 && out_len == 0 => {
                let queued = BRIDGE.lock().shutdown(owner, token)?;
                if queued {
                    process::wake_poll_waiters();
                }
                Ok(0)
            }
            OP_ERROR if in_len == 0 && out_len == 4 => {
                let error = BRIDGE.lock().error(owner, token)?;
                copy_buffer_to_user(output, &error.to_le_bytes())?;
                Ok(4)
            }
            OP_ENDPOINTS if in_len == 0 && out_len == eps_len => {
                let endpoints = BRIDGE.lock().endpoints6(owner, token)?;
                if v2 {
                    copy_buffer_to_user(output, &endpoints.encode())?;
                } else {
                    let v4 = Endpoints {
                        local: endpoints.local.to_v4().ok_or(EINVAL)?,
                        remote: endpoints.remote.to_v4().ok_or(EINVAL)?,
                    };
                    copy_buffer_to_user(output, &v4.encode())?;
                }
                Ok(eps_len as i64)
            }
            OP_BIND if in_len == ep_len && out_len == 0 => {
                let bytes = copy_buffer_from_user(input, ep_len)?;
                let endpoint = if v2 {
                    Endpoint6::decode(&bytes).map_err(wire_error)?
                } else {
                    let ep = Endpoint::decode(&bytes).map_err(wire_error)?;
                    if !ep.is_bind_address() {
                        return Err(EINVAL);
                    }
                    Endpoint6::bind_from_v4(ep)
                };
                if !endpoint.is_bind_address() {
                    return Err(EINVAL);
                }
                if BRIDGE.lock().bind6(owner, token, endpoint)? {
                    Ok(0)
                } else {
                    process::wake_poll_waiters();
                    Err(EAGAIN)
                }
            }
            OP_LISTEN if in_len == 4 && out_len == 0 => {
                let bytes = copy_buffer_from_user(input, 4)?;
                let backlog = u32::from_le_bytes(bytes.try_into().map_err(|_| EINVAL)?);
                if BRIDGE.lock().listen(owner, token, backlog)? {
                    Ok(0)
                } else {
                    process::wake_poll_waiters();
                    Err(EAGAIN)
                }
            }
            OP_ACCEPT if in_len == 0 && (out_len == 0 || out_len == ep_len) => {
                match BRIDGE.lock().accept_begin6(owner, token)? {
                    AcceptStep6::Ready {
                        child_token,
                        remote,
                    } => {
                        if out_len == ep_len {
                            let encoded = if v2 {
                                let buf = remote.encode();
                                copy_buffer_to_user(output, &buf)
                            } else {
                                let ep = match remote.to_v4() {
                                    Some(ep) => ep,
                                    None => {
                                        close(FdTarget::NetSocket { token: child_token });
                                        return Err(EINVAL);
                                    }
                                };
                                copy_buffer_to_user(output, &ep.encode())
                            };
                            if let Err(error) = encoded {
                                close(FdTarget::NetSocket { token: child_token });
                                return Err(error);
                            }
                        }
                        install(FdTarget::NetSocket { token: child_token })
                    }
                    AcceptStep6::Pending => {
                        process::wake_poll_waiters();
                        Err(EAGAIN)
                    }
                    AcceptStep6::Empty => Err(EAGAIN),
                }
            }
            OP_CONNECT | OP_SEND_TO | OP_RECV_FROM | OP_SHUTDOWN_WRITE | OP_ERROR
            | OP_ENDPOINTS | OP_BIND | OP_LISTEN | OP_ACCEPT => Err(EINVAL),
            _ => Err(EOPNOTSUPP),
        }
    })())
}

fn unicast_version(version: u16, bytes: &[u8]) -> Result<Endpoint6, i64> {
    let endpoint = if version == VERSION_V2 {
        Endpoint6::decode(bytes).map_err(wire_error)?
    } else {
        Endpoint6::from_v4(Endpoint::decode(bytes).map_err(wire_error)?)
    };
    if endpoint.is_unicast() {
        Ok(endpoint)
    } else {
        Err(EINVAL)
    }
}
