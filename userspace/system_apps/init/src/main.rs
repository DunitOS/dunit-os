#![no_std]
#![no_main]

//! Dunit init — PID 1 of the GUI session and the userspace service manager (M5 item 3).
//!
//! In GUI mode the kernel launches `/app/init` as the foreground process instead
//! of spawning the compositor directly. init is pure userspace *policy*: the
//! kernel knows nothing about which services exist. init:
//!   1. first-boot provisioning — on a freshly installed /persist it creates the
//!      user-config root and drops a `/persist/.provisioned` marker (idempotent;
//!      skipped on every subsequent boot).
//!   2. service discovery — reads every `/system/services/*.toml` manifest
//!      (one service per file: name/exec/restart/critical/max_restarts/fallback).
//!   3. supervision — spawns each service and polls it with the non-blocking
//!      wait() syscall, applying the restart policy (never / on-failure / always)
//!      with a per-service restart budget, an optional fallback, and a CRITICAL
//!      marker when a critical service stays down.
//!
//! init must NEVER exit (it is PID 1) and must NEVER touch display/input — those
//! are acquired by the compositor via its own syscalls, independent of which
//! process is in the foreground, so `gui_server` running as init's child still
//! owns the screen. Every lifecycle event is logged with an `[INIT]` prefix so
//! tools/qemu_test.py can assert the behaviour from the serial log.

use core::panic::PanicInfo;

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use dunit::{
    fs::{self, FileType},
    io::{self, Write},
    process::{Child, ExitStatus},
    time::{self, Duration},
};

const SERVICES_DIR: &str = "/system/services";
const BOOT_SERVICES_DIR: &str = "/system/boot-services";
const BOOT_EXEC_PREFIX: &str = "/system/libexec/";
const PROVISION_MARKER: &str = "/persist/.provisioned";
const PERSIST_CONFIG_DIR: &str = "/persist/config";
const PROVISION_CONTENT: &str = "schema=1\n";
const POLL_INTERVAL_MS: u64 = 40;
const DEFAULT_MAX_RESTARTS: u32 = 5;
const MAX_SERVICES: usize = 32;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    let _ = io::stdout().write(b"[INIT] PANIC in init (PID 1)\n");
    loop {
        core::hint::spin_loop();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RestartPolicy {
    Never,
    OnFailure,
    Always,
}

enum RunState {
    Idle,
    Running(Child),
    Stopped,
}

struct Service {
    name: String,
    exec: String,
    policy: RestartPolicy,
    critical: bool,
    max_restarts: u32,
    fallback: Option<String>,
    net_device: Option<u32>,
    net_service: bool,
    net_connect: Option<u32>,
    state: RunState,
    restarts: u32,
    fallback_spawned: bool,
    fallback_child: Option<Child>,
}

// ---- small helpers ---------------------------------------------------------

/// Emit one `[INIT] <body>` log line in a SINGLE `write_stdout`. The kernel's
/// stdout mirror (sys_write -> write_stdio) appends a newline to any write that
/// does not already end in one, so a logical line split across several writes is
/// torn apart on the serial log — which would break marker matching. We build
/// the whole line (with its trailing '\n') and print it in one call.
fn emit(body: &str) {
    let mut line = String::from("[INIT] ");
    line.push_str(body);
    line.push('\n');
    let _ = io::stdout().write_all(line.as_bytes());
}

fn log(msg: &str) {
    emit(msg);
}

fn log_svc(name: &str, msg: &str) {
    let mut body = String::from(name);
    body.push_str(": ");
    body.push_str(msg);
    emit(&body);
}

fn push_u32(s: &mut String, mut v: u32) {
    if v == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    s.push_str(unsafe { core::str::from_utf8_unchecked(&buf[i..]) });
}

fn push_i32(s: &mut String, v: i32) {
    if v < 0 {
        s.push('-');
    }
    push_u32(s, v.unsigned_abs());
}

fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut v: u32 = 0;
    for c in s.bytes() {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((c - b'0') as u32)?;
    }
    Some(v)
}

// ---- manifest parsing ------------------------------------------------------

