//! Native observer: assertions read the actual receiving application's copied
//! field or filesystem result. Application identities and expectations are data.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use dunit::{
    io::Write,
    time::Duration,
    transfer::{self, Action, Format, Kind, Offer, Payload, Status},
    window::{Window, WindowEvent, IN_KEY},
};
fn native(error: transfer::Error) -> dunit::Error {
    dunit::Error(error.0)
}
const SENTINEL: &str = "m77-empty";
#[dunit::main]
fn main() -> dunit::Result<()> {
    let cases: Vec<String> = dunit::fs::read_to_string("/sdk-matrix.tsv")?
        .lines()
        .map(String::from)
        .collect();
    let mut window = Window::open(240, 120)?;
    window.draw(|frame| frame.pixels_mut().fill(0xff25334a))?;
    let mut index = 0;
    let mut checked: Vec<usize> = Vec::new();
    let mut choosing: Option<String> = None;
    let mut publication = None;
    let mut reading = None;
    let mut watching = false;
    let mut fs_ready = false;
    let mut move_check = false;
    dunit::println!("[MATRIX] observer ready")?;
    loop {
        match window.next_event(Duration::from_millis(20))? {
            Some(WindowEvent::CloseRequested) => break,
            Some(WindowEvent::Resized { .. }) => {
                window.draw(|frame| frame.pixels_mut().fill(0xff25334a))?
            }
            Some(WindowEvent::Input(input)) if input.kind == IN_KEY => {
                if let Some(number) = &mut choosing {
                    if matches!(input.value, 10 | 13) {
                        index = number.parse::<usize>().unwrap();
                        assert!(index < cases.len());
                        choosing = None;
                    } else if let Some(ch) =
                        char::from_u32(input.value).filter(|ch| ch.is_ascii_digit())
                    {
                        number.push(ch);
                    }
                    continue;
                }
                if input.value == b'm' as u32 {
                    move_check = true;
                    dunit::println!("[MATRIX] multi-file move armed")?;
                    continue;
                }
                if input.value == b'g' as u32 {
                    choosing = Some(String::new());
                } else if input.value == b'n' as u32 {
                    assert!(index < cases.len());
                    match dunit::fs::remove_file("/tmp/matrix-result") {
                        Ok(()) => {}
                        Err(error) if error.0 == dunit::errno::ENOENT => {}
                        Err(error) => return Err(error),
                    }
                    fs_ready = false;
                    watching = cases[index].split('\t').nth(2) != Some("clipboard");
                    publication = Some(
                        window
                            .transfers()?
                            .publish_clipboard(
                                Payload::text(SENTINEL).map_err(native)?,
                                1,
                                Action::Copy,
                            )
                            .map_err(native)?,
                    );
                } else if input.value == b'v' as u32 {
                    let fields: Vec<_> = cases[index].split('\t').collect();
                    if fields[2] == "clipboard" {
                        reading = Some(
                            window
                                .transfers()?
                                .read_clipboard(Offer {
                                    formats: alloc::vec![Format::Text],
                                    actions: 1,
                                    preferred: Action::Copy,
                                })
                                .map_err(native)?,
                        );
                    } else {
                        assert!(fs_ready, "filesystem result not observed");
                        let path = if fields[2] == "file" {
                            "/tmp/matrix-target/item"
                        } else {
                            "/tmp/matrix-result"
                        };
                        let bytes = dunit::fs::read(path)?;
                        let expected = if fields[2] == "file" {
                            String::from("matrix-file")
                        } else {
                            alloc::format!("{}\n", fields[3])
                        };
                        assert_eq!(bytes, expected.as_bytes(), "native receiver result");
                        if move_check {
                            assert_eq!(fields[2], "file");
                            assert_eq!(
                                dunit::fs::read("/tmp/matrix-target/item2")?,
                                b"second-file"
                            );
                            for path in ["/tmp/matrix-source/item", "/tmp/matrix-source/item2"] {
                                assert_eq!(
                                    dunit::fs::metadata(path).unwrap_err().0,
                                    dunit::errno::ENOENT
                                );
                            }
                            dunit::println!("[DND] multi-file native move ok")?;
                        }
                        dunit::io::stdout().write_all(
                            alloc::format!("[DND] {}->{} ok\n", fields[0], fields[1]).as_bytes(),
                        )?;
                        assert!(!checked.contains(&index));
                        checked.push(index);
                    }
                } else if input.value == b'q' as u32 {
                    assert!(!checked.is_empty());
                    break;
                }
            }
            Some(WindowEvent::Transfer(event)) => {
                let m = event.message;
                if m.kind == Kind::Result && publication == Some(m.request) {
                    assert_eq!(m.arg0, Status::Ok as u32);
                    publication = None;
                    dunit::io::stdout().write_all(
                        alloc::format!("[MATRIX] case {} ready\n", index + 1).as_bytes(),
                    )?;
                } else if m.kind == Kind::Data && reading == Some(m.request) {
                    let fields: Vec<_> = cases[index].split('\t').collect();
                    let payload = event.payload.unwrap();
                    assert_eq!(
                        payload.text_value(),
                        Some(fields[3]),
                        "native target field readback"
                    );
                    window
                        .transfers()?
                        .complete(m.transfer, Status::Ok)
                        .map_err(native)?;
                    reading = None;
                    dunit::io::stdout().write_all(
                        alloc::format!("[DND] {}->{} ok\n", fields[0], fields[1]).as_bytes(),
                    )?;
                    assert!(!checked.contains(&index));
                    checked.push(index);
                } else if m.kind == Kind::Result && reading == Some(m.request) && m.arg0 != 0 {
                    panic!("clipboard read failed {}", m.arg0);
                }
            }
            _ => {}
        }
        if watching && !fs_ready {
            let fields: Vec<_> = cases[index].split('\t').collect();
            let path = if fields[2] == "file" {
                "/tmp/matrix-target/item"
            } else {
                "/tmp/matrix-result"
            };
            let expected = if fields[2] == "file" {
                String::from("matrix-file")
            } else {
                alloc::format!("{}\n", fields[3])
            };
            if dunit::fs::read(path).is_ok_and(|bytes| bytes == expected.as_bytes()) {
                fs_ready = true;
                watching = false;
                dunit::io::stdout().write_all(
                    alloc::format!("[MATRIX] filesystem case {} ready\n", index + 1).as_bytes(),
                )?;
            }
        }
    }
    window.close()?;
    dunit::println!("[MATRIX] all selected native cells checked")?;
    Ok(())
}
