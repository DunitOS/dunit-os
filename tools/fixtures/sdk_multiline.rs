extern crate alloc;
use alloc::{format, string::String, vec};
use dunit::ui::{Attrs, Component, ComponentWindow, InputOptions, Sizing, View};
use dunit::{
    time::{Duration, Instant},
    window::{WindowEvent, IN_SCROLL, IN_UP},
};
#[derive(Clone)]
enum Msg {
    Text(String),
    Fill,
}
struct Demo(String);
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Text(s) => self.0 = s,
            Msg::Fill => self.0 = (0..50).map(|i| format!("line {i:02} é界\n")).collect(),
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                View::text_area(
                    "editor",
                    "Document",
                    self.0.clone(),
                    Msg::Text,
                    InputOptions::default(),
                )
                .layout(Attrs {
                    height: Sizing::Px(140.),
                    ..Attrs::default()
                }),
                View::input(
                    "single",
                    "Single line",
                    "single",
                    Msg::Text,
                    InputOptions {
                        read_only: true,
                        ..InputOptions::default()
                    },
                )
                .layout(Attrs {
                    height: Sizing::Px(40.),
                    ..Attrs::default()
                }),
            ],
        )
    }
}
fn cursor(ui: &ComponentWindow<Demo>) -> usize {
    ui.view().children()[0].text_editor().unwrap().cursor()
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(480, 280, Demo("alpha\né界\nomega".into()))?;
    dunit::println!("[MULTILINE] ready")?;
    let mut stage = 0;
    let mut scroll = 0.;
    let mut scroll_time = None;
    let mut selected = String::new();
    loop {
        let event = ui.next_event(Duration::from_millis(50))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        let value = ui.component().0.as_str();
        match stage {
            0 if ui.view().children()[0].interaction().focused => {
                dunit::println!("[MULTILINE] focus OK")?;
                stage = 1;
            }
            1 if value == "alpha\né界\nomega\n" => {
                dunit::println!("[MULTILINE] enter split OK")?;
                stage = 2;
            }
            2 if value == "alpha\né界\nomega\ntail" => {
                dunit::println!("[MULTILINE] typed line OK")?;
                stage = 3;
            }
            3 if value == "alpha\né界\nZtail" => {
                dunit::println!("[MULTILINE] shift up replaced OK")?;
                stage = 4;
            }
            4 if value == "alpha\né界\nomega\ntail" => {
                assert_eq!(
                    ui.view().children()[0].text_editor().unwrap().selection(),
                    "alpha\né界\n".len().."alpha\né界\nomega\n".len()
                );
                dunit::println!("[MULTILINE] undo LF selection OK")?;
                stage = 5;
            }
            5 if value == "alpha\né界\nZtail" => {
                dunit::println!("[MULTILINE] redo OK")?;
                stage = 6;
            }
            6 if value == "alpha\né\nZtail" => {
                assert_eq!(cursor(&ui), "alpha\né".len());
                dunit::println!("[MULTILINE] line navigation UTF8 OK")?;
                stage = 7;
            }
            7 if value == "alpha\né界\nZtail" => {
                ui.update(Msg::Fill)?;
                dunit::println!("[MULTILINE] long document ready")?;
                stage = 8;
            }
            8 if cursor(&ui) == ui.component().0.len() => {
                assert!(ui.input_scroll_y("editor").unwrap() > 0.);
                scroll = ui.input_scroll_y("editor").unwrap();
                dunit::println!("[MULTILINE] caret vertical scroll OK")?;
                stage = 9;
            }
            9 if matches!(event, Some(WindowEvent::Input(i)) if i.kind == IN_SCROLL && i.x < 0)
                && ui.input_scroll_y("editor").unwrap() < scroll =>
            {
                scroll = ui.input_scroll_y("editor").unwrap();
                scroll_time = Some(Instant::now()?);
                dunit::println!("[MULTILINE] wheel scroll OK")?;
                stage = 10;
            }
            10 if scroll_time.unwrap().elapsed()? >= Duration::from_millis(700) => {
                assert_eq!(ui.input_scroll_y("editor"), Some(scroll));
                dunit::println!("[MULTILINE] wheel survives blink OK")?;
                stage = 11;
            }
            11 if cursor(&ui) == 0 => {
                assert_eq!(ui.input_scroll_y("editor"), Some(0.));
                dunit::println!("[MULTILINE] document home OK")?;
                stage = 12;
            }
            12 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_UP)
                && ui.view().children()[0]
                    .text_editor()
                    .unwrap()
                    .selection()
                    .len()
                    > 0 =>
            {
                let range = ui.view().children()[0].text_editor().unwrap().selection();
                assert!(value[range.clone()].contains('\n'));
                selected = value.into();
                dunit::println!("[MULTILINE] pointer across lines OK")?;
                stage = 13;
            }
            13 if value != selected && value.contains('X') => {
                dunit::println!("[MULTILINE] pointer replaced OK")?;
                stage = 14;
            }
            14 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.component().0.contains('X'));
                assert!(ui.size().0 > 480);
                dunit::println!("[MULTILINE] resize retained OK")?;
                stage = 15;
                break;
            }
            _ => {}
        }
    }
    assert_eq!(stage, 15);
    ui.close()?;
    dunit::println!("[MULTILINE] ok")?;
    Ok(())
}
