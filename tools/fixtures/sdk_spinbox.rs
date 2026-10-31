extern crate alloc;
use alloc::{format, vec};
use dunit::io::Write;
use dunit::ui::{Attrs, Component, ComponentWindow, Sizing, SpinboxOptions, View};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_KEY, IN_UP},
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
        let spin = |key, label, value, readonly| {
            let mut o = SpinboxOptions::new(-100, 100, 10);
            o.read_only = readonly;
            View::spinbox(key, label, value, o, Msg::Value).layout(Attrs {
                width: Sizing::Px(300.),
                height: Sizing::Px(48.),
                ..Attrs::default()
            })
        };
        View::column(
            "root",
            vec![
                spin("quantity", "Quantity", self.value, false),
                spin("disabled", "Disabled", 20, false).disabled(true),
                spin("readonly", "Read only", 40, true),
                View::button("apply", "Apply", Msg::Apply),
                View::text(
                    "status",
                    format!("Quantity {} | updates {}", self.value, self.calls),
                ),
                View::text("help", "Up/Down: step | Enter: commit | Esc: cancel"),
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
fn draft(ui: &ComponentWindow<Demo>) -> &str {
    ui.view().children()[0].text_editor().unwrap().text()
}
fn pressed(ui: &ComponentWindow<Demo>) -> bool {
    ui.view().children()[0].interaction().pressed
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        480,
        320,
        Demo {
            value: 20,
            calls: 0,
            applied: 0,
        },
    )?;
    dunit::println!("[SPINBOX] ready")?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_secs(30))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let marker = match stage {
            0 if focused(&ui, "quantity") => Some("focus OK"),
            1 if draft(&ui) == "-" => {
                assert_eq!(ui.component().value, 20);
                assert_eq!(ui.component().calls, 0);
                Some("incomplete draft OK")
            }
            2 if draft(&ui) == "-36" => {
                assert_eq!(ui.component().value, 20);
                Some("buffered edit OK")
            }
            3 if ui.component().value == -40 => {
                assert_eq!(draft(&ui), "-40");
                assert_eq!(ui.component().calls, 1);
                Some("commit snap OK")
            }
            4 if ui.component().value == -30 => Some("arrow step OK"),
            5 if ui.component().value == 70 => Some("page step OK"),
            6 if draft(&ui) == "9" => Some("cancel draft ready"),
            7 if draft(&ui) == "70" => {
                assert_eq!(ui.component().calls, 3);
                Some("escape OK")
            }
            8 if pressed(&ui) => {
                assert_eq!(ui.component().value, 70);
                Some("button press OK")
            }
            9 if ui.component().value == 80 => {
                assert!(!pressed(&ui));
                assert_eq!(ui.component().calls, 4);
                Some("button release OK")
            }
            10 if pressed(&ui) => Some("cancel press ready"),
            11 if !pressed(&ui) => Some("pointer cancel OK"),
            12 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_UP) => {
                assert_eq!(ui.component().value, 80);
                assert_eq!(ui.component().calls, 4);
                Some("cancel no update OK")
            }
            13 if focused(&ui, "readonly") => {
                assert!(!focused(&ui, "disabled"));
                Some("tab skip disabled OK")
            }
            14 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY && i.x==0x48) => {
                assert_eq!(ui.component().calls, 4);
                assert_eq!(ui.view().children()[2].text_editor().unwrap().text(), "40");
                Some("readonly OK")
            }
            15 if ui.component().applied == 1 => {
                assert!(focused(&ui, "apply"));
                Some("button activation OK")
            }
            16 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 480);
                assert!(focused(&ui, "apply"));
                Some("resize focus OK")
            }
            17 if ui.component().value == 70 => {
                assert_eq!(ui.component().calls, 5);
                assert!(focused(&ui, "quantity"));
                Some("resized keyboard OK")
            }
            18 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY && i.x==0x1c) => {
                break
            }
            _ => None,
        };
        if let Some(marker) = marker {
            // Kernel serial tagging happens per write; keep each marker atomic.
            dunit::io::stdout().write_all(format!("[SPINBOX] {marker}\n").as_bytes())?;
            stage += 1;
        }
    }
    ui.close()?;
    dunit::println!("[SPINBOX] ok")?;
    Ok(())
}
