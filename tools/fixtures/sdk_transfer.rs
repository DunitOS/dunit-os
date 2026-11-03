extern crate alloc;
use alloc::{string::String, vec};
use dunit::io::Write;
use dunit::{
    time::Duration,
    transfer::{self, Action, Format, Kind, Offer, Payload, Status},
    window::{Window, WindowEvent, IN_DOWN, IN_FOCUS_OUT, IN_KEY, IN_LEAVE, IN_MOVE, IN_UP},
};
fn native(error: transfer::Error) -> dunit::Error {
    dunit::Error(error.0)
}
fn text() -> String {
    "clipboard: Привет 👩‍💻 e\u{301}\n".repeat(1024)
}
fn offer() -> Offer {
    Offer {
        formats: vec![
            Format::Custom(String::from("application/x-m77-proof")),
            Format::Rgba,
            Format::Files,
            Format::Text,
        ],
        actions: 7,
        preferred: Action::Copy,
    }
}
fn file_payload() -> dunit::Result<Payload> {
    dunit::fs::write("/tmp/m77-file", b"anchored original")?;
    let file = transfer::File::open("/tmp/m77-file", false).map_err(native)?;
    Payload::from_files(vec![file]).map_err(native)
}
fn binary_payload() -> dunit::Result<Payload> {
    let rgba = transfer::Rgba {
        width: 2,
        height: 1,
        pixels: vec![255, 0, 0, 255, 0, 128, 255, 64],
    }
    .encode()
    .unwrap();
    Payload::new(
        transfer::Bundle {
            representations: vec![
                transfer::Representation {
                    format: Format::Rgba,
                    data: rgba,
                },
                transfer::Representation {
                    format: Format::Custom(String::from("application/x-m77-proof")),
                    data: vec![0, 255, 17, 0, 128],
                },
            ],
        },
        vec![],
    )
    .map_err(native)
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let identity = dunit::env::var("DUNIT_APP_ID")?.unwrap();
    assert!(identity == "sdk_transfer_fixture" || identity == "sdk_transfer_target");
    let source = identity == "sdk_transfer_fixture";
    let mut window = Window::open(320, 180)?;
    window.draw(|frame| {
        frame
            .pixels_mut()
            .fill(if source { 0xff304050 } else { 0xff405030 })
    })?;
    let mut published = None;
    let mut read = None;
    let mut reads = 0;
    let mut dragging = None;
    let mut press = None;
    let mut drag_requests = 0;
    let mut file_mode = false;
    let mut binary_mode = 0u8;
    let mut drag_count = 0;
    let mut drag_focus_losses = 0;
    let mut acceptance = None;
    let mut drop = None;
    let mut opening = None;
    let mut was_opener = false;
    if !source {
        window
            .transfers()?
            .register_target(offer())
            .map_err(native)?;
        window
            .transfers()?
            .register_opener(Offer {
                formats: vec![Format::Files],
                actions: 1,
                preferred: Action::Copy,
            })
            .map_err(native)?;
    }
    if source {
        dunit::println!("[TRANSFER] source ready")?;
    } else {
        dunit::println!("[TRANSFER] target ready")?;
    }
    loop {
        match window.next_event(Duration::from_millis(20))? {
            Some(WindowEvent::CloseRequested) => break,
            Some(WindowEvent::Resized { .. }) => window.draw(|frame| {
                frame
                    .pixels_mut()
                    .fill(if source { 0xff304050 } else { 0xff405030 })
            })?,
            Some(WindowEvent::Input(input)) if source && input.kind == IN_FOCUS_OUT => {
                if dragging.is_some() {
                    drag_focus_losses += 1;
                }
            }
            Some(WindowEvent::Input(input)) if input.kind == IN_KEY => {
                if input.value == b'i' as u32 || input.value == b'k' as u32 {
                    binary_mode = if input.value == b'i' as u32 { 1 } else { 2 };
                    dunit::println!("[TRANSFER] binary preference ready")?;
                } else if source && input.value == b'f' as u32 {
                    binary_mode = 0;
                    file_mode = true;
                    dunit::println!("[TRANSFER] files selected")?;
                } else if input.value == b't' as u32 {
                    binary_mode = 0;
                    file_mode = false;
                    dunit::println!("[TRANSFER] text selected")?;
                } else if source && input.x == 0x2e && input.y as u8 & 2 != 0 {
                    let payload = if binary_mode != 0 {
                        binary_payload()?
                    } else if file_mode {
                        file_payload()?
                    } else {
                        Payload::text(&text()).map_err(native)?
                    };
                    published = Some(
                        window
                            .transfers()?
                            .publish_clipboard(payload, 1, Action::Copy)
                            .map_err(native)?,
                    );
                } else if !source && input.x == 0x2f && input.y as u8 & 2 != 0 {
                    read = Some(
                        window
                            .transfers()?
                            .read_clipboard(if binary_mode != 0 {
                                Offer {
                                    formats: vec![if binary_mode == 1 {
                                        Format::Rgba
                                    } else {
                                        Format::Custom(String::from("application/x-m77-proof"))
                                    }],
                                    actions: 1,
                                    preferred: Action::Copy,
                                }
                            } else {
                                offer()
                            })
                            .map_err(native)?,
                    );
                } else if input.value == b'q' as u32 {
                    break;
                }
            }
            Some(WindowEvent::Input(input))
                if source && input.kind == IN_DOWN && input.pointer_button() == 0 =>
            {
                press = Some((input.x, input.y));
            }
            Some(WindowEvent::Input(input)) if source && matches!(input.kind, IN_UP | IN_LEAVE) => {
                press = None;
            }
            Some(WindowEvent::Input(input)) if source && input.kind == IN_MOVE => {
                if let Some((x, y)) = press {
                    let threshold = dwm_settings::load().transfers.drag_threshold_px as i64;
                    let dx = i64::from(input.x) - i64::from(x);
                    let dy = i64::from(input.y) - i64::from(y);
                    if dx * dx + dy * dy >= threshold * threshold {
                        drag_requests = window.transfers()?.provider_requests();
                        drag_count += 1;
                        drag_focus_losses = 0;
                        let payload = if binary_mode != 0 {
                            binary_payload()?
                        } else if file_mode {
                            file_payload()?
                        } else {
                            Payload::text(&text()).map_err(native)?
                        };
                        dragging = Some(
                            window
                                .transfers()?
                                .start_drag(
                                    payload,
                                    if binary_mode != 0 { 4 } else { 1 },
                                    if binary_mode != 0 {
                                        Action::Link
                                    } else {
                                        Action::Copy
                                    },
                                )
                                .map_err(native)?,
                        );
                        press = None;
                    }
                }
            }
            Some(WindowEvent::Transfer(event)) => {
                let m = event.message;
                if source && m.kind == Kind::Result && published == Some(m.request) {
                    assert_eq!(m.arg0, 0);
                    published = None;
                    if binary_mode != 0 {
                        dunit::println!("[TRANSFER] image and custom snapshot published")?;
                    } else if file_mode {
                        dunit::fs::rename("/tmp/m77-file", "/tmp/m77-renamed")?;
                        dunit::fs::write("/tmp/m77-file", b"replacement must not be copied")?;
                        dunit::println!("[TRANSFER] files snapshot published and renamed")?;
                    } else {
                        dunit::println!("[TRANSFER] text snapshot published")?;
                    }
                } else if source && m.kind == Kind::Result && dragging == Some(m.request) {
                    assert_eq!(m.arg0, 0);
                    dunit::io::stdout().write_all(
                        alloc::format!("[TRANSFER] drag started {}\n", drag_count).as_bytes(),
                    )?;
                } else if source
                    && event.source
                    && m.kind == Kind::Cancelled
                    && dragging == Some(m.request)
                {
                    assert!(
                        m.arg0 == Status::Cancelled as u32 || m.arg0 == Status::Unsupported as u32
                    );
                    assert_eq!(window.transfers()?.provider_requests(), drag_requests);
                    dragging = None;
                    if m.arg0 == Status::Unsupported as u32 {
                        dunit::println!("[TRANSFER] unsupported drop never requested data")?;
                    } else {
                        dunit::println!("[TRANSFER] escape cancelled without requesting data")?;
                    }
                } else if source
                    && event.source
                    && m.kind == Kind::Finished
                    && dragging == Some(m.request)
                {
                    assert_eq!(m.arg0, Status::Ok as u32);
                    assert_eq!(window.transfers()?.provider_requests(), drag_requests + 1);
                    if binary_mode != 0 {
                        assert!(
                            drag_focus_losses >= 2,
                            "workspace leave/return plus target focus"
                        );
                        dunit::println!(
                            "[TRANSFER] drag crossed workspaces without losing capture"
                        )?;
                    }
                    dragging = None;
                    dunit::println!("[TRANSFER] drop completed with one provider request")?;
                } else if !source && m.kind == Kind::Finished && opening == Some(m.transfer) {
                    assert_eq!(m.arg0, Status::Ok as u32);
                    break;
                } else if !source && m.kind == Kind::Result && acceptance == Some(m.request) {
                    assert_eq!(m.arg0, Status::Ok as u32);
                    acceptance = None;
                    dunit::println!("[TRANSFER] target accepted")?;
                } else if !source && m.kind == Kind::DragDrop {
                    drop = Some(m.transfer);
                } else if !source && m.kind == Kind::DragEnter {
                    let selected = transfer::negotiate(
                        event.offer.as_ref().unwrap(),
                        &offer(),
                        event.offer.as_ref().unwrap().preferred,
                    )
                    .unwrap();
                    acceptance = Some(
                        window
                            .transfers()?
                            .drag_accept(m.transfer, selected.format, selected.action)
                            .map_err(native)?,
                    );
                } else if !source && m.kind == Kind::OpenData {
                    let payload = event.payload.unwrap();
                    assert_eq!(payload.files.len(), 1);
                    opening = Some(m.transfer);
                    was_opener = true;
                    assert_eq!(
                        payload.files[0].read_all(64).map_err(native)?,
                        b"anchored original"
                    );
                    assert_eq!(
                        payload.files[0].handle.rights().map_err(native)?,
                        transfer::RIGHT_READ | transfer::RIGHT_TRANSFER
                    );
                    window
                        .transfers()?
                        .complete(m.transfer, Status::Ok)
                        .map_err(native)?;
                    dunit::println!(
                        "[TRANSFER] dock opened actual read-only capability in a new process"
                    )?;
                } else if !source && m.kind == Kind::Data {
                    if let Some(payload) = event.payload {
                        if let Some(bytes) = payload.bundle.get(&Format::Rgba) {
                            let rgba = transfer::Rgba::decode(bytes).unwrap();
                            assert_eq!((rgba.width, rgba.height), (2, 1));
                            assert_eq!(rgba.pixels, vec![255, 0, 0, 255, 0, 128, 255, 64]);
                            dunit::println!("[TRANSFER] RGBA bytes checked")?;
                        } else if let Some(bytes) = payload
                            .bundle
                            .get(&Format::Custom(String::from("application/x-m77-proof")))
                        {
                            assert_eq!(bytes, &[0, 255, 17, 0, 128]);
                            if drop == Some(m.transfer) {
                                drop = None;
                                assert_eq!(m.arg1, Action::Link as u32);
                                dunit::println!("[TRANSFER] custom Link drop checked")?;
                            } else {
                                dunit::println!("[TRANSFER] custom clipboard bytes checked")?;
                            }
                        } else if !payload.files.is_empty() {
                            assert_eq!(payload.files.len(), 1);
                            let file = &payload.files[0];
                            let mut bytes = [0; 17];
                            assert_eq!(file.read_at(0, &mut bytes).map_err(native)?, 17);
                            assert_eq!(&bytes, b"anchored original");
                            assert!(file.handle.rights().map_err(native)? & 2 == 0);
                            dunit::fs::create_dir("/tmp/m77-copy-target")?;
                            let directory = transfer::File::open("/tmp/m77-copy-target", true)
                                .map_err(native)?;
                            transfer::file_ops::copy_into(file, &directory.handle, "result")
                                .map_err(|e| native(e.operation))?;
                            assert_eq!(
                                dunit::fs::read("/tmp/m77-copy-target/result")?,
                                b"anchored original"
                            );
                            dunit::println!(
                                "[TRANSFER] file capabilities preserve inode and copy bytes"
                            )?;
                        } else {
                            assert_eq!(payload.text_value(), Some(text().as_str()));
                            if drop == Some(m.transfer) {
                                drop = None;
                                dunit::println!(
                                    "[TRANSFER] cross-process drop delivered complete UTF-8"
                                )?;
                            } else {
                                reads += 1;
                                dunit::io::stdout().write_all(
                                    alloc::format!(
                                        "[TRANSFER] text read {} bytes={}\n",
                                        reads,
                                        payload.text_value().unwrap().len()
                                    )
                                    .as_bytes(),
                                )?;
                            }
                        }
                        window
                            .transfers()?
                            .complete(m.transfer, Status::Ok)
                            .map_err(native)?;
                    }
                    if read == Some(m.request) {
                        read = None;
                    }
                }
            }
            _ => {}
        }
    }
    window.close()?;
    if source {
        dunit::println!("[TRANSFER] source closed")?;
    } else if was_opener {
        dunit::println!("[TRANSFER] opener closed")?;
    } else {
        dunit::println!("[TRANSFER] target closed")?;
    }
    Ok(())
}
