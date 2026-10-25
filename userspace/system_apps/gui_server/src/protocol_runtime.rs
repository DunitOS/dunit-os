//! Existing gui-v1 client transport, shared-buffer lifecycle and protocol checks.
use super::*;
pub(super) fn opcode_of(p: &[u8]) -> Option<Opcode> {
    Opcode::from_u16(u16::from_le_bytes([p[8], p[9]]))
}

/// Read a little-endian u32 at `off`.
pub(super) fn u32_at(p: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([p[off], p[off + 1], p[off + 2], p[off + 3]])
}

/// Read a little-endian u64 at `off`.
pub(super) fn u64_at(p: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&p[off..off + 8]);
    u64::from_le_bytes(a)
}

/// Find the first packet with `op` in a batch of outbound packets.
pub(super) fn find<'a>(batch: &'a [Vec<u8>], op: Opcode) -> Option<&'a Vec<u8>> {
    batch.iter().find(|p| opcode_of(p) == Some(op))
}

/// Drive one client's full map-and-present lifecycle through the reference
/// server with real wire packets, backing the surface with a real kernel
/// shared buffer, then composite the committed pixels to the framebuffer.
pub(super) fn run_compositor_cycle(server: &mut Server, conn: gui_protocol_v1::server::ConnId) -> bool {
    const SURFACE: u64 = 1; // client-chosen surface object id
    const BUFFER: u64 = 2; // client-chosen buffer object id
    const W: u32 = 64;
    const H: u32 = 64;
    const FMT_XRGB8888: u32 = 1;
    let bytes = (W * H * 4) as usize;

    // Back the buffer with a real zero-copy kernel shared buffer and paint a
    // recognizable gradient into the mapped frames (XRGB8888, little-endian).
    let buf = libdunit::handle_create_shared(bytes);
    if buf <= 0 {
        return false;
    }
    let buf = buf as u32;
    let mapped = libdunit::handle_map(buf, 0, bytes);
    if mapped <= 0 {
        libdunit::handle_close(buf);
        return false;
    }
    let pixels = mapped as usize as *mut u8;
    unsafe {
        for y in 0..H as usize {
            for x in 0..W as usize {
                let off = (y * W as usize + x) * 4;
                core::ptr::write_volatile(pixels.add(off), (x * 4) as u8); // B
                core::ptr::write_volatile(pixels.add(off + 1), (y * 4) as u8); // G
                core::ptr::write_volatile(pixels.add(off + 2), 0x80); // R
                core::ptr::write_volatile(pixels.add(off + 3), 0xff); // X
            }
        }
    }

    let ok = drive_protocol(server, conn, bytes, W, H, FMT_XRGB8888, SURFACE, BUFFER, pixels);
    libdunit::shared_buffer::release(buf, mapped as usize, bytes);
    ok
}

