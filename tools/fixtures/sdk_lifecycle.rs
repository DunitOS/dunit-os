extern crate alloc;
use alloc::{format, vec};
use dunit::{
    time::Duration,
    ui::*,
    window::{WindowEvent, IN_FOCUS_IN, IN_FOCUS_OUT, IN_KEY, IN_KEY_UP},
};
struct Demo {
    actions: usize,
    done: bool,
}
#[derive(Clone)]
enum Msg {
    Activate,
    Done,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Activate => self.actions += 1,
            Msg::Done => self.done = true,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                View::button(
                    "action",
                    format!("Activate {}", self.actions),
                    Msg::Activate,
                )
                .layout(Attrs {
                    height: Sizing::Px(48.),
                    ..Default::default()
                }),
                View::menu(
                    "menu",
                    "File",
                    vec![MenuItem::action("item", "Apply", Msg::Activate)],
                )
                .menu_mnemonic('f')
                .layout(Attrs {
                    height: Sizing::Px(32.),
                    ..Default::default()
                }),
                View::text(
                    "help",
                    "Hold Space / Enter; Escape cancels; switch windows to cancel capture.",
                ),
            ],
        )
        .on_key(
            Key {
                character: 'q',
                modifiers: 2,
            },
            Msg::Done,
        )
    }
}
fn marker(s: &str) -> dunit::Result<()> {
    dunit::println!("{}", format!("[LIFECYCLE] {s}"))
}
fn pressed(ui: &ComponentWindow<Demo>) -> bool {
    ui.view().children()[0].interaction().pressed
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        480,
        280,
        Demo {
            actions: 0,
            done: false,
        },
    )?;
    assert!(ui.input_lifecycle());
    marker("ready")?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_secs(20))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let s = match stage {
            0 if ui.view().children()[0].interaction().focused => Some("button focused"),
            1 if pressed(&ui) => {
                assert_eq!(ui.component().actions, 0);
                Some("space held")
            }
            2 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY&&i.x==0x39) => {
                assert!(pressed(&ui));
                assert_eq!(ui.component().actions, 0);
                Some("repeat stays held")
            }
            3 if ui.component().actions == 1 => {
                assert!(!pressed(&ui));
                Some("release activates once")
            }
            4 if pressed(&ui) => {
                assert_eq!(ui.component().actions, 1);
                Some("enter held")
            }
            5 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY&&i.x==1) => {
                assert!(!pressed(&ui));
                assert_eq!(ui.component().actions, 1);
                Some("escape cancels")
            }
            6 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY_UP&&i.x==0x1c) => {
                assert!(!pressed(&ui));
                assert_eq!(ui.component().actions, 1);
                Some("cancelled release ignored")
            }
            7 if pressed(&ui) => {
                assert_eq!(ui.component().actions, 1);
                Some("focus cancellation armed")
            }
            8 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_FOCUS_OUT) => {
                assert!(!pressed(&ui));
                assert_eq!(ui.component().actions, 1);
                assert!(ui
                    .view()
                    .children()
                    .iter()
                    .all(|v| !v.interaction().focused));
                Some("focus cancels held")
            }
            9 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_FOCUS_IN) => {
                assert!(!pressed(&ui));
                assert_eq!(ui.component().actions, 1);
                assert!(ui.view().children()[0].interaction().focused);
                Some("focus returns without activation")
            }
            10 if ui.popup_state().is_some() => Some("menu open"),
            11 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_FOCUS_OUT) => {
                assert!(ui.popup_state().is_none());
                assert_eq!(ui.component().actions, 1);
                Some("focus dismisses menu")
            }
            12 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_FOCUS_IN) => {
                assert!(ui.popup_state().is_none());
                assert_eq!(ui.component().actions, 1);
                Some("menu stays closed")
            }
            13 if ui.component().done => break,
            _ => None,
        };
        if let Some(s) = s {
            marker(s)?;
            stage += 1;
        }
    }
    ui.close()?;
    marker("ok")?;
    Ok(())
}
