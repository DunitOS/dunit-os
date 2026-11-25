//! Sandboxed GUI Response Viewer (`gui_browser`): ordinary GUI rights, zero ambient sockets.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::{format, string::String, vec};
use dunit::{
    args,
    browser::{read_pipe_frame, write_pipe_frame, BrowserClient, FetchOutcome, FetchRequest, Method, ResourceIntent},
    env, errno, process,
    time::Duration,
    ui::{Component, ComponentWindow, InputOptions, View},
    window::WindowEvent,
    Error, Result,
};
use libdunit::{self as sys, process::SpawnAction};

const W: u32 = 680;
const H: u32 = 440;
const DEFAULT_URL: &str = "https://https.du.net.test/viewer";
const MAX_BRIDGE_FRAME: usize = 1024 * 1024;
const STYLE: &str = "\
Column#win { padding: 10; gap: 6; } \
Row#bar { gap: 6; } \
Input#url { padding: 4; font-size: 13; } \
Button#go { padding: 6; font-size: 13; } \
Button#reload { padding: 6; font-size: 13; } \
Text#status { font-size: 12; color: $accent; } \
Text#headers { font-size: 11; } \
Textarea#body { padding: 6; font-size: 12; }";

fn deny_ambient_sockets() -> bool {
    let has_net_root = env::var("DUNIT_NET_ROOT").ok().flatten().is_some();
    let sock_denied = sys::net_socket_open(0, 1, &[]) < 0;
    let grant_denied = sys::net_connect_grant(0, process::id(), 3) < 0;
    let dev_denied = sys::net_device_grant(0, 0, process::id(), sys::RIGHT_READ) < 0;
    let queue_denied = sys::net_queue_open(0, &[]) < 0;
    !has_net_root && sock_denied && grant_denied && dev_denied && queue_denied
}

fn run_renderer_probe() -> Result<()> {
    if !deny_ambient_sockets() {
        let _ = dunit::println!("[BROWSER-RENDERER] FAIL ambient socket allowed");
        return Err(Error(errno::EACCES));
    }
    dunit::println!("[BROWSER-RENDERER] ambient sockets denied OK")
}

fn run_ipc_bridge() -> Result<()> {
    if !deny_ambient_sockets() {
        return Err(Error(errno::EACCES));
    }
    let req_fd = env::var("BROWSER_REQ_FD")?
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or(Error(errno::EINVAL))?;
    let resp_fd = env::var("BROWSER_RESP_FD")?
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or(Error(errno::EINVAL))?;
    let mut client = BrowserClient::new();
    let _ = client.wait_ready(Duration::from_secs(20));

    while let Ok(frame) = read_pipe_frame(req_fd, 64 * 1024) {
        let Ok(req) = FetchRequest::decode(&frame) else {
            let _ = write_pipe_frame(resp_fd, b"ERR\ninvalid request\n");
            continue;
        };
        if client.current_endpoint().is_none() {
            let _ = client.wait_ready(Duration::from_secs(10));
        }
        let encoded = 'encode: {
            match client.fetch(&req, Duration::from_secs(15)) {
            Ok(FetchOutcome::Ok(ok)) => {
                let hdr_summary = ok
                    .payload
                    .headers
                    .iter()
                    .take(4)
                    .map(|h| format!("{}: {}", h.name, h.value))
                    .collect::<alloc::vec::Vec<_>>()
                    .join(" | ");
                let body_str = core::str::from_utf8(&ok.payload.body).unwrap_or("<binary response>");
                let cache_tag = if ok.meta.flags & dunit::browser::FLAG_CACHE_HIT != 0 {
                    "HIT"
                } else {
                    "MISS"
                };
                format!(
                    "OK\nHTTP {} {} | {} | cache={} pid={} gen={} cookies={}\n{}\n{}",
                    ok.payload.status,
                    ok.payload.final_url,
                    ok.payload.tls_info,
                    cache_tag,
                    ok.meta.service_pid,
                    ok.meta.generation,
                    ok.meta.cookie_count,
                    hdr_summary,
                    body_str
                )
            }
            Ok(FetchOutcome::Remote(err)) => format!(
                "ERR\nBlocked/Error {}: {} (pid={} gen={})\n\n{}",
                err.code, err.meta.summary, err.meta.service_pid, err.meta.generation, err.meta.summary
            ),
            Err(err) => {
                if let Some(ep) = client.current_endpoint() {
                    if client.wait_restart(ep.pid, Duration::from_secs(5)).is_ok() {
                        if let Ok(FetchOutcome::Ok(ok)) = client.fetch(&req, Duration::from_secs(15)) {
                            let hdr_summary = ok
                                .payload
                                .headers
                                .iter()
                                .take(4)
                                .map(|h| format!("{}: {}", h.name, h.value))
                                .collect::<alloc::vec::Vec<_>>()
                                .join(" | ");
                            let body_str = core::str::from_utf8(&ok.payload.body).unwrap_or("<binary response>");
                            let cache_tag = if ok.meta.flags & dunit::browser::FLAG_CACHE_HIT != 0 {
                                "HIT"
                            } else {
                                "MISS"
                            };
                            break 'encode format!(
                                "OK\nHTTP {} {} | {} | cache={} pid={} gen={} cookies={}\n{}\n{}",
                                ok.payload.status,
                                ok.payload.final_url,
                                ok.payload.tls_info,
                                cache_tag,
                                ok.meta.service_pid,
                                ok.meta.generation,
                                ok.meta.cookie_count,
                                hdr_summary,
                                body_str
                            );
                        }
                    }
                }
                format!("ERR\nbrowser_netd IPC error: {:?}\n\nService unreachable", err)
            }
        }
        };
        if write_pipe_frame(resp_fd, encoded.as_bytes()).is_err() {
            break;
        }
    }
    Ok(())
}