/// Feed one client's request stream through the reference server as real wire
/// packets and, on the committed frame, blit the shared buffer to the display.
/// Returns true iff every stage produced the expected reply and the framebuffer
/// present succeeded.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_protocol(
    server: &mut Server,
    conn: gui_protocol_v1::server::ConnId,
    bytes: usize,
    w: u32,
    h: u32,
    fmt: u32,
    surface: u64,
    buffer: u64,
    pixels: *const u8,
) -> bool {
    use gui_protocol_v1::wire::FEATURE_ARGB8888;

    // 1) HELLO -> WELCOME (completes connection negotiation).
    let hello = Request::Hello {
        min_minor: 0,
        max_minor: 0,
        offered_features: FEATURE_ARGB8888,
        required_features: 0,
    }
    .encode(0, 1);
    if find(&server.deliver(conn, &hello, None), Opcode::Welcome).is_none() {
        return false;
    }

    // 2) CREATE_SURFACE -> RESULT + CONFIGURE; the configure token is the
    //    CONFIGURE packet's header serial (offset 24).
    let create =
        Request::CreateSurface { role: 1, width: w, height: h, format: fmt }.encode(surface, 2);
    let out = server.deliver(conn, &create, None);
    let token = match find(&out, Opcode::Configure) {
        Some(cfg) => u64_at(cfg, 24),
        None => return false,
    };

    // 3) ACK_CONFIGURE -> RESULT.
    let ack = Request::AckConfigure { configure: token }.encode(surface, 3);
    if find(&server.deliver(conn, &ack, None), Opcode::Result).is_none() {
        return false;
    }

    // 4) IMPORT_BUFFER — cap carries the backing size in bytes -> RESULT.
    let import = Request::ImportBuffer {
        width: w,
        height: h,
        stride: w * 4,
        format: fmt,
        offset: 0,
    }
    .encode(buffer, 4);
    if find(&server.deliver(conn, &import, Some(bytes as u64)), Opcode::Result).is_none() {
        return false;
    }

    // 5) ATTACH_BUFFER (no damage) -> RESULT.
    let attach = Request::AttachBuffer { buffer, damage: Vec::new() }.encode(surface, 5);
    if find(&server.deliver(conn, &attach, None), Opcode::Result).is_none() {
        return false;
    }

    // 6) COMMIT with a frame callback -> RESULT; the surface is now mapped.
    let commit = Request::Commit { configure: token, frame_callback: 1 }.encode(surface, 6);
    if find(&server.deliver(conn, &commit, None), Opcode::Result).is_none() {
        return false;
    }

    // 7) Composition tick fires the mapped surface's frame callback:
    //    FRAME_DONE with status 0 (PRESENTED, offset 48) addressed to conn.
    let presented = server.composite().iter().any(|(c, p)| {
        *c == conn && opcode_of(p) == Some(Opcode::FrameDone) && u32_at(p, 48) == 0
    });
    if !presented {
        return false;
    }

    // 8) Blit the committed surface's pixels to the framebuffer at the
    //    compositor's chosen placement. Gated by the kernel on display master.
    let data = unsafe { core::slice::from_raw_parts(pixels, bytes) };
    if libdunit::fb_present(data, w, h, 100, 100) != 0 {
        return false;
    }

    // 8.5) Damaged second frame: commit a new buffer that differs only in a
    //      small rectangle and re-present ONLY that damaged region. Proves the
    //      compositor honors client-declared damage instead of repainting the
    //      whole surface every frame.
    if !present_damaged_frame(server, conn, surface, token, w, h, fmt) {
        return false;
    }

    // 8.75) Server-initiated resize (spec §9 reconfigure): the compositor pushes
    //       a fresh CONFIGURE at new geometry; the client must ACK the new token,
    //       re-import a buffer of the new size with a FRESH object id, and commit.
    //       This is the exact mechanism interactive maximize/fullscreen drives —
    //       synthesized in-process so the whole round trip is serially verifiable.
    if !drive_reconfigure(server, conn, surface, fmt) {
        return false;
    }

    // 9) Focus/input routing: the compositor owns the input master and fans
    //    events out to clients through the protocol's single-seat router. Give
    //    the mapped surface pointer + keyboard focus, then inject a pointer
    //    motion/button and a key press; the router must emit the corresponding
    //    client-bound events (ENTER on focus change, then BUTTON/KEY). This is
    //    the path real keyboard/mouse input from the input master will drive.
    let target = Some((conn, surface));
    let entered_ptr = server
        .set_pointer_focus(target, 10, 10)
        .iter()
        .any(|(c, p)| *c == conn && opcode_of(p) == Some(Opcode::PointerEnter));
    let _ = server.pointer_motion(12, 14);
    let got_button = server
        .pointer_button(0, true)
        .iter()
        .any(|(c, p)| *c == conn && opcode_of(p) == Some(Opcode::PointerButton));
    let entered_kbd = server
        .set_keyboard_focus(target, 0)
        .iter()
        .any(|(c, p)| *c == conn && opcode_of(p) == Some(Opcode::KeyEnter));
    let got_key = server
        .key(0x04, true, 0, false)
        .iter()
        .any(|(c, p)| *c == conn && opcode_of(p) == Some(Opcode::Key));

    let routed = entered_ptr && got_button && entered_kbd && got_key;
    if routed {
        libdunit::println("gui_server: input routing OK");
    } else {
        libdunit::println("gui_server: FAIL input routing");
    }
    routed
}

/// Commit a second buffer to `surface` whose contents differ from the first
/// only inside a small rectangle, declaring that rectangle as damage, then
/// re-present ONLY the damaged sub-rect to the framebuffer. Returns true iff the
/// protocol lifecycle and the clipped present both succeed.
pub(super) fn present_damaged_frame(
    server: &mut Server,
    conn: gui_protocol_v1::server::ConnId,
    surface: u64,
    token: u64,
    w: u32,
    h: u32,
    fmt: u32,
) -> bool {

    const BUFFER2: u64 = 3;
    let bytes = (w * h * 4) as usize;
    // Damage rectangle within the surface (compositor-space offset added below).
    let (dx, dy, dw, dh) = (8u32, 8u32, 16u32, 16u32);

    let buf = libdunit::handle_create_shared(bytes);
    if buf <= 0 {
        return false;
    }
    let buf = buf as u32;
    let mapped = libdunit::handle_map(buf, 0, bytes);
    if mapped <= 0 {
        libdunit::handle_close(buf);
        return false;
    }
    let pixels = mapped as usize as *mut u8;
    // Fill: same gradient as frame 1 everywhere, except a solid-red damaged box.
    unsafe {
        for y in 0..h as usize {
            for x in 0..w as usize {
                let off = (y * w as usize + x) * 4;
                let in_damage = (x as u32) >= dx
                    && (x as u32) < dx + dw
                    && (y as u32) >= dy
                    && (y as u32) < dy + dh;
                if in_damage {
                    core::ptr::write_volatile(pixels.add(off), 0x00); // B
                    core::ptr::write_volatile(pixels.add(off + 1), 0x00); // G
                    core::ptr::write_volatile(pixels.add(off + 2), 0xff); // R
                    core::ptr::write_volatile(pixels.add(off + 3), 0xff); // X
                } else {
                    core::ptr::write_volatile(pixels.add(off), (x * 4) as u8);
                    core::ptr::write_volatile(pixels.add(off + 1), (y * 4) as u8);
                    core::ptr::write_volatile(pixels.add(off + 2), 0x80);
                    core::ptr::write_volatile(pixels.add(off + 3), 0xff);
                }
            }
        }
    }

    let ok = drive_damage(server, conn, surface, token, w, h, fmt, BUFFER2, dx, dy, dw, dh, pixels);
    libdunit::shared_buffer::release(buf, mapped as usize, bytes);
    ok
}

