//! Native capability, transport and initial composition proofs before desktop entry.
use super::*;
pub(super) fn start() -> ! {
    libdunit::println("gui_server: start");

    // 1) The M2 protocol reference server links and runs here (allocator from
    //    libdunit, no_std+alloc core). Instantiate it and admit one client so a
    //    genuine object is allocated — this exercises the crate end to end.
    let mut server = Server::new();
    let conn = match server.connect() {
        Some(conn) => {
            libdunit::println("gui_server: protocol server up (client admitted)");
            conn
        }
        None => {
            libdunit::println("gui_server: FAIL protocol server rejected first client");
            libdunit::exit(1);
        }
    };

    // 2) Acquire the exclusive display-master capability (kernel syscall 53).
    //    A single GUI server owns the display; a second acquire would fail.
    let display = libdunit::handle_display_acquire();
    if display > 0 {
        libdunit::println("gui_server: display master acquired");
    } else {
        libdunit::println("gui_server: display master unavailable");
    }

    // 3) Acquire the exclusive input-master capability (kernel syscall 55).
    //    The compositor is the sole reader of keyboard/mouse input; it fans
    //    events out to clients over the protocol. A second acquire would fail.
    let input = libdunit::handle_input_acquire();
    if input > 0 {
        libdunit::println("gui_server: input master acquired");
    } else {
        libdunit::println("gui_server: input master unavailable");
    }

    // 4) Shared-buffer capability smoke (M3 item 1): allocate a zero-copy shared
    //    frame buffer, map it writable via the handle's WRITE right, round-trip a
    //    byte pattern through the aliased physical frames, then release. This
    //    exercises the syscall path that clients will use to hand pixel buffers
    //    to the compositor.
    let buf = libdunit::handle_create_shared(4096);
    if buf > 0 {
        let mapped = libdunit::handle_map(buf as u32, 0, 4096);
        if mapped > 0 {
            let ptr = mapped as usize as *mut u8;
            let mut ok = true;
            unsafe {
                for i in 0..4096usize {
                    core::ptr::write_volatile(ptr.add(i), (i & 0xff) as u8);
                }
                for i in 0..4096usize {
                    if core::ptr::read_volatile(ptr.add(i)) != (i & 0xff) as u8 {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                libdunit::println("gui_server: shared buffer round-trip OK");
            } else {
                libdunit::println("gui_server: FAIL shared buffer mismatch");
            }
        } else {
            libdunit::println("gui_server: FAIL shared buffer map");
        }
        libdunit::shared_buffer::release(
            buf as u32,
            if mapped > 0 { mapped as usize } else { 0 },
            4096,
        );
    } else {
        libdunit::println("gui_server: FAIL shared buffer create");
    }

    // 5) Cross-process capability transfer (M3 item 1; groundwork for item 4's
    //    untrusted clients): create a shared buffer, write a marker, spawn an
    //    untrusted child, transfer a handle to it, and prove both sides see the
    //    same physical frames (bidirectional zero-copy across a process bound).
    let xbuf = libdunit::handle_create_shared(4096);
    if xbuf > 0 {
        let mapped = libdunit::handle_map(xbuf as u32, 0, 4096);
        if mapped > 0 {
            let ptr = mapped as usize as *mut u8;
            unsafe {
                core::ptr::write_volatile(ptr, 0xA5);
                core::ptr::write_volatile(ptr.add(1), 0x5A);
            }
            let peer = libdunit::spawn("gui_shbuf_peer");
            if peer > 0 {
                // Transfer a duplicate so we keep our own handle + mapping.
                let dup = libdunit::handle_dup(
                    xbuf as u32,
                    libdunit::RIGHT_READ
                        | libdunit::RIGHT_WRITE
                        | libdunit::RIGHT_MAP
                        | libdunit::RIGHT_TRANSFER,
                );
                let child_handle = if dup > 0 {
                    libdunit::handle_transfer(dup as u32, peer as u32)
                } else {
                    -1
                };
                if child_handle > 0 {
                    let mut msg = [0u8; 8];
                    msg[..4].copy_from_slice(&libdunit::get_pid().to_le_bytes());
                    msg[4..].copy_from_slice(&(child_handle as u32).to_le_bytes());
                    libdunit::ipc_send(peer as u32, &msg);
                    let mut ack = [0u8; 4];
                    if libdunit::ipc_recv_blocking(&mut ack, 0) == 4 && &ack == b"done" {
                        let seen = unsafe {
                            core::ptr::read_volatile(ptr.add(2)) == 0xC3
                                && core::ptr::read_volatile(ptr.add(3)) == 0x3C
                        };
                        if seen {
                            libdunit::println("gui_server: cross-process shared buffer OK");
                        } else {
                            libdunit::println("gui_server: FAIL child writes not visible");
                        }
                    } else {
                        libdunit::println("gui_server: FAIL child ack");
                    }
                } else {
                    libdunit::println("gui_server: FAIL capability transfer");
                }
                // Reap the child so it does not linger as a zombie.
                let mut status = libdunit::WaitStatus::empty();
                for _ in 0..50 {
                    if libdunit::wait(peer as u32, &mut status) == peer as isize {
                        break;
                    }
                    libdunit::sleep_ms(10);
                }
            } else {
                libdunit::println("gui_server: FAIL peer spawn");
            }
        }
        libdunit::shared_buffer::release(
            xbuf as u32,
            if mapped > 0 { mapped as usize } else { 0 },
            4096,
        );
    }

    // 6) Framebuffer output path (M3 item 3, first slice): as the display master,
    //    blit a small pixel block into the system framebuffer via syscall 57. The
    //    kernel gates fb_present on the display-master owner, so a non-compositor
    //    process is denied (see gui_shbuf_peer). This is the mechanism the software
    //    compositor will drive to present composited surfaces.
    let mut block = [0u8; 16 * 16 * 4];
    for px in block.chunks_exact_mut(4) {
        // Solid opaque blue (little-endian XRGB/BGRA: B,G,R,A).
        px[0] = 0xC0;
        px[1] = 0x40;
        px[2] = 0x20;
        px[3] = 0xff;
    }
    if libdunit::fb_present(&block, 16, 16, 0, 0) == 0 {
        libdunit::println("gui_server: framebuffer present OK");
    } else {
        libdunit::println("gui_server: FAIL framebuffer present");
    }

    // 7) Software compositor cycle (M3 item 3): drive the gui_protocol_v1
    //    reference server with REAL wire packets through one client's full
    //    map-and-present lifecycle, back the surface with a REAL kernel shared
    //    buffer, and composite its pixels to the framebuffer.
    //
    //    `Server` is a pure protocol state machine — it never touches pixels and
    //    exposes no surface geometry — so the compositor keeps its own
    //    buffer-id -> shared-frame mapping and its own placement policy, feeds
    //    the client's requests through `deliver`, and on the committed frame
    //    blits the mapped pixels to the framebuffer via `fb_present`. This is the
    //    exact mechanism the untrusted cross-process clients (item 4) will drive;
    //    here the request stream is synthesized in-process to keep it serially
    //    verifiable without a second protocol-speaking process.
    if run_compositor_cycle(&mut server, conn) {
        libdunit::println("gui_server: compositor cycle OK");
    } else {
        libdunit::println("gui_server: FAIL compositor cycle");
    }

    // 8) Cross-process compositing (M3 item 4): serve TWO real, untrusted
    //    client ELFs concurrently over IPC. Each renders into its own shared
    //    buffer, transfers the buffer capability to us, and drives its surface
    //    through the gui-v1 wire protocol (client→compositor messages carry a
    //    4-byte client-id envelope so we route them to the right connection).
    //    We map each transferred buffer read-only, run its requests through a
    //    per-client Server connection, and composite both surfaces to the
    //    framebuffer at separate slots. Neither client holds display/input.
    // serve_two_clients() emits "served two untrusted clients OK" itself (right
    // before it holds the composited frame on screen); only report failure here.
    if !serve_two_clients(input as u32) {
        libdunit::println("gui_server: FAIL serving clients");
    }

    libdunit::println("gui_server: OK");
    libdunit::exit(0)
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
    transfers: &mut transfer_runtime::Runtime,
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
        let mut attached=0;
        let n = libdunit::ipc_recv_handle_blocking(&mut rx,&mut sender,&mut attached,3000);
        if n <= 0 {
            break;
        }
        let n = n as usize;
        let attachment=if attached!=0{Some(unsafe{dunit_transfer::OwnedHandle::adopt(attached)})}else{None};
        let idx = match clients.iter().position(|c| c.pid == sender) {
            Some(i) => i,
            None => continue,
        };
        // A settings client's resolution self-test can present during this resize
        // proof: capture its reload ping into `reload_seen` so the first interactive
        // session re-applies the edited `[display]` rather than dropping the signal.
        // A NOTIFY posted mid-resize is still preserved (bring-up race safety).
        dispatch_client_payload(server,&mut clients[idx],&rx[..n],attachment,reload_seen,pending_notifies,transfers);
        transfers.flush();
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