struct BridgeChannel {
    req_w: usize,
    resp_r: usize,
    child: dunit::process::Child,
}

impl BridgeChannel {
    fn spawn() -> Result<Self> {
        let mut req_pipe = [0u32; 2];
        let mut resp_pipe = [0u32; 2];
        if sys::pipe(&mut req_pipe, 0) < 0 {
            return Err(Error(errno::EIO));
        }
        if sys::pipe(&mut resp_pipe, 0) < 0 {
            let _ = sys::close(req_pipe[0] as usize);
            let _ = sys::close(req_pipe[1] as usize);
            return Err(Error(errno::EIO));
        }
        let req_env = format!("BROWSER_REQ_FD={}", req_pipe[0]);
        let resp_env = format!("BROWSER_RESP_FD={}", resp_pipe[1]);
        let child = match dunit::process::Child::spawn_with_actions(
            "/app/gui_browser",
            &["/app/gui_browser", "--ipc-bridge"],
            &[&req_env, &resp_env],
            &[SpawnAction::Close(req_pipe[1]), SpawnAction::Close(resp_pipe[0])],
        ) {
            Ok(c) => c,
            Err(err) => {
                let _ = sys::close(req_pipe[0] as usize);
                let _ = sys::close(resp_pipe[1] as usize);
                let _ = sys::close(req_pipe[1] as usize);
                let _ = sys::close(resp_pipe[0] as usize);
                return Err(err);
            }
        };
        let _ = sys::close(req_pipe[0] as usize);
        let _ = sys::close(resp_pipe[1] as usize);
        Ok(Self {
            req_w: req_pipe[1] as usize,
            resp_r: resp_pipe[0] as usize,
            child,
        })
    }

    fn fetch(&self, url: &str, bypass_cache: bool) -> (bool, String, String, String) {
        let req = FetchRequest {
            method: Method::Get,
            intent: ResourceIntent::Document,
            bypass_cache,
            url: String::from(url),
            headers: vec![dunit::browser::Header {
                name: String::from("Accept"),
                value: String::from("text/plain, text/html"),
            }],
            body: vec![],
        };
        if write_pipe_frame(self.req_w, &req.encode()).is_err() {
            return (
                false,
                String::from("IPC pipe closed"),
                String::new(),
                String::from("Bridge unavailable"),
            );
        }
        let Ok(raw) = read_pipe_frame(self.resp_r, MAX_BRIDGE_FRAME) else {
            return (
                false,
                String::from("IPC read error"),
                String::new(),
                String::from("Bridge timed out"),
            );
        };
        let text = core::str::from_utf8(&raw).unwrap_or("ERR\ninvalid utf-8\n\n");
        let mut lines = text.splitn(4, '\n');
        let tag = lines.next().unwrap_or("ERR");
        let status = lines.next().unwrap_or("unknown");
        let headers = lines.next().unwrap_or("");
        let body = lines.next().unwrap_or("");
        (tag == "OK", String::from(status), String::from(headers), String::from(body))
    }
}