/// Protocol + present half of a damaged frame: import/attach(damage)/commit the
/// new buffer, complete the frame callback, then blit only the damaged rect.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_damage(
    server: &mut Server,
    conn: gui_protocol_v1::server::ConnId,
    surface: u64,
    token: u64,
    w: u32,
    h: u32,
    fmt: u32,
    buffer: u64,
    dx: u32,
    dy: u32,
    dw: u32,
    dh: u32,
    pixels: *const u8,
) -> bool {

    let bytes = (w * h * 4) as usize;

    // IMPORT_BUFFER (serial 7) -> RESULT.
    let import = Request::ImportBuffer { width: w, height: h, stride: w * 4, format: fmt, offset: 0 }
        .encode(buffer, 7);
    if find(&server.deliver(conn, &import, Some(bytes as u64)), Opcode::Result).is_none() {
        return false;
    }
    // ATTACH_BUFFER with a single damage rect (serial 8) -> RESULT.
    let damage = {
        let mut v = Vec::new();
        v.push(gui_protocol_v1::wire::Rect { x: dx as i32, y: dy as i32, w: dw, h: dh });
        v
    };
    let attach = Request::AttachBuffer { buffer, damage }.encode(surface, 8);
    if find(&server.deliver(conn, &attach, None), Opcode::Result).is_none() {
        return false;
    }
    // COMMIT (serial 9, reuse the still-valid configure token) -> RESULT (+ the
    // previous buffer's BUFFER_RELEASE, which we don't require here).
    let commit = Request::Commit { configure: token, frame_callback: 1 }.encode(surface, 9);
    if find(&server.deliver(conn, &commit, None), Opcode::Result).is_none() {
        return false;
    }
    // Frame callback fires for the still-mapped surface.
    let presented = server.composite().iter().any(|(c, p)| {
        *c == conn && opcode_of(p) == Some(Opcode::FrameDone) && u32_at(p, 48) == 0
    });
    if !presented {
        return false;
    }

    // Blit ONLY the damaged rect: pack its strided rows out of the surface
    // buffer, then present at the surface's screen origin (100,100) + rect.
    let mut region = Vec::with_capacity((dw * dh * 4) as usize);
    for row in 0..dh as usize {
        let src_y = dy as usize + row;
        let base = (src_y * w as usize + dx as usize) * 4;
        let len = dw as usize * 4;
        let src = unsafe { core::slice::from_raw_parts(pixels.add(base), len) };
        region.extend_from_slice(src);
    }
    let ok = libdunit::fb_present(&region, dw, dh, 100 + dx, 100 + dy) == 0;
    if ok {
        libdunit::println("gui_server: damage present OK");
    } else {
        libdunit::println("gui_server: FAIL damage present");
    }
    ok
}

/// Drive a server-initiated CONFIGURE (resize) round trip in-process: push a
/// fresh CONFIGURE at new geometry via `Server::reconfigure`, then play the
/// client half a resizing app must implement — ACK the new token, back a real
/// kernel buffer of the new size, IMPORT it with a FRESH object id, ATTACH,
/// COMMIT, and present. Proves the reconfigure mechanism end to end without a
/// second process. Returns true iff every stage produced the expected reply.
pub(super) fn drive_reconfigure(
    server: &mut Server,
    conn: gui_protocol_v1::server::ConnId,
    surface: u64,
    fmt: u32,
) -> bool {
    // Resize object id: strictly greater than every id seen so far (the damage
    // frame's BUFFER2 = 3 raised the watermark), so it passes the freshness gate.
    const RESIZE_BUFFER: u64 = 4;
    const W2: u32 = 96;
    const H2: u32 = 48;
    const STATE_ACTIVATED: u32 = 1;
    let bytes = (W2 * H2 * 4) as usize;

    // Push CONFIGURE(W2, H2); its header serial (offset 24) is the new token the
    // client must ACK before its next COMMIT is accepted. `reconfigure` returns
    // conn-tagged packets (like `composite`), so pick ours out of the batch.
    let out = server.reconfigure(conn, surface, W2, H2, 1, STATE_ACTIVATED);
    let token = match out
        .iter()
        .find(|(c, p)| *c == conn && opcode_of(p) == Some(Opcode::Configure))
    {
        Some((_, cfg)) => u64_at(cfg, 24),
        None => return false,
    };

    // ACK the new configure token -> RESULT.
    let ack = Request::AckConfigure { configure: token }.encode(surface, 10);
    if find(&server.deliver(conn, &ack, None), Opcode::Result).is_none() {
        return false;
    }
    libdunit::println("gui_server: reconfigure w=96 h=48 acked");

    // Back the resized surface with a real kernel shared buffer and paint it.
    let buf = libdunit::handle_create_shared(bytes);
    if buf <= 0 {
        return false;
    }
    let buf = buf as u32;
    let mapped = libdunit::handle_map(buf, 0, bytes);
    if mapped <= 0 {
        libdunit::handle_close(buf);
        return false;
    }
    let pixels = mapped as usize as *mut u8;
    unsafe {
        for y in 0..H2 as usize {
            for x in 0..W2 as usize {
                let off = (y * W2 as usize + x) * 4;
                core::ptr::write_volatile(pixels.add(off), (x * 2) as u8); // B
                core::ptr::write_volatile(pixels.add(off + 1), (y * 4) as u8); // G
                core::ptr::write_volatile(pixels.add(off + 2), 0x40); // R
                core::ptr::write_volatile(pixels.add(off + 3), 0xff); // X
            }
        }
    }

    // IMPORT (fresh id) / ATTACH / COMMIT (new token) -> RESULT each.
    let import = Request::ImportBuffer { width: W2, height: H2, stride: W2 * 4, format: fmt, offset: 0 }
        .encode(RESIZE_BUFFER, 11);
    let imported = find(&server.deliver(conn, &import, Some(bytes as u64)), Opcode::Result).is_some();
    let attach = Request::AttachBuffer { buffer: RESIZE_BUFFER, damage: Vec::new() }.encode(surface, 12);
    let attached = imported && find(&server.deliver(conn, &attach, None), Opcode::Result).is_some();
    let commit = Request::Commit { configure: token, frame_callback: 1 }.encode(surface, 13);
    let committed = attached && find(&server.deliver(conn, &commit, None), Opcode::Result).is_some();
    let presented = committed
        && server.composite().iter().any(|(c, p)| {
            *c == conn && opcode_of(p) == Some(Opcode::FrameDone) && u32_at(p, 48) == 0
        });

    let data = unsafe { core::slice::from_raw_parts(pixels, bytes) };
    let ok = presented && libdunit::fb_present(data, W2, H2, 100, 100) == 0;
    libdunit::shared_buffer::release(buf, mapped as usize, bytes);
    if ok {
        libdunit::println("gui_server: client resized to 96x48 OK");
    } else {
        libdunit::println("gui_server: FAIL client resize");
    }
    ok
}

