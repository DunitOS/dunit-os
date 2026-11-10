#![no_std]
#![no_main]
//! Unprivileged interactive SDK client; each process owns an independent model.
extern crate alloc;
use alloc::vec;
use dunit::ui::{Component, ComponentWindow, View};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_KEY},
};
struct Client {
    clicks: u32,
    first: bool,
}
#[derive(Clone)]
enum Message {
    Click,
    Identify(u32),
}
impl Component for Client {
    type Message = Message;
    fn update(&mut self, message: Message) {
        match message {
            Message::Click => self.clicks = self.clicks.wrapping_add(1),
            Message::Identify(id) => self.first = id == 1,
        }
    }
    fn view(&self) -> View<Message> {
        View::column(
            "win",
            vec![
                View::text("title", if self.first { "Dunit" } else { "Window 2" }),
                View::text("sub", alloc::format!("clicks: {}", self.clicks)),
                View::button("ok", "OK", Message::Click),
            ],
        )
    }
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed_with_style(240, 120, Client { clicks: 0,first:true }, "Column#win { padding: 12; } Text#title { font-size: 18; } Text#sub { font-size: 13; color: $accent; } Button#ok { padding: 6; font-size: 15; }")?;
    ui.update(Message::Identify(ui.launch_id()))?;
    dunit::println!("gui_client: surface presented OK")?;
    loop {
        match ui.next_event(Duration::from_millis(40))? {
            Some(WindowEvent::CloseRequested) => break,
            Some(WindowEvent::Resized { .. }) => {
                dunit::println!("gui_client: reconfigured OK")?;
            }
            Some(WindowEvent::Input(input)) if input.kind == IN_KEY => {
                use dunit::io::Write;
                dunit::io::stdout().write_all(&[
                    b'k',
                    b'e',
                    b'y',
                    b'=',
                    input.value as u8,
                    b'\n',
                ])?;
            }
            _ => {}
        }
    }
    ui.close()
}
