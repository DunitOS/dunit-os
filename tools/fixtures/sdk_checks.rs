extern crate alloc;
use alloc::{format, vec};
use dunit::ui::{Attrs, Component, ComponentWindow, Sizing, View};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_UP},
};
#[derive(Clone)]
enum Msg {
    Toggle(usize),
    Select(usize),
}
struct Demo {
    values: [bool; 3],
    choice: usize,
    calls: u32,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, msg: Msg) {
        self.calls += 1;
        match msg {
            Msg::Toggle(i) => self.values[i] = !self.values[i],
            Msg::Select(i) => self.choice = i,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                View::toggle_button(
                    "toggle",
                    format!("Toggle {}", self.values[0]),
                    self.values[0],
                    Msg::Toggle(0),
                ),
                View::checkbox("check", "Checkbox", self.values[1], Msg::Toggle(1)),
                View::switch("switch", "Switch", self.values[2], Msg::Toggle(2)),
                View::radio("a", "Alpha", "group", self.choice == 0, Msg::Select(0)),
                View::radio("disabled", "Disabled", "group", false, Msg::Select(99)).disabled(true),
                View::radio("b", "Beta", "group", self.choice == 1, Msg::Select(1)),
                View::radio("c", "Gamma", "group", self.choice == 2, Msg::Select(2)),
            ]
            .into_iter()
            .map(|v| {
                v.layout(Attrs {
                    height: Sizing::Px(44.),
                    ..Attrs::default()
                })
            })
            .collect(),
        )
    }
}
fn focused(ui: &ComponentWindow<Demo>, key: &str) -> bool {
    ui.view()
        .children()
        .iter()
        .find(|v| v.key() == key)
        .unwrap()
        .interaction()
        .focused
}
fn checked(ui: &ComponentWindow<Demo>, key: &str) -> bool {
    ui.view()
        .children()
        .iter()
        .find(|v| v.key() == key)
        .unwrap()
        .interaction()
        .checked
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        480,
        420,
        Demo {
            values: [false; 3],
            choice: 0,
            calls: 0,
        },
    )?;
    dunit::println!("[CHECKS] ready")?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_secs(30))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        let model = ui.component();
        match stage {
            0 if focused(&ui, "toggle") => {
                dunit::println!("[CHECKS] focus OK")?;
                stage = 1;
            }
            1 if model.values[0] => {
                assert!(checked(&ui, "toggle"));
                dunit::println!("[CHECKS] toggle keyboard OK")?;
                stage = 2;
            }
            2 if model.values[1] => {
                assert!(checked(&ui, "check"));
                dunit::println!("[CHECKS] checkbox keyboard OK")?;
                stage = 3;
            }
            3 if model.values[2] => {
                assert!(checked(&ui, "switch"));
                dunit::println!("[CHECKS] switch keyboard OK")?;
                stage = 4;
            }
            4 if model.choice == 1 => {
                assert!(focused(&ui, "b") && checked(&ui, "b") && !checked(&ui, "a"));
                assert_eq!(model.calls, 4);
                dunit::println!("[CHECKS] radio arrow skip disabled OK")?;
                stage = 5;
            }
            5 if model.choice == 2 => {
                assert!(focused(&ui, "c"));
                dunit::println!("[CHECKS] radio down OK")?;
                stage = 6;
            }
            6 if model.choice == 0 && model.calls == 6 => {
                assert!(focused(&ui, "a"));
                dunit::println!("[CHECKS] radio wrap OK")?;
                stage = 7;
            }
            7 if focused(&ui, "switch") => {
                assert_eq!(model.calls, 6);
                dunit::println!("[CHECKS] group tab stop OK")?;
                stage = 8;
            }
            8 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(focused(&ui, "switch"));
                assert_eq!(model.values, [true; 3]);
                assert!(ui.size().0 > 480);
                dunit::println!("[CHECKS] resize values retained OK")?;
                stage = 9;
            }
            9 if !model.values[0] => {
                assert!(!checked(&ui, "toggle"));
                dunit::println!("[CHECKS] toggle pointer OK")?;
                stage = 10;
            }
            10 if !model.values[1] => {
                assert!(!checked(&ui, "check"));
                dunit::println!("[CHECKS] checkbox pointer OK")?;
                stage = 11;
            }
            11 if !model.values[2] => {
                assert!(!checked(&ui, "switch"));
                dunit::println!("[CHECKS] switch pointer OK")?;
                stage = 12;
            }
            12 if model.choice == 1 => {
                assert!(checked(&ui, "b") && !checked(&ui, "a"));
                assert_eq!(model.calls, 10);
                dunit::println!("[CHECKS] radio pointer OK")?;
                stage = 13;
            }
            13 if matches!(event,Some(WindowEvent::Input(input)) if input.kind==IN_UP) => {
                assert_eq!(model.calls, 10);
                assert_eq!(model.values, [false; 3]);
                assert_eq!(model.choice, 1);
                assert!(!focused(&ui, "disabled"));
                dunit::println!("[CHECKS] disabled pointer ignored OK")?;
                stage = 14;
                break;
            }
            _ => {}
        }
    }
    assert_eq!(stage, 14);
    ui.close()?;
    dunit::println!("[CHECKS] ok")?;
    Ok(())
}