pub(super) use gui_protocol_v1::control::{IN_MOVE, IN_DOWN, IN_UP, IN_LEAVE, IN_KEY, IN_SCROLL, IN_QUIT};

/// How long the maximize/restore self-test waits for a client to re-import the
/// work-area-sized buffer before advancing anyway (in desktop ticks; ~60/s, so
/// ~4s). A resizable client grows well within this; a fixed-size client that
/// declines the server-pushed resize is force-settled here so the self-test —
/// and the recycle proof gated on every window reaching `st_step == 2` — never
/// stalls on a client exercising its right to keep its own size.
pub(super) const ST_SETTLE_TICKS: u32 = 240;

/// Send one input control message to a client.
pub(super) fn send_input(pid: u32, kind: u8, lx: i32, ly: i32, button: u32) {
    let msg = gui_protocol_v1::control::Input {
        kind,
        x: lx,
        y: ly,
        value: button,
    }
    .encode();
    libdunit::ipc_send(pid, &msg);
}

/// True for the modifier keys themselves (Shift/Ctrl/Alt/Super/Caps), whose
/// make/break we never forward to clients — their state already rides in every
/// other event's `mods` field. Scancodes are the `sc & 0x7F` form the kernel
/// reports (no release bit).
pub(super) fn is_modifier_scancode(sc: u8) -> bool {
    matches!(sc, 0x1D | 0x2A | 0x36 | 0x38 | 0x3A | 0x5B | 0x5C)
}

/// Per-client compositing state held by the multi-client server loop.
#[derive(Clone, Copy)]
pub(super) struct ClientState {
    pub(super) pid: u32,
    pub(super) conn: gui_protocol_v1::server::ConnId,
    pub(super) slot_x: u32,
    pub(super) slot_y: u32,
    pub(super) buf_ptr: *const u8,
    pub(super) buf_size: usize,
    pub(super) mapped_handle: u32,
    pub(super) surf_w: u32,
    pub(super) surf_h: u32,
    pub(super) presented: bool,
    /// Surface object id the client chose in CreateSurface (needed to push a
    /// server-initiated CONFIGURE via `Server::reconfigure` for resize/maximize).
    pub(super) surface: u64,
    /// Pixel format the client imported its buffer with (FORMAT_XRGB8888 = 1 or
    /// FORMAT_ARGB8888 = 2). Drives per-pixel blend vs. opaque copy at composite.
    pub(super) format: u32,
    // Desktop bring-up state (used by run_desktop_session for runtime-spawned
    // clients): `ready` once the buffer is mapped and a frame has been committed
    // so it is safe to blit; `win_created` once it owns a Win in the model.
    pub(super) ready: bool,
    pub(super) win_created: bool,
    pub(super) placement: Option<Placement>,
    /// Index into the live application registry (`settings::Applications.apps`)
    /// of the app this client runs (0xFF = unknown/not registered), used to draw
    /// a "running" indicator on the matching dock icon.
    pub(super) app: u8,
    /// Workspace (0-based) this client's window lives on, captured from the
    /// active workspace at spawn time. Startup clients default to workspace 0.
    pub(super) ws: usize,
}