impl Drop for BridgeChannel {
    fn drop(&mut self) {
        let _ = sys::close(self.req_w);
        let _ = sys::close(self.resp_r);
        let _ = self.child.terminate_and_wait();
    }
}

struct BrowserViewer {
    url: String,
    status: String,
    headers: String,
    body: String,
    pending_fetch: Option<(String, bool)>,
}

#[derive(Clone)]
enum Msg {
    UrlChanged(String),
    Noop,
    Fetch,
    Reload,
    Loaded {
        status: String,
        headers: String,
        body: String,
    },
}

fn body_noop(_: String) -> Msg {
    Msg::Noop
}

impl Component for BrowserViewer {
    type Message = Msg;

    fn update(&mut self, message: Msg) {
        match message {
            Msg::UrlChanged(v) => self.url = v,
            Msg::Noop => {}
            Msg::Fetch => self.pending_fetch = Some((self.url.clone(), false)),
            Msg::Reload => self.pending_fetch = Some((self.url.clone(), true)),
            Msg::Loaded {
                status,
                headers,
                body,
            } => {
                self.pending_fetch = None;
                self.status = status;
                self.headers = headers;
                self.body = body;
            }
        }
    }

    fn view(&self) -> View<Msg> {
        View::column(
            "win",
            vec![
                View::row(
                    "bar",
                    vec![
                        View::input("url", "URL", &self.url, Msg::UrlChanged, InputOptions::default())
                            .on_submit(Msg::Fetch),
                        View::button("go", "Fetch", Msg::Fetch),
                        View::button("reload", "Reload", Msg::Reload),
                    ],
                ),
                View::text("status", &self.status),
                View::text("headers", &self.headers),
                View::text_area(
                    "body",
                    "Response Body",
                    &self.body,
                    body_noop,
                    InputOptions::default(),
                ),
            ],
        )
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let argv = args::args()?;
    if argv.iter().any(|a| a == "--renderer-probe") {
        return run_renderer_probe();
    }
    if argv.iter().any(|a| a == "--ipc-bridge") {
        return run_ipc_bridge();
    }

    if !deny_ambient_sockets() {
        dunit::println!("[GUI-BROWSER] FAIL ambient socket allowed")?;
        return Err(Error(errno::EACCES));
    }

    let mut probe = process::Child::spawn_with_env(
        "/app/gui_browser",
        &["/app/gui_browser", "--renderer-probe"],
        &[],
    )?;
    if !probe.wait()?.success() {
        dunit::println!("[GUI-BROWSER] FAIL renderer probe")?;
        return Err(Error(errno::EIO));
    }

    let bridge = BridgeChannel::spawn()?;
    let (ok, status, headers, body) = bridge.fetch(DEFAULT_URL, false);
    if ok {
        dunit::println!("[GUI-BROWSER] HTTPS shared-buffer response rendered")?;
    }

    let mut ui = ComponentWindow::open_themed_with_style(
        W,
        H,
        BrowserViewer {
            url: String::from(DEFAULT_URL),
            status,
            headers,
            body,
            pending_fetch: None,
        },
        STYLE,
    )?;
    dunit::println!("[GUI-BROWSER] surface presented OK")?;

    loop {
        if let Some(WindowEvent::CloseRequested) = ui.next_event(Duration::from_millis(60))? {
            break;
        }
        if let Some((target, bypass)) = ui.component().pending_fetch.clone() {
            let (_, s, h, b) = bridge.fetch(&target, bypass);
            ui.update(Msg::Loaded {
                status: s,
                headers: h,
                body: b,
            })?;
        }
    }
    ui.close()
}
