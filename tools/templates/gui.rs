use dunit::time::Duration;
use dunit::window::{Frame, Window, WindowEvent};

fn paint(frame: &mut Frame<'_>) {
    // Use the current desktop palette. This canvas is ready for your content.
    let theme = dwm_settings::load().theme;
    frame.pixels_mut().fill(theme.menu);
}

#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut window = Window::open(480, 320)?;
    window.draw(paint)?;
    loop {
        match window.next_event(Duration::from_secs(60))? {
            Some(WindowEvent::CloseRequested) => break,
            Some(WindowEvent::Resized { .. }) => window.draw(paint)?,
            _ => {}
        }
    }
    window.close()
}

#[dunit::test]
fn native_window_frames() -> dunit::Result<()> {
    assert!(Window::open(0, 320).is_err());
    let mut window = Window::open(480, 320)?;
    assert!(Window::open(480, 320).is_err());
    window.draw(paint)?;
    let before = dunit::system::stats()?.pmm_used_bytes;
    // More than the server's live-buffer limit: completed frames must release
    // their old protocol object, capability and local mapping on each swap.
    for _ in 0..200 {
        window.draw(paint)?;
    }
    // 200 leaked 480x320 backings alone exceed 117 MiB. Allow desktop startup
    // allocations while bounding the steady drawing path's physical memory.
    assert!(dunit::system::stats()?.pmm_used_bytes <= before + 32 * 1024 * 1024);
    assert_eq!(window.size(), (480, 320));
    window.close()
}