impl ClientState {
    const fn empty() -> ClientState {
        ClientState {
            pid: 0,
            conn: 0,
            slot_x: 0,
            slot_y: 0,
            buf_ptr: core::ptr::null(),
            buf_size: 0,
            mapped_handle: 0,
            surf_w: 0,
            surf_h: 0,
            presented: false,
            surface: 0,
            format: FORMAT_XRGB8888,
            ready: false,
            win_created: false,
            placement: None,
            app: 0xFF,
            ws: 0,
        }
    }
}

/// Process one inbound (envelope-stripped) message for client `c`: either map
/// its announced buffer capability, or track geometry + feed the protocol
/// packet through the Server and relay the replies back to the client.
pub(super) fn handle_client_payload(
    server: &mut Server,
    c: &mut ClientState,
    payload: &[u8],
    reload: &mut bool,
    notify_out: &mut Vec<alloc::string::String>,
) {
    let n = payload.len();
    if let Some(announcement) = gui_protocol_v1::control::Capability::decode(payload) {
        let handle = announcement.handle;
        // Игнорируем размер, названный клиентом (payload[8..12]) — он недоверенный.
        // Мапим буфер и берём ИСТИННУЮ длину backing-объекта из ядра: только она
        // задаёт безопасную границу для последующего блита.
        // Query length before mapping so every successful mapping has an exact
        // release range, including rejected announcements. Re-announcing our
        // current capability is not a new owned reference.
        if handle == c.mapped_handle { return; }
        let real_len = libdunit::handle_shared_len(handle);
        if real_len <= 0 {
            libdunit::handle_close(handle);
            return;
        }
        let mapped = libdunit::handle_map(handle, 0, real_len as usize);
        if mapped <= 0 {
            libdunit::handle_close(handle);
            return;
        }
        libdunit::shared_buffer::release(c.mapped_handle, c.buf_ptr as usize, c.buf_size);
        c.buf_ptr = mapped as usize as *const u8;
        c.buf_size = real_len as usize;
        c.mapped_handle = handle;
        return;
    }
    // "Reload settings" signal from a trusted-enough client (gui_settings): the
    // config file was just rewritten, so raise a flag the desktop loop drains at
    // the top of the next frame (re-read + re-seed derived state). No payload
    // beyond the magic — the new values come from the config file, not the wire.
    if gui_protocol_v1::control::is_reload(payload) {
        *reload = true;
        return;
    }
    // "Post a notification" signal: the client names the toast text; the frame
    // loop turns it into a toast through the SAME `notify_post` path a launch
    // uses (so `[notifications]` policy still gates it). The text length is the
    // client's claim, so clamp it to the real remaining bytes AND a hard cap
    // before copying — an untrusted client must not drive an out-of-bounds read
    // or an unbounded allocation.
    if gui_protocol_v1::control::is_notification(payload) {
        if let Some(text) = gui_protocol_v1::control::notification_text(payload) {
            notify_out.push(text.into());
        }
        return;
    }
    // Заголовок wire-протокола — 32 байта; CreateSurface читает поля вплоть до
    // offset 44, поэтому короткие пакеты отбрасываем ДО индексирования (иначе
    // паника → падение всего компоситора: DoS от одного клиента).
    if n < 32 {
        return;
    }
    let op = Opcode::from_u16(u16::from_le_bytes([payload[8], payload[9]]));
    if op == Some(Opcode::CreateSurface) {
        if n < 44 {
            return;
        }
        // Surface object id is the wire header `object` (offset 16); remember it so
        // the compositor can push a server-initiated CONFIGURE for resize/maximize.
        c.surface = u64::from_le_bytes([
            payload[16], payload[17], payload[18], payload[19],
            payload[20], payload[21], payload[22], payload[23],
        ]);
        c.surf_w = u32::from_le_bytes([payload[36], payload[37], payload[38], payload[39]]);
        c.surf_h = u32::from_le_bytes([payload[40], payload[41], payload[42], payload[43]]);
        // Surface format (offset 44) selects opaque copy (XRGB) vs. per-pixel
        // blend (ARGB) at composite time; older short packets keep the default.
        if n >= 48 {
            c.format = u32::from_le_bytes([payload[44], payload[45], payload[46], payload[47]]);
        }
    }
    // A client that resizes (in response to our CONFIGURE) re-imports a buffer of
    // the new geometry with a FRESH object id. IMPORT_BUFFER carries the true
    // pixel dimensions (width@32, height@36), so track them here — this is the
    // single point where the compositor learns a surface's current size, for both
    // the initial import and every subsequent resize.
    if op == Some(Opcode::ImportBuffer) && n >= 40 {
        c.surf_w = u32::from_le_bytes([payload[32], payload[33], payload[34], payload[35]]);
        c.surf_h = u32::from_le_bytes([payload[36], payload[37], payload[38], payload[39]]);
    }
    let cap = if op == Some(Opcode::ImportBuffer) {
        Some(c.buf_size as u64)
    } else {
        None
    };
    for reply in &server.deliver(c.conn, payload, cap) {
        libdunit::ipc_send(c.pid, reply);
    }
}
/// Bring one client online: reserve a protocol connection, spawn the `exec` ELF,
/// and hand it the `[our_pid][client_id]` handshake it blocks on at startup. The
/// returned `ClientState` is *pending* — its buffer is mapped and it becomes
/// `ready` only once its protocol handshake is pumped (see `pump_clients`). The
/// `id` is a tint hint only; routing uses the kernel-authenticated sender pid.
/// `app_idx` is the caller-resolved registry index (0xFF = not in the registry),
/// stored so the dock can draw a "running" marker on the matching icon.
pub(super) fn spawn_client(
    server: &mut Server,
    id: u32,
    slot_x: u32,
    slot_y: u32,
    app_idx: u8,
    exec: &str,
) -> Option<ClientState> {
    let conn = server.connect()?;
    let pid = libdunit::spawn(exec);
    if pid <= 0 {
        return None;
    }
    let mut c = ClientState::empty();
    c.pid = pid as u32;
    c.conn = conn;
    c.slot_x = slot_x;
    c.slot_y = slot_y;
    c.app = app_idx;
    let mut hs = [0u8; 8];
    hs[0..4].copy_from_slice(&libdunit::get_pid().to_le_bytes());
    hs[4..8].copy_from_slice(&id.to_le_bytes());
    libdunit::ipc_send(c.pid, &hs);
    Some(c)
}

