extern crate alloc;
use dunit::{
    time::{self, Duration},
    ui::{Color, Component, ComponentWindow, Renderer, View},
    window::{WindowEvent, IN_KEY},
};
struct Document;
#[derive(Clone)]
enum Message {}
impl Component for Document {
    type Message = Message;
    fn update(&mut self, message: Message) {
        match message {}
    }
    fn view(&self) -> View<Message> {
        View::column(
            "document",
            alloc::vec![View::text("title", "Canvas document")],
        )
    }
}
fn pixels(size: (u32, u32)) -> alloc::vec::Vec<u32> {
    alloc::vec![0xff244466;size.0 as usize*size.1 as usize]
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let (theme, font) = dunit::ui::load_theme("Column#document { background: #00000000; }")?;
    let initial = (480, 320);
    let mut ui = ComponentWindow::open(
        initial.0,
        initial.1,
        Document,
        Renderer::new(font, theme.cascade, Color::rgba(0, 0, 0, 0)),
    )?;
    ui.set_canvas(pixels(initial), initial.0, initial.1)?;
    ui.enable_metrics(true);
    ui.reset_metrics();
    dunit::println!("[CANVAS] ready")?;
    let mut staged = false;
    let mut resized = 0;
    loop {
        match ui.next_event(Duration::from_secs(60))? {
            Some(WindowEvent::Input(input))
                if input.kind == IN_KEY && input.value == b'a' as u32 =>
            {
                let old = ui.size();
                let snapshot = pixels(old);
                dunit::println!("[CANVAS] old snapshot staged")?;
                // QMP maximizes while this snapshot is held. The configure is
                // queued in IPC, so the SDK discovers it during publication.
                time::sleep(Duration::from_secs(1))?;
                ui.set_canvas(snapshot, old.0, old.1)?;
                assert_ne!(ui.size(), old);
                assert_eq!(
                    ui.metrics().window.completed_frames,
                    0,
                    "unready canvas published"
                );
                staged = true;
                dunit::println!("[CANVAS] concurrent resize deferred without publication")?;
            }
            Some(WindowEvent::Resized { width, height }) => {
                assert!(staged);
                assert_eq!(
                    ui.metrics().window.completed_frames,
                    resized,
                    "automatic blank resize frame"
                );
                ui.set_canvas(pixels((width, height)), width, height)?;
                assert_eq!(
                    ui.last_uploaded_pixels(),
                    u64::from(width) * u64::from(height)
                );
                resized += 1;
                assert_eq!(ui.metrics().window.completed_frames, resized);
                if resized == 1 {
                    assert_ne!((width, height), initial);
                    dunit::println!("[CANVAS] maximize one complete canvas frame")?;
                } else {
                    assert_eq!((width, height), initial);
                    dunit::println!("[CANVAS] restore one complete canvas frame")?;
                }
            }
            Some(WindowEvent::CloseRequested) => break,
            _ => {}
        }
    }
    assert_eq!(resized, 2);
    ui.close()?;
    dunit::println!("[CANVAS] close OK")?;
    Ok(())
}
