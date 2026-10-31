extern crate alloc;
use alloc::{format, vec};
use dunit::ui::{Attrs, Component, ComponentWindow, Sizing, SliderOptions, View};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_KEY},
};
struct Demo {
    value: i64,
    calls: u32,
    applied: u32,
}
#[derive(Clone)]
enum Msg {
    Value(i64),
    Apply,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Value(v) => {
                self.value = v;
                self.calls += 1;
            }
            Msg::Apply => self.applied += 1,
        }
    }
    fn view(&self) -> View<Msg> {
        let slider = |key, label, value| {
            View::slider(
                key,
                label,
                value,
                SliderOptions::new(0, 100, 10),
                Msg::Value,
            )
            .layout(Attrs {
                width: Sizing::Px(300.),
                height: Sizing::Px(48.),
                ..Attrs::default()
            })
        };
        View::column(
            "root",
            vec![
                slider("volume", "Volume", self.value),
                slider("locked", "Disabled range", 20).disabled(true),
                View::button("apply", "Apply", Msg::Apply),
                View::text(
                    "status",
                    format!("Volume {} | updates {}", self.value, self.calls),
                ),
            ],
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
fn pressed(ui: &ComponentWindow<Demo>) -> bool {
    ui.view().children()[0].interaction().pressed
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        480,
        240,
        Demo {
            value: 40,
            calls: 0,
            applied: 0,
        },
    )?;
    dunit::println!("[SLIDER] ready")?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_secs(30))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        match stage {
            0 if focused(&ui, "volume") => {
                dunit::println!("[SLIDER] focus OK")?;
                stage = 1;
            }
            1 if ui.component().value == 50 => {
                dunit::println!("[SLIDER] arrow step OK")?;
                stage = 2;
            }
            2 if ui.component().value == 100 => {
                dunit::println!("[SLIDER] end OK")?;
                stage = 3;
            }
            3 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY && i.x==0x4d) => {
                assert_eq!(ui.component().value, 100);
                assert_eq!(ui.component().calls, 2);
                dunit::println!("[SLIDER] endpoint no duplicate OK")?;
                stage = 4;
            }
            4 if ui.component().value == 0 => {
                dunit::println!("[SLIDER] home OK")?;
                stage = 5;
            }
            5 if ui.component().value == 100 => {
                dunit::println!("[SLIDER] page up OK")?;
                stage = 6;
            }
            6 if ui.component().value == 0 => {
                dunit::println!("[SLIDER] page down OK")?;
                stage = 7;
            }
            7 if ui.component().value == 80 => {
                assert!(pressed(&ui));
                dunit::println!("[SLIDER] track click OK")?;
                stage = 8;
            }
            8 if ui.component().value == 100 => {
                assert!(pressed(&ui));
                dunit::println!("[SLIDER] captured drag clamp OK")?;
                stage = 9;
            }
            9 if !pressed(&ui) => {
                assert_eq!(ui.component().calls, 7);
                dunit::println!("[SLIDER] release OK")?;
                stage = 10;
            }
            10 if ui.component().applied == 1 => {
                assert!(focused(&ui, "apply"));
                assert!(!focused(&ui, "locked"));
                dunit::println!("[SLIDER] tab and button OK")?;
                stage = 11;
            }
            11 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 480);
                assert!(focused(&ui, "apply"));
                dunit::println!("[SLIDER] resize focus OK")?;
                stage = 12;
            }
            12 if ui.component().value == 0 => {
                assert!(focused(&ui, "volume"));
                dunit::println!("[SLIDER] resized keyboard OK")?;
                break;
            }
            _ => {}
        }
    }
    ui.close()?;
    dunit::println!("[SLIDER] ok")?;
    Ok(())
}