/// Open the app at registry index `ri` as a fresh client window on `current_ws`,
/// respecting the `max_windows` cap, and post the config-gated launch toast. This
/// is the SINGLE "launch an app" path shared by the launcher menu (mouse click +
/// keyboard Enter) and the dock — so program-launch policy lives in exactly one
/// place instead of being copied per trigger. Returns true iff a client spawned.
pub(super) fn try_launch(
    server: &mut Server,
    clients: &mut Vec<ClientState>,
    apps: &Applications,
    ri: usize,
    current_ws: usize,
    next_id: &mut u32,
    max_windows: usize,
    notifs: &mut Vec<Toast>,
    notifications: &settings::Notifications,
    now_ms: u64,
) -> bool {
    if clients.len() >= max_windows {
        return false;
    }
    let Some(entry) = apps.apps.get(ri) else {
        return false;
    };
    let exec = entry.exec.as_str();
    let name = entry.name.as_str();
    if let Some(mut c) = spawn_client(server, *next_id, 0, 0, ri as u8, exec) {
        c.ws = current_ws;
        clients.push(c);
        *next_id += 1;
        notify_post(notifs, notifications, now_ms, name);
        true
    } else {
        false
    }
}

/// Bring up the desktop: autostart the configured `[startup]` apps, prove the
/// M3 cross-process isolation invariant (≥2 untrusted clients composited
/// concurrently, routed by KERNEL-AUTHENTICATED sender pid so one cannot inject
/// into another's connection), then hand off to the interactive session. What
/// launches at boot is data (`settings::Applications::startup`), so changing the
/// startup set needs no gui_server edit. Returns true iff the isolation smoke saw
/// two clients present at once (the baseline config autostarts three).
pub(super) fn serve_two_clients() -> bool {
    let cfg = settings::load_config();
    // The display mode as configured at boot, captured BEFORE any client is spawned
    // — so a settings client that rewrites `[display]` during bring-up cannot move
    // this baseline. Threaded into the first `run_desktop_session` so a bring-up
    // resolution edit is diffed against the true boot mode (see that fn's head).
    let boot_display = cfg.settings.display;
    let mut server = Server::new();
    let mut clients: Vec<ClientState> = Vec::new();
    // Notification texts a client posts (NOTIFY_MAGIC) DURING bring-up — before the
    // interactive desktop loop and its per-tick `pump_clients` exist. An autostart
    // app can toast the moment it presents (e.g. gui_demo's "I'm up" toast), which
    // lands here in `serve_two_clients` or the `resize_one_client` resize proof, not
    // in `run_desktop_session`. Collect them instead of dropping them, and hand them
    // to the session so it emits each through the SAME config-gated `notify_post`
    // path once its toast queue exists (single source of truth; nothing is lost to a
    // bring-up race).
    let mut pending_notifies: Vec<alloc::string::String> = Vec::new();
    // A reload pinged by a client (e.g. gui_settings' resolution self-test) while it
    // presents during bring-up — captured here instead of dropped, then handed to
    // the first interactive session so the edited `[display]` actually re-applies.
    let mut bringup_reload = false;

    // Autostart the configured startup apps. Each entry is a registry index; a
    // stale/out-of-range index is skipped rather than trusted. Initial slots
    // cascade — the desktop session re-tiles them under the panel/dock anyway.
    let mut next_id = 1u32;
    for (k, &ri) in cfg.apps.startup.iter().enumerate() {
        let Some(entry) = cfg.apps.apps.get(ri) else {
            continue;
        };
        let sx = 300 + (k as u32) * 70;
        let sy = 100 + (k as u32) * 90;
        if let Some(c) = spawn_client(&mut server, next_id, sx, sy, ri as u8, entry.exec.as_str()) {
            clients.push(c);
            next_id += 1;
        }
    }
    if clients.is_empty() {
        // No startup apps (or none spawnable): still raise the shell so the
        // panel/dock/launcher are usable and can spawn apps on demand.
        libdunit::println("gui_server: no startup apps to autostart");
        // Loop so a live `[display]` change (which returns `true`) re-enters the
        // session at the new resolution. The caller owns notifications until
        // a session has actually processed them, including across mode changes.
        let mut pn = pending_notifies;
        let mut baseline = Some(boot_display);
        let mut pointer = PointerSession::default();
        while run_desktop_session(
            &mut server,
            &mut clients,
            &mut pn,
            baseline.take(),
            core::mem::take(&mut bringup_reload),
            &mut pointer,
        ) {}
        return false;
    }

    // Pump until the isolation invariant is proven: at least two untrusted
    // clients presented concurrently (or, with a single startup app, that one).
    // We break the INSTANT the target count is reached — inside the composite —
    // so any slower startup app (e.g. the terminal, which sets up a pty first)
    // stays UNpresented here and becomes a runtime-pumped window in the desktop
    // loop. That is what later fires "gui_server: desktop input ready", so the
    // keyboard-ready gate keeps working regardless of client presentation order.
    let target = clients.len().min(2);
    let mut rx = [0u8; 256];
    let mut presented_total = clients.iter().filter(|c| c.presented).count();
    'pump: for _ in 0..128 {
        if presented_total >= target {
            break;
        }
        // Route by the kernel-authenticated sender pid; a client cannot spoof
        // another's identity, so the protocol connections stay isolated.
        let mut sender: u32 = 0;
        let n = libdunit::ipc_recv_blocking_from(&mut rx, &mut sender, 3000);
        if n <= 0 {
            break;
        }
        let n = n as usize;
        let idx = match clients.iter().position(|c| c.pid == sender) {
            Some(idx) => idx,
            None => continue, // message from an unknown pid — ignore
        };
        // A settings client can present and run its resolution self-test here (a
        // bring-up race): capture its reload ping into `bringup_reload` instead of
        // dropping it, so the first interactive session re-applies the edited
        // `[display]` (see run_desktop_session's first-entry baseline). NOTIFYs
        // posted this early are still preserved in `pending_notifies`.
        handle_client_payload(&mut server, &mut clients[idx], &rx[..n], &mut bringup_reload, &mut pending_notifies);

        // A composition tick: route each FRAME_DONE to its client and blit that
        // client's committed buffer into its own slot.
        for (fc, fp) in &server.composite() {
            for c in clients.iter_mut() {
                if c.conn != *fc {
                    continue;
                }
                libdunit::ipc_send(c.pid, fp);
                let status = if fp.len() >= 52 {
                    u32::from_le_bytes([fp[48], fp[49], fp[50], fp[51]])
                } else {
                    0
                };
                if status != 0 || c.presented {
                    continue;
                }
                if !c.buf_ptr.is_null() && c.surf_w > 0 && c.surf_h > 0 {
                    // Non-wrapping bounds check against the REAL mapped length
                    // (c.buf_size came from handle_shared_len, not the client).
                    let want = c.surf_w as u64 * c.surf_h as u64 * 4;
                    if want <= c.buf_size as u64 {
                        let data =
                            unsafe { core::slice::from_raw_parts(c.buf_ptr, want as usize) };
                        if libdunit::fb_present(data, c.surf_w, c.surf_h, c.slot_x, c.slot_y) == 0 {
                            c.presented = true;
                            presented_total += 1;
                            // Stop the instant the invariant holds, leaving the
                            // rest of the startup set for the runtime pump.
                            if presented_total >= target {
                                break 'pump;
                            }
                        }
                    }
                }
            }
        }
    }

    let both = presented_total >= 2;

    // Hand off to the interactive desktop session: a persistent compositor loop
    // that owns a full-screen back buffer, decorates each client surface with a
    // draggable title bar, routes the real mouse (syscall 60) into focus/raise/
    // drag, and re-presents every tick. This is the M4 userspace DWM taking over
    // from the linear M3 smoke.
    if both {
        // Emit the isolation marker BEFORE the session loop so automated smokes
        // observe it immediately (headless runs are force-quit after capture).
        libdunit::println("gui_server: served two untrusted clients OK");
        // Cross-process resize proof: push a server-initiated CONFIGURE to one
        // live client and drive its re-negotiation to completion. This exercises
        // the maximize/fullscreen mechanism across a REAL process boundary (the
        // client re-allocates its buffer and re-imports it with a fresh object
        // id), which the in-process compositor self-test cannot cover.
        resize_one_client(&mut server, &mut clients, &mut pending_notifies, &mut bringup_reload);
    }
    // Loop so a live `[display]` change re-enters the session at the new
    // resolution (rebuilding all buffers); unprocessed notifications survive.
    let mut pn = pending_notifies;
    // The first session carries the boot display baseline + any reload pinged during
    // bring-up (serve_two_clients / resize_one_client); re-entries pass None/false
    // because the config file is authoritative by then.
    let mut baseline = Some(boot_display);
    let mut pointer = PointerSession::default();
    while run_desktop_session(
        &mut server,
        &mut clients,
        &mut pn,
        baseline.take(),
        core::mem::take(&mut bringup_reload),
        &mut pointer,
    ) {}

    for c in clients.iter() {
        if c.mapped_handle != 0 {
            libdunit::shared_buffer::release(c.mapped_handle, c.buf_ptr as usize, c.buf_size);
        }
        let mut status = libdunit::WaitStatus::empty();
        for _ in 0..50 {
            if libdunit::wait(c.pid, &mut status) == c.pid as isize {
                break;
            }
            libdunit::sleep_ms(10);
        }
    }
    both
}

