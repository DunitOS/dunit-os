//! A separate SDK client must keep publishing after the panel process faults.
extern crate alloc;
use alloc::{format, vec};
use dunit::{time::Duration, ui::*};
struct Survivor(u32, bool);
#[derive(Clone)]
enum Msg {
    Tick,
    Arm,
}
impl Component for Survivor {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Tick => self.0 += 1,
            Msg::Arm => self.1 = true,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "survivor",
            vec![View::text("value", format!("Survived {}", self.0))],
        )
        .on_key(Key::plain('g'), Msg::Arm)
    }
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(320, 100, Survivor(0, false))?;
    dunit::println!("[SHELLPROBE] survivor ready")?;
    // The harness sends g only after observing the compositor reap the peer.
    for _ in 0..600 {
        ui.next_event(Duration::from_millis(100))?;
        if ui.component().1 {
            break;
        }
    }
    assert!(ui.component().1);
    assert_eq!(ui.component().0, 0);
    dunit::println!("[SHELLPROBE] survivor armed after reap")?;
    for _ in 0..24 {
        ui.update(Msg::Tick)?;
    }
    assert_eq!(ui.component().0, 24);
    assert_eq!(ui.view().children()[0].content(), Some("Survived 24"));
    ui.close()?;
    dunit::println!("[SHELLPROBE] survivor 24 commits after peer loss OK")?;
    Ok(())
}
