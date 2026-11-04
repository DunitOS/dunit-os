//! Bounded external-shell publication probe, not a replacement DWM shell.
extern crate alloc;
use alloc::{format, vec};
use dunit::{
    time::Duration,
    ui::*,
    window::{WindowEvent, IN_KEY},
};
struct Panel(u32, bool);
#[derive(Clone)]
enum Msg {
    Tick,
    Arm,
}
impl Component for Panel {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Tick => self.0 += 1,
            Msg::Arm => self.1 = true,
        }
    }
    fn view(&self) -> View<Msg> {
        View::row(
            "panel",
            vec![View::text("caption", format!("Shell probe {}", self.0))],
        )
        .on_key(Key::plain(' '), Msg::Tick)
        .on_key(Key::plain('g'), Msg::Arm)
    }
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(640, 28, Panel(0, false))?;
    ui.enable_metrics(true);
    ui.reset_metrics();
    dunit::println!(
        "{}",
        format!("[SHELLPROBE] ready pid={}", dunit::process::id())
    )?;
    let mut recording = false;
    while ui.component().0 < 24 {
        let event = ui.next_event(Duration::from_secs(30))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        if ui.component().1 && !recording {
            ui.reset_metrics();
            recording = true;
            dunit::println!("[SHELLPROBE] armed")?;
        }
        if let Some(WindowEvent::Input(input)) = event {
            if input.kind == IN_KEY && input.value == b' ' as u32 {
                assert!(ui.view().children()[0]
                    .content()
                    .unwrap()
                    .contains(&format!("{}", ui.component().0)));
            }
        }
    }
    assert_eq!(ui.component().0, 24);
    assert!(recording);
    assert_eq!(ui.metrics().window.commits, 24);
    assert_eq!(ui.metrics().window.completed_frames, 24);
    dunit::println!("{}", format!("[SHELLPROBE] metrics {:?}", ui.metrics()))?;
    dunit::println!("[SHELLPROBE] published 24; faulting without close")?;
    unsafe { core::arch::asm!("ud2", options(noreturn)) }
}
