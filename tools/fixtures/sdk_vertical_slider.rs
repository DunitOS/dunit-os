extern crate alloc;
use alloc::{format, vec};
use dunit::ui::{
    Attrs, Component, ComponentWindow, Sizing, SliderOptions, SliderOrientation, View,
};
use dunit::{
    io::Write,
    time::Duration,
    window::{WindowEvent, IN_KEY, IN_MOVE, IN_UP},
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
            View::vertical_slider(
                key,
                label,
                value,
                SliderOptions::new(0, 100, 10),
                Msg::Value,
            )
            .layout(Attrs {
                width: Sizing::Px(48.),
                height: Sizing::Px(240.),
                ..Default::default()
            })
        };
        View::row(
            "root",
            vec![
                slider("level", "Level", self.value),
                slider("disabled", "Disabled", 20).disabled(true),
                View::column(
                    "details",
                    vec![
                        View::text("title", "Vertical slider"),
                        View::text(
                            "status",
                            format!("Level {} | updates {}", self.value, self.calls),
                        ),
                        View::button("apply", "Apply", Msg::Apply),
                        View::text("help", "Up/Down: step"),
                    ],
                ),
            ],
        )
    }
}
fn focused(ui: &ComponentWindow<Demo>, key: &str) -> bool {
    ui.view()
        .children()
        .iter()
        .chain(ui.view().children()[2].children())
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
        320,
        Demo {
            value: 40,
            calls: 0,
            applied: 0,
        },
    )?;
    assert_eq!(
        ui.view().children()[0].slider_range().unwrap().orientation,
        SliderOrientation::Vertical
    );
    dunit::println!("[VERTICAL-SLIDER] ready")?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_secs(30))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let marker = match stage {
            0 if focused(&ui, "level") => Some("focus OK"),
            1 if ui.component().value == 50 => Some("up increases OK"),
            2 if ui.component().value == 100 => Some("end OK"),
            3 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY && i.x==0x48) => {
                assert_eq!(ui.component().calls, 2);
                Some("endpoint no duplicate OK")
            }
            4 if ui.component().value == 0 => Some("home OK"),
            5 if ui.component().value == 100 => Some("page up OK"),
            6 if ui.component().value == 0 => Some("page down OK"),
            7 if ui.component().value == 80 => {
                assert!(pressed(&ui));
                Some("track click OK")
            }
            8 if ui.component().value == 100 => {
                assert!(pressed(&ui));
                Some("upward drag clamp OK")
            }
            9 if !pressed(&ui) => {
                assert_eq!(ui.component().calls, 7);
                Some("release OK")
            }
            10 if pressed(&ui) => {
                assert_eq!(ui.component().calls, 7);
                Some("thumb grab no jump OK")
            }
            11 if !pressed(&ui) => Some("escape cancels capture OK"),
            12 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_MOVE && i.y>=238) => {
                assert_eq!(ui.component().value, 100);
                assert_eq!(ui.component().calls, 7);
                Some("cancelled move OK")
            }
            13 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_UP) => {
                assert_eq!(ui.component().calls, 7);
                Some("cancelled release OK")
            }
            14 if ui.component().applied == 1 => {
                assert!(focused(&ui, "apply"));
                assert!(!focused(&ui, "disabled"));
                Some("tab skip and button OK")
            }
            15 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 480);
                assert!(focused(&ui, "apply"));
                Some("resize focus OK")
            }
            16 if ui.component().value == 90 => {
                assert!(focused(&ui, "level"));
                assert_eq!(ui.component().calls, 8);
                Some("resized down decreases OK")
            }
            17 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY && i.x==0x1c) => {
                break
            }
            _ => None,
        };
        if let Some(marker) = marker {
            ui_marker(marker)?;
            stage += 1;
        }
    }
    ui.close()?;
    dunit::println!("[VERTICAL-SLIDER] ok")?;
    Ok(())
}
fn ui_marker(marker: &str) -> dunit::Result<()> {
    // Serial tagging happens per write: keep the whole marker contiguous.
    dunit::io::stdout().write_all(format!("[VERTICAL-SLIDER] {marker}\n").as_bytes())
}