/// Parse one `/system/services/*.toml` manifest. The format is a flat set of
/// `key = value` lines (no sections); unknown keys are ignored for forward
/// compatibility. Returns `None` for a manifest with no `exec` — such a file is
/// malformed and is skipped with a warning rather than crashing init.
fn parse_service(text: &str, boot_catalog: bool) -> Option<Service> {
    let mut name = String::new();
    let mut exec = String::new();
    let mut policy = RestartPolicy::OnFailure;
    let mut critical = false;
    let mut max_restarts = DEFAULT_MAX_RESTARTS;
    let mut fallback: Option<String> = None;
    let mut net_device: Option<u32> = None;
    let mut net_service = false;
    let mut net_connect = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = match line.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let key = key.trim();
        let val = unquote(val.trim());
        match key {
            "name" => name = String::from(val),
            "exec" => exec = String::from(val),
            "restart" => {
                policy = match val {
                    "never" => RestartPolicy::Never,
                    "always" => RestartPolicy::Always,
                    _ => RestartPolicy::OnFailure,
                }
            }
            "critical" => critical = val == "true" || val == "1",
            "max_restarts" => {
                if let Some(n) = parse_u32(val) {
                    max_restarts = n;
                }
            }
            "fallback" => {
                if !val.is_empty() {
                    fallback = Some(String::from(val));
                }
            }
            "net_device" => {
                if !boot_catalog || net_device.is_some() {
                    return None;
                }
                net_device = Some(parse_u32(val)?);
            }
            "net_service" => {
                if !boot_catalog || net_service || val != "true" {
                    return None;
                }
                net_service = true;
            }
            "net_connect" => {
                if !boot_catalog || net_connect.is_some() {
                    return None;
                }
                let scope = parse_u32(val)?;
                if !(1..=3).contains(&scope) {
                    return None;
                }
                net_connect = Some(scope);
            }
            _ => {}
        }
    }

    if exec.is_empty() {
        return None;
    }
    if boot_catalog {
        let executable = exec.strip_prefix(BOOT_EXEC_PREFIX)?;
        if executable.is_empty()
            || executable == "."
            || executable == ".."
            || !executable
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            || (net_device.is_some() == net_connect.is_some())
            || (net_service && net_device.is_none())
            || fallback.is_some()
        {
            return None;
        }
    }
    if name.is_empty() {
        name = exec.clone();
    }
    Some(Service {
        name,
        exec,
        policy,
        critical,
        max_restarts,
        fallback,
        net_device,
        net_service,
        net_connect,
        state: RunState::Idle,
        restarts: 0,
        fallback_spawned: false,
        fallback_child: None,
    })
}

/// Files are processed in sorted order so the serial log is deterministic.
fn load_services(directory: &str, boot_catalog: bool, limit: usize) -> Vec<Service> {
    let mut services: Vec<Service> = Vec::new();
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => {
            if !boot_catalog {
                log("no /system/services directory");
            }
            return services;
        }
    };

    let mut names: Vec<String> = Vec::new();
    for entry in entries.iter().take(limit) {
        if entry.file_type != FileType::File {
            continue;
        }
        let nm = entry.name.as_str();
        if nm.ends_with(".toml") {
            names.push(String::from(nm));
        }
    }
    names.sort_unstable();

    for nm in &names {
        let mut path = String::from(directory);
        path.push('/');
        path.push_str(nm);
        match fs::read_to_string(&path) {
            Ok(text) => match parse_service(&text, boot_catalog) {
                Some(svc) => {
                    log_svc(&svc.name, "registered");
                    services.push(svc);
                }
                None => {
                    let mut body = String::from("skipping malformed service manifest ");
                    body.push_str(nm);
                    emit(&body);
                }
            },
            Err(_) => {
                let mut body = String::from("cannot read service manifest ");
                body.push_str(nm);
                emit(&body);
            }
        }
    }
    services
}

// ---- first-boot provisioning ----------------------------------------------

