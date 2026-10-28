extern crate alloc;
use alloc::{string::String, vec};
use dunit::ui::{Attrs, Component, ComponentWindow, InputOptions, Sizing, View};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_KEY, IN_UP},
};
#[derive(Clone)]
enum Msg {
    Query(String),
    Password(String),
    Readonly(String),
}
struct Demo {
    query: String,
    password: String,
    calls: u32,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, msg: Msg) {
        self.calls += 1;
        match msg {
            Msg::Query(v) => self.query = v,
            Msg::Password(v) => self.password = v,
            Msg::Readonly(value) => panic!("read-only dispatched: {}", value),
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                View::input(
                    "query",
                    "Query",
                    self.query.clone(),
                    Msg::Query,
                    InputOptions {
                        placeholder: "Search".into(),
                        ..InputOptions::default()
                    },
                ),
                View::input(
                    "password",
                    "Password",
                    self.password.clone(),
                    Msg::Password,
                    InputOptions {
                        password: true,
                        max_chars: 4,
                        placeholder: "Four characters".into(),
                        ..InputOptions::default()
                    },
                ),
                View::input(
                    "readonly",
                    "Read only",
                    "Fixed",
                    Msg::Readonly,
                    InputOptions {
                        read_only: true,
                        ..InputOptions::default()
                    },
                ),
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
fn field<'a>(ui: &'a ComponentWindow<Demo>, key: &str) -> &'a dunit::ui::TextEditor {
    ui.view()
        .children()
        .iter()
        .find(|v| v.key() == key)
        .unwrap()
        .text_editor()
        .unwrap()
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        480,
        240,
        Demo {
            query: "Привет".into(),
            password: String::new(),
            calls: 0,
        },
    )?;
    dunit::println!("[INPUT] ready")?;
    let mut stage = 0;
    let mut focused_pixels = vec![];
    let mut selected_before = String::new();
    let mut delete_expected = String::new();
    loop {
        let event = ui.next_event(Duration::from_millis(50))?;
        if matches!(event, Some(WindowEvent::CloseRequested)) {
            break;
        }
        let model = ui.component();
        match stage {
            0 if field(&ui, "query").cursor() == "Привет".len()
                && ui.view().children()[0].interaction().focused =>
            {
                focused_pixels = ui.pixels().to_vec();
                dunit::println!("[INPUT] focused OK")?;
                stage = 1;
            }
            1 if ui.pixels() != focused_pixels => {
                assert_eq!(model.calls, 0);
                assert!(ui.last_render().damage.is_some());
                dunit::println!("[INPUT] blink timer OK")?;
                stage = 2;
            }
            2 if model.query == "Приветabc" => {
                dunit::println!("[INPUT] unicode typing OK")?;
                stage = 3;
            }
            3 if model.query == "Приветac" => {
                assert_eq!(field(&ui, "query").cursor(), "Приветa".len());
                dunit::println!("[INPUT] raw backspace OK")?;
                stage = 4;
            }
            4 if model.query == "Zc" => {
                dunit::println!("[INPUT] shift selection replaced OK")?;
                stage = 5;
            }
            5 if model.query == "Приветac" => {
                assert_eq!(field(&ui, "query").selection(), 0.."Приветa".len());
                dunit::println!("[INPUT] undo OK")?;
                stage = 6;
            }
            6 if model.query == "Zc" => {
                dunit::println!("[INPUT] redo OK")?;
                stage = 7;
            }
            7 if model.query == "c" => {
                dunit::println!("[INPUT] home delete OK")?;
                stage = 8;
            }
            8 if model.query == "q" => {
                dunit::println!("[INPUT] select all OK")?;
                stage = 9;
            }
            9 if model.query.len() == 81 => {
                assert!(ui.input_scroll_x("query").unwrap() > 0.);
                dunit::println!("[INPUT] long line scrolled OK")?;
                stage = 10;
            }
            10 if model.password == "secr" => {
                assert_eq!(field(&ui, "password").text().chars().count(), 4);
                dunit::println!("[INPUT] password and limit OK")?;
                stage = 11;
            }
            11 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY&&i.value==b'x' as u32)
                && ui.view().children()[2].interaction().focused =>
            {
                assert_eq!(field(&ui, "readonly").text(), "Fixed");
                assert_eq!(model.password, "secr");
                dunit::println!("[INPUT] read only OK")?;
                stage = 12;
            }
            12 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert_eq!(model.query.len(), 81);
                assert_eq!(model.password, "secr");
                assert!(ui.size().0 > 480);
                dunit::println!("[INPUT] resize retained OK")?;
                stage = 13;
            }
            13 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_UP)
                && field(&ui, "query").selection().len() > 0 =>
            {
                let selection = field(&ui, "query").selection();
                assert!(
                    model.query.is_char_boundary(selection.start)
                        && model.query.is_char_boundary(selection.end)
                );
                selected_before = model.query.clone();
                dunit::println!("[INPUT] pointer selection OK")?;
                stage = 14;
            }
            14 if model.query != selected_before && model.query.contains('X') => {
                let cursor = field(&ui, "query").cursor();
                delete_expected = model.query[cursor..].into();
                dunit::println!("[INPUT] pointer replace OK")?;
                stage = 15;
            }
            15 if model.query == delete_expected => {
                assert_eq!(field(&ui, "query").cursor(), 0);
                dunit::println!("[INPUT] ctrl raw backspace OK")?;
                stage = 16;
                break;
            }
            _ => {}
        }
    }
    assert_eq!(stage, 16);
    ui.close()?;
    dunit::println!("[INPUT] ok")?;
    Ok(())
}
