extern crate alloc;
use alloc::vec;
use dunit::time::Duration;
use dunit::ui::{Component, ComponentWindow, View};
use dunit::window::{Window, WindowEvent};

struct Welcome;
impl Component for Welcome {
    type Message = ();
    fn update(&mut self, _: ()) {}
    fn view(&self) -> View<()> {
        View::column(
            "root",
            vec![
                View::text("title", "Hello, Dunit!"),
                View::text("body", "Your application starts here."),
            ],
        )
    }
}

#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut window = ComponentWindow::open_themed(480, 320, Welcome)?;
    loop {
        match window.next_event(Duration::from_secs(60))? {
            Some(WindowEvent::CloseRequested) => break,
            _ => {}
        }
    }
    window.close()
}

#[dunit::test]
fn native_window_frames() -> dunit::Result<()> {
    let (theme, _) = dunit::ui::load_theme("")?;
    let color = theme.background;
    let background =
        0xff00_0000 | ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32;
    assert!(Window::open(0, 320).is_err());
    let mut window = Window::open(480, 320)?;
    assert!(Window::open(480, 320).is_err());
    window.draw(|frame| frame.pixels_mut().fill(background))?;
    let before = dunit::system::stats()?.pmm_used_bytes;
    // Exceed the server's live-buffer limit using real commits, even though
    // an unchanged Component view would correctly skip these presentations.
    for _ in 0..200 {
        window.draw(|frame| frame.pixels_mut().fill(background))?;
    }
    assert!(dunit::system::stats()?.pmm_used_bytes <= before + 32 * 1024 * 1024);
    assert_eq!(window.size(), (480, 320));
    window.close()
}
