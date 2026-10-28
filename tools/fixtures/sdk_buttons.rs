extern crate alloc;
use alloc::{format, vec};
use dunit::ui::{Attrs, Component, ComponentWindow, Sizing, View};
use dunit::{time::Duration, window::WindowEvent};

struct Demo(u32);
#[derive(Clone)]
struct Activate;
impl Component for Demo {
    type Message = Activate;
    fn update(&mut self, _: Activate) {
        self.0 += 1;
    }
    fn view(&self) -> View<Activate> {
        let mut buttons = vec![
            View::button("a", format!("Activate {}", self.0), Activate),
            View::button("disabled", "Disabled", Activate).disabled(true),
            View::button("b", "Other action", Activate),
        ];
        if self.0 > 0 {
            buttons.reverse();
        }
        View::column(
            "root",
            buttons
                .into_iter()
                .map(|v| {
                    v.layout(Attrs {
                        height: Sizing::Px(48.),
                        ..Attrs::default()
                    })
                })
                .collect(),
        )
    }
}
fn state(ui: &ComponentWindow<Demo>, key: &str) -> (bool, bool, bool) {
    let state = ui
        .view()
        .children()
        .iter()
        .find(|v| v.key() == key)
        .unwrap()
        .interaction();
    (state.hovered, state.focused, state.pressed)
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(480, 240, Demo(0))?;
    dunit::println!("[WIDGETS] ready")?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_secs(30))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        match stage {
            0 if state(&ui, "a").1 => {
                assert_eq!(ui.component().0, 0);
                dunit::println!("[WIDGETS] tab focus OK")?;
                stage = 1;
            }
            1 if ui.component().0 == 1 => {
                assert!(state(&ui, "a").1);
                assert_eq!(ui.view().children()[2].key(), "a");
                dunit::println!("[WIDGETS] space and reorder OK")?;
                stage = 2;
            }
            2 if state(&ui, "b").1 => {
                assert!(!state(&ui, "disabled").1);
                dunit::println!("[WIDGETS] skip disabled OK")?;
                stage = 3;
            }
            3 if ui.component().0 == 2 => {
                assert!(state(&ui, "b").1);
                dunit::println!("[WIDGETS] enter OK")?;
                stage = 4;
            }
            4 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(state(&ui, "b").1);
                assert!(ui.size().0 > 480);
                dunit::println!("[WIDGETS] resize retained focus OK")?;
                stage = 5;
            }
            5 if state(&ui, "b").2 => {
                assert_eq!(ui.component().0, 2);
                dunit::println!("[WIDGETS] pointer pressed OK")?;
                stage = 6;
            }
            6 if !state(&ui, "b").0 && !state(&ui, "b").2 => {
                assert_eq!(ui.component().0, 2);
                dunit::println!("[WIDGETS] leave cancelled OK")?;
                stage = 7;
            }
            7 if state(&ui, "b").0 && !state(&ui, "b").2 => {
                assert_eq!(ui.component().0, 2);
                dunit::println!("[WIDGETS] returned without press OK")?;
                stage = 8;
            }
            8 if ui.component().0 == 3 => {
                assert!(!state(&ui, "b").2);
                dunit::println!("[WIDGETS] pointer activated OK")?;
                stage = 9;
                break;
            }
            _ => {}
        }
    }
    assert_eq!(stage, 9);
    ui.close()?;
    dunit::println!("[WIDGETS] buttons ok")?;
    Ok(())
}