fn provision() {
    if fs::File::open(PROVISION_MARKER).is_ok() {
        log("/persist already provisioned");
        return;
    }
    log("first boot: provisioning /persist");
    // Create the persistent user-config root. mkdir is best-effort here: the
    // authoritative gate is the marker file written below, so an "already
    // exists" style failure must not abort provisioning.
    let _ = fs::create_dir(PERSIST_CONFIG_DIR);
    match fs::write(PROVISION_MARKER, PROVISION_CONTENT.as_bytes()) {
        Ok(()) => log("provisioning complete; wrote /persist/.provisioned"),
        Err(_) => log("WARN provisioning marker write failed (/persist read-only?)"),
    }
}

// ---- supervision -----------------------------------------------------------

fn spawn_service(svc: &Service) -> dunit::Result<Child> {
    if svc.net_device.is_none() && svc.net_connect.is_none() {
        return Child::spawn(&svc.exec);
    }
    let root = dunit::env::var("DUNIT_NET_ROOT")?
        .as_deref()
        .and_then(parse_u32)
        .filter(|&handle| handle != 0)
        .ok_or(dunit::Error(dunit::errno::EACCES))?;
    let mut sender = String::from("DUNIT_NET_SUPERVISOR=");
    push_u32(&mut sender, dunit::process::id());
    let mut child = Child::spawn_with_env(&svc.exec, &[&svc.exec], &[&sender, "DUNIT_NET_ROOT="])?;
    let grant = (|| {
        let handle = if let Some(interface) = svc.net_device {
            let mut rights = libdunit::RIGHT_READ | libdunit::RIGHT_WRITE | libdunit::RIGHT_NET_RAW;
            if svc.net_service {
                rights |= libdunit::RIGHT_NET_SERVICE;
            }
            libdunit::net_device_grant(root, interface, child.id(), rights)
        } else if let Some(scope) = svc.net_connect {
            libdunit::net_connect_grant(root, child.id(), scope)
        } else {
            return Err(dunit::Error(dunit::errno::EACCES));
        };
        if handle < 0 {
            return Err(dunit::Error(handle));
        }
        let handle = u32::try_from(handle)
            .ok()
            .filter(|&handle| handle != 0)
            .ok_or(dunit::Error(dunit::errno::EIO))?;
        let message = handle.to_le_bytes();
        let sent = libdunit::ipc_send(child.id(), &message);
        if sent == message.len() as isize {
            Ok(())
        } else {
            Err(dunit::Error(if sent < 0 {
                sent
            } else {
                dunit::errno::EIO
            }))
        }
    })();
    if let Err(error) = grant {
        // A failed delivery must also retire any installed handle. Keep
        // ownership on an unexpected cleanup error so supervision can reap it.
        if child.terminate_and_wait().is_err() {
            log_svc(&svc.name, "network grant failed; tracking child until exit");
            return Ok(child);
        }
        return Err(error);
    }
    log_svc(&svc.name, "network grant delivered");
    Ok(child)
}

fn start_service(svc: &mut Service) {
    let child = match spawn_service(svc) {
        Ok(child) => child,
        Err(_) => {
            // Count spawn failures against the budget so a bad exec path cannot spin.
            svc.restarts += 1;
            if svc.restarts >= svc.max_restarts {
                log_svc(&svc.name, "giving up after repeated spawn failures");
                svc.state = RunState::Stopped;
                if svc.critical {
                    log_svc(&svc.name, "CRITICAL service is down");
                }
            } else {
                log_svc(&svc.name, "spawn failed; will retry");
                svc.state = RunState::Idle;
            }
            return;
        }
    };
    let mut m = String::from("spawn pid=");
    push_u32(&mut m, child.id());
    log_svc(&svc.name, &m);
    svc.state = RunState::Running(child);
}

fn launch_fallback(svc: &mut Service) {
    if svc.fallback_spawned {
        return;
    }
    let fb = match &svc.fallback {
        Some(fb) => fb.clone(),
        None => return,
    };
    if let Ok(child) = Child::spawn(&fb) {
        svc.fallback_spawned = true;
        svc.fallback_child = Some(child);
        let mut m = String::from("launched fallback ");
        m.push_str(&fb);
        log_svc(&svc.name, &m);
    } else {
        let mut m = String::from("fallback spawn failed ");
        m.push_str(&fb);
        log_svc(&svc.name, &m);
    }
}