/// Push a server-initiated CONFIGURE to one already-presented client and pump its
/// re-negotiation until it re-imports a fresh buffer at the new geometry and
/// re-presents. This drives the exact maximize/fullscreen mechanism across a real
/// process boundary: `Server::reconfigure` emits the CONFIGURE, the client backs
/// a new buffer and replays IMPORT/ATTACH/COMMIT with a FRESH object id, and the
/// compositor picks up the new size from the client's IMPORT_BUFFER. Verifiable
/// headlessly (the in-process self-test cannot cross an address-space boundary).
pub(super) fn resize_one_client(
    server: &mut Server,
    clients: &mut [ClientState],
    pending_notifies: &mut Vec<alloc::string::String>,
    reload_seen: &mut bool,
) {
    const NEWW: u32 = 320;
    const NEWH: u32 = 160;
    const STATE_ACTIVATED: u32 = 1;
    // First presented client that announced a surface object (a gui_client).
    let target = match clients.iter().position(|c| c.presented && c.surface != 0) {
        Some(i) => i,
        None => return,
    };
    let (conn, surface, pid) = (clients[target].conn, clients[target].surface, clients[target].pid);

    // Emit CONFIGURE(NEWW, NEWH) and forward it to the owning client.
    for (fc, fp) in &server.reconfigure(conn, surface, NEWW, NEWH, 1, STATE_ACTIVATED) {
        if *fc == conn {
            libdunit::ipc_send(pid, fp);
        }
    }

    let mut rx = [0u8; 256];
    for _ in 0..256 {
        let mut sender: u32 = 0;
        let n = libdunit::ipc_recv_blocking_from(&mut rx, &mut sender, 3000);
        if n <= 0 {
            break;
        }
        let n = n as usize;
        let idx = match clients.iter().position(|c| c.pid == sender) {
            Some(i) => i,
            None => continue,
        };
        // A settings client's resolution self-test can present during this resize
        // proof: capture its reload ping into `reload_seen` so the first interactive
        // session re-applies the edited `[display]` rather than dropping the signal.
        // A NOTIFY posted mid-resize is still preserved (bring-up race safety).
        handle_client_payload(server, &mut clients[idx], &rx[..n], reload_seen, pending_notifies);
        // Route every frame callback to its owner (keeps the other clients live).
        // Mirror `pump_clients`: a client that completes its handshake here (a
        // slower startup app such as the terminal or file manager, which finishes
        // committing while we drive the target's resize) MUST be latched `ready`,
        // exactly as the desktop pump would. Otherwise it is orphaned — presented
        // client-side but never adopted as a runtime window — because it then goes
        // idle in `run_desktop_session` and sends nothing further to re-trigger a
        // composite. This keeps the documented "slower startup app becomes a
        // runtime-pumped window" invariant true even across this resize proof.
        for (fc, fp) in &server.composite() {
            let status = if fp.len() >= 52 {
                u32::from_le_bytes([fp[48], fp[49], fp[50], fp[51]])
            } else {
                0
            };
            for c in clients.iter_mut() {
                if c.conn != *fc {
                    continue;
                }
                libdunit::ipc_send(c.pid, fp);
                if status == 0 && !c.buf_ptr.is_null() && c.surf_w > 0 && c.surf_h > 0 {
                    c.ready = true;
                }
            }
        }
        // Done once the target reports the new geometry (via its IMPORT_BUFFER)
        // with a fresh backing buffer that we can blit within bounds.
        let c = &clients[target];
        if c.surf_w == NEWW && c.surf_h == NEWH && !c.buf_ptr.is_null() {
            let want = c.surf_w as u64 * c.surf_h as u64 * 4;
            if want <= c.buf_size as u64 {
                let data = unsafe { core::slice::from_raw_parts(c.buf_ptr, want as usize) };
                if libdunit::fb_present(data, c.surf_w, c.surf_h, c.slot_x, c.slot_y) == 0 {
                    libdunit::println("gui_server: live client resized to 320x160 OK");
                    return;
                }
            }
        }
    }
    libdunit::println("gui_server: FAIL live client resize");
}

/// Push a server-initiated CONFIGURE to the client that owns `pid` and forward it
/// to that client, so it re-imports a `w`x`h` buffer (Phase 3 maximize / restore /
/// fullscreen). The client's re-negotiation (IMPORT/ATTACH/COMMIT with a fresh
/// object id) is drained by the desktop loop's `pump_clients`, which updates the
/// client's `ClientState` geometry; the per-frame Win<->ClientState sync then
/// mirrors the fresh buffer + size into the owning `Win`. A no-op for an unknown
/// pid or a client that never announced a surface object (so a fixed-size client
/// that ignores CONFIGURE is simply never asked to resize).
pub(super) fn reconfigure_client(
    server: &mut Server,
    clients: &[ClientState],
    pid: u32,
    w: u32,
    h: u32,
    state: u32,
) {
    let Some(c) = clients.iter().find(|c| c.pid == pid && c.surface != 0) else {
        return;
    };
    let (conn, surface) = (c.conn, c.surface);
    for (fc, fp) in &server.reconfigure(conn, surface, w, h, 1, state) {
        if *fc == conn {
            libdunit::ipc_send(pid, fp);
        }
    }
}
