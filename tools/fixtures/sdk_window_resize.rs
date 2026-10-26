use dunit::time::{self, Duration};
use dunit::window::{Frame, Window, WindowEvent, IN_KEY};
fn paint(frame: &mut Frame<'_>) {
    frame.pixels_mut().fill(dwm_settings::load().theme.menu);
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let initial = (480, 320);
    let mut window = Window::open(initial.0, initial.1)?;
    window.draw(paint)?;
    dunit::println!("[SDKRESIZE] ready")?;
    let mut recovered = false;
    let mut placement = None;
    loop {
        match window.next_event(Duration::from_secs(60))? {
            Some(WindowEvent::Input(input))
                if input.kind == IN_KEY && input.value == b'a' as u32 =>
            {
                let mut calls = 0;
                window.draw(|frame| {
                    calls += 1;
                    if calls == 1 {
                        // QMP maximizes this real window while painting is in
                        // progress. The old ACK/COMMIT must become stale.
                        dunit::println!("[SDKRESIZE] painting").unwrap();
                        time::sleep(Duration::from_secs(1)).unwrap();
                    }
                    paint(frame);
                })?;
                assert!(calls >= 2, "stale frame was not repainted");
                assert_ne!(window.size(), initial);
                let session = dwm_settings::load_session();
                let saved = session
                    .iter()
                    .find(|entry| entry.id.as_str() == "sdk_resize_fixture")
                    .expect("maximized window has no saved floating placement");
                assert!(saved.maximized);
                placement = Some((saved.x, saved.y));
                recovered = true;
                dunit::println!(
                    "[SDKRESIZE] recovered size={}x{}",
                    window.size().0,
                    window.size().1
                )?;
                dunit::println!("[SDKRESIZE] recovered repaint OK")?;
            }
            Some(WindowEvent::Resized { .. }) => {
                window.draw(paint)?;
                dunit::println!(
                    "[SDKRESIZE] event size={}x{}",
                    window.size().0,
                    window.size().1
                )?;
                if recovered && window.size() == initial {
                    let session = dwm_settings::load_session();
                    let saved = session
                        .iter()
                        .find(|entry| entry.id.as_str() == "sdk_resize_fixture")
                        .expect("restored window has no saved placement");
                    assert!(!saved.maximized);
                    assert_eq!(Some((saved.x, saved.y)), placement);
                    dunit::println!("[SDKRESIZE] restored placement OK")?;
                }
            }
            Some(WindowEvent::CloseRequested) => break,
            _ => {}
        }
    }
    assert!(recovered);
    window.close()?;
    dunit::println!("[SDKRESIZE] close OK")?;
    Ok(())
}