fn on_exit(svc: &mut Service, st: ExitStatus) {
    if st.spawn_prepared() {
        // Reaped before it ever ran. spawn() leaves children Ready, so this is
        // not expected in steady state; recover without charging the budget.
        log_svc(&svc.name, "reaped before run; restarting");
        svc.state = RunState::Idle;
        return;
    }

    on_completion(svc, st.success(), st.fault_kind().is_some(), st.raw_code());
}

fn on_completion(svc: &mut Service, success: bool, faulted: bool, code: i32) {
    if success {
        log_svc(&svc.name, "exited cleanly (code 0)");
    } else if faulted {
        log_svc(&svc.name, "faulted");
    } else {
        let mut m = String::from("exited code=");
        push_i32(&mut m, code);
        m.push_str(" (failure)");
        log_svc(&svc.name, &m);
    }

    let want_restart = match svc.policy {
        RestartPolicy::Never => false,
        RestartPolicy::Always => true,
        RestartPolicy::OnFailure => !success,
    };

    if want_restart && svc.restarts < svc.max_restarts {
        svc.restarts += 1;
        let mut m = String::from("restart (");
        push_u32(&mut m, svc.restarts);
        m.push('/');
        push_u32(&mut m, svc.max_restarts);
        m.push(')');
        log_svc(&svc.name, &m);
        svc.state = RunState::Idle;
        return;
    }

    // Terminal: policy says stop, or the restart budget is exhausted.
    if want_restart {
        log_svc(&svc.name, "restart budget exhausted");
        launch_fallback(svc);
    } else {
        log_svc(&svc.name, "stopped (restart policy)");
    }
    if svc.critical && !success {
        log_svc(&svc.name, "CRITICAL service is down");
    }
    svc.state = RunState::Stopped;
}

fn step(svc: &mut Service) {
    // Fallbacks run once and do not inherit the failed service's restart
    // policy. Retain ownership only to reap their eventual completion.
    if let Some(child) = &mut svc.fallback_child {
        if !matches!(child.try_wait(), Ok(None)) {
            svc.fallback_child = None;
        }
    }
    match &mut svc.state {
        RunState::Idle => start_service(svc),
        RunState::Running(child) => match child.try_wait() {
            Ok(None) => {} // still running
            Ok(Some(status)) => on_exit(svc, status),
            Err(_) => {
                // ECHILD/ENOENT or other unexpected error: the child is gone.
                // Preserve the native supervisor's synthetic failure fault.
                log_svc(&svc.name, "wait error; treating as failure");
                on_completion(svc, false, true, -1);
            }
        },
        RunState::Stopped => {}
    }
}

// PID 1 retains an explicit entry and panic handler: the ordinary SDK entry
// exits on return/error/panic, while init must never terminate. All service I/O,
// filesystem access, child ownership and polling time use the SDK lower layers.
#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: libdunit::RawArgv, envp: libdunit::RawEnvp) -> ! {
    libdunit::init_runtime(argc, argv, envp);
    log("start: PID 1 userspace service manager");
    provision();

    let mut boot_services = load_services(BOOT_SERVICES_DIR, true, MAX_SERVICES);
    let services = load_services(SERVICES_DIR, false, MAX_SERVICES - boot_services.len());
    if services.is_empty() && boot_services.is_empty() {
        log("no services registered; idling");
    }
    if !services.is_empty() {
        let mut m = String::from("supervising ");
        push_u32(&mut m, services.len() as u32);
        m.push_str(" service(s)");
        log(&m);
    }
    if !boot_services.is_empty() {
        let mut m = String::from("supervising ");
        push_u32(&mut m, boot_services.len() as u32);
        m.push_str(" boot service(s)");
        log(&m);
    }
    boot_services.extend(services);

    loop {
        for svc in &mut boot_services {
            step(svc);
        }
        let _ = time::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}
