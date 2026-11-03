extern crate alloc;
use alloc::{string::String, vec};
use dunit::{
    time::Duration,
    transfer::{Kind, Status},
    ui::{Attrs, Component, ComponentWindow, InputOptions, Sizing, View},
    window::{WindowEvent, IN_KEY},
};
fn sample() -> String {
    String::from("Привет 👩‍💻 e\u{301}")
}
#[derive(Clone)]
enum Msg {
    Text(String),
    Secret(String),
    Readonly,
    Reset,
}
struct Demo {
    text: String,
    secret: String,
    calls: u32,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Text(text) => {
                self.text = text;
                self.calls += 1;
            }
            Msg::Secret(text) => {
                self.secret = text;
                self.calls += 1;
            }
            Msg::Readonly => panic!("read-only field changed"),
            Msg::Reset => {
                self.text.clear();
                self.calls = 0;
            }
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                View::input(
                    "text",
                    "Text",
                    self.text.clone(),
                    Msg::Text,
                    InputOptions::default(),
                ),
                View::input(
                    "readonly",
                    "Read only",
                    "Fixed",
                    |_| Msg::Readonly,
                    InputOptions {
                        read_only: true,
                        ..InputOptions::default()
                    },
                ),
                View::input(
                    "secret",
                    "Password",
                    self.secret.clone(),
                    Msg::Secret,
                    InputOptions {
                        password: true,
                        max_chars: 4,
                        ..InputOptions::default()
                    },
                ),
            ]
            .into_iter()
            .map(|view| {
                view.layout(Attrs {
                    height: Sizing::Px(44.0),
                    ..Attrs::default()
                })
            })
            .collect(),
        )
    }
}
fn native(error: dunit::transfer::Error) -> dunit::Error {
    dunit::Error(error.0)
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let source = dunit::env::var("DUNIT_APP_ID")?.unwrap() == "sdk_clipboard_fixture";
    let mut ui = ComponentWindow::open_themed(
        480,
        240,
        Demo {
            text: if source { sample() } else { String::new() },
            secret: String::from("hide"),
            calls: 0,
        },
    )?;
    if source {
        dunit::println!("[CLIP] source ready")?;
    } else {
        dunit::println!("[CLIP] target ready")?;
    }
    let mut stage = 0;
    let mut read = None;
    let mut verified = false;
    let mut expecting_drop = false;
    let mut dropped = false;
    loop {
        let event = ui.next_event(Duration::from_millis(20))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        if let Some(WindowEvent::Input(input)) = &event {
            if input.kind == IN_KEY && !source {
                if input.x == 0x3f {
                    ui.update(Msg::Reset)?;
                    expecting_drop = true;
                    dunit::println!("[CLIP] drop target reset")?;
                } else if input.x == 0x40 {
                    ui.update(Msg::Reset)?;
                    stage = 2;
                    dunit::println!("[CLIP] target reset")?;
                } else if input.x == 0x41 {
                    assert_eq!(ui.component().calls, 3);
                    assert_eq!(ui.component().secret, "hide");
                    dunit::println!("[CLIP] readonly paste rejected")?;
                } else if input.x == 0x42 {
                    read = Some(
                        ui.transfers()?
                            .read_clipboard(dunit::text_clipboard::preferences())
                            .map_err(native)?,
                    );
                }
            }
        }
        if let Some(WindowEvent::Transfer(event)) = event {
            if event.source
                && event.message.kind == Kind::Result
                && event.message.arg0 == Status::Ok as u32
            {
                if source && ui.component().calls == 0 {
                    dunit::println!("[CLIP] source copied")?;
                }
                if !source && ui.view().children()[1].interaction().focused {
                    dunit::println!("[CLIP] readonly copied")?;
                }
            }
            if event.message.kind == Kind::Data && read == Some(event.message.request) {
                assert_eq!(event.payload.unwrap().text_value(), Some("Fixed"));
                assert_eq!(ui.component().secret, "hide");
                assert_eq!(ui.component().calls, 3);
                ui.transfers()?
                    .complete(event.message.transfer, Status::Ok)
                    .map_err(native)?;
                read = None;
                verified = true;
                dunit::println!("[CLIP] password export blocked and over-limit paste atomic")?;
            }
        }
        let model = ui.component();
        if source && stage == 0 && model.text.is_empty() {
            assert_eq!(model.calls, 1);
            stage = 1;
            dunit::println!("[CLIP] cut after durable snapshot")?;
        }
        if !source {
            if expecting_drop && model.text == sample() {
                assert_eq!(model.calls, 1);
                expecting_drop = false;
                dropped = true;
                dunit::println!("[CLIP] text drop reached controlled model")?;
            }
            match stage {
                0 if model.text == sample() => {
                    assert_eq!(model.calls, 1);
                    stage = 1;
                    dunit::println!("[CLIP] whole UTF-8 paste reached controlled model")?;
                }
                2 if model.text == sample() => {
                    assert_eq!(model.calls, 1);
                    stage = 3;
                    dunit::println!("[CLIP] paste survives source exit")?;
                }
                3 if model.text.is_empty() => {
                    assert_eq!(model.calls, 2);
                    stage = 4;
                    dunit::println!("[CLIP] paste undone once")?;
                }
                4 if model.text == sample() => {
                    assert_eq!(model.calls, 3);
                    stage = 5;
                    dunit::println!("[CLIP] paste redone once")?;
                }
                _ => {}
            }
        }
    }
    if source {
        assert_eq!(stage, 1);
    } else {
        assert!(verified);
        assert_eq!(stage, 5);
        assert!(dropped);
    }
    ui.close()?;
    if source {
        dunit::println!("[CLIP] source closed")?;
    } else {
        dunit::println!("[CLIP] target closed")?;
    }
    Ok(())
}
