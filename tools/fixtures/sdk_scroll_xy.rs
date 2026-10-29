extern crate alloc;
use alloc::{format, string::String, vec, vec::Vec};
use dunit::ui::{Attrs, Component, ComponentWindow, InputOptions, Key, ScrollAxes, Sizing, View};
use dunit::{
    time::{Duration, Instant},
    window::{WindowEvent, IN_DOWN, IN_KEY, IN_MOVE, IN_UP},
};

fn document() -> String {
    let mut text = String::from("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijkl\n");
    for row in 1..50 {
        text.push_str(&format!("line {row:02}\n"));
    }
    text
}
struct Demo {
    text: String,
    small: bool,
    exit: bool,
}
#[derive(Clone)]
enum Msg {
    Text(String),
    Shrink,
    Exit,
    Click,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Text(text) => self.text = text,
            Msg::Shrink => {
                self.small = true;
                self.text = String::from("short");
            }
            Msg::Exit => self.exit = true,
            Msg::Click => {}
        }
    }
    fn view(&self) -> View<Msg> {
        let mut content = Vec::new();
        if self.small {
            content.push(View::text("small", "Short content"));
        } else {
            content.push(
                View::row(
                    "wide",
                    (0..10)
                        .map(|id| {
                            View::button(format!("wide-{id}"), format!("Item {id}"), Msg::Click)
                                .layout(Attrs {
                                    width: Sizing::Px(90.),
                                    height: Sizing::Px(60.),
                                    ..Attrs::default()
                                })
                        })
                        .collect(),
                )
                .layout(Attrs {
                    width: Sizing::Px(900.),
                    height: Sizing::Px(60.),
                    ..Attrs::default()
                }),
            );
            for id in 0..20 {
                content.push(
                    View::button(format!("row-{id}"), format!("Row {id}"), Msg::Click).layout(
                        Attrs {
                            height: Sizing::Px(24.),
                            ..Attrs::default()
                        },
                    ),
                );
            }
        }
        View::column(
            "root",
            vec![
                View::scroll("scroll", "Two axes", content)
                    .scroll_axes(ScrollAxes::Both)
                    .layout(Attrs {
                        height: Sizing::Px(140.),
                        ..Attrs::default()
                    }),
                View::text_area(
                    "editor",
                    "Two axis document",
                    self.text.clone(),
                    Msg::Text,
                    InputOptions::default(),
                )
                .layout(Attrs {
                    height: Sizing::Px(160.),
                    ..Attrs::default()
                }),
                View::text("footer", "Click here for S shrink / X exit"),
            ],
        )
        .on_key(Key::plain('s'), Msg::Shrink)
        .on_key(Key::plain('x'), Msg::Exit)
    }
}
fn xy(ui: &ComponentWindow<Demo>) -> (f32, f32) {
    (
        ui.scroll_offset_x("scroll").unwrap(),
        ui.scroll_offset("scroll").unwrap(),
    )
}
fn editor_xy(ui: &ComponentWindow<Demo>) -> (f32, f32) {
    (
        ui.input_scroll_x("editor").unwrap(),
        ui.input_scroll_y("editor").unwrap(),
    )
}
fn geometry(ui: &ComponentWindow<Demo>) -> dunit::Result<()> {
    for key in ["scroll", "editor"] {
        dunit::println!(
            "{}",
            format!(
                "[SCROLLXY] geometry {key} viewport={:?} thumb_x={:?} thumb_y={:?}",
                ui.scroll_viewport(key),
                ui.scroll_thumb_x(key),
                ui.scroll_thumb(key)
            )
        )?;
    }
    Ok(())
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let original = document();
    let mut ui = ComponentWindow::open_themed(
        480,
        400,
        Demo {
            text: original.clone(),
            small: false,
            exit: false,
        },
    )?;
    geometry(&ui)?;
    assert!(ui.scroll_thumb_x("scroll").is_some());
    assert!(ui.scroll_thumb("scroll").is_some());
    assert!(ui.scroll_thumb_x("editor").is_some());
    assert!(ui.scroll_thumb("editor").is_some());
    dunit::println!("[SCROLLXY] ready")?;
    let mut stage = 0;
    let mut last = (0., 0.);
    let mut selection = 0..0;
    let mut retained = (0., 0.);
    let mut timer = None;
    loop {
        let event = ui.next_event(Duration::from_millis(50))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let current = xy(&ui);
        let edit = editor_xy(&ui);
        // Bar press, motion and release must preserve the editor selection;
        // a misplaced gesture must fail instead of silently editing text.
        if (10..=12).contains(&stage) {
            assert_eq!(
                ui.view().children()[1].text_editor().unwrap().selection(),
                selection
            );
            assert_eq!(ui.component().text, original);
        }
        if matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_UP) {
            let release = match stage {
                6 => Some("horizontal released"),
                7 => Some("vertical released"),
                8 => Some("editor click released"),
                11 => Some("editor vertical released"),
                12 | 13 => Some("editor horizontal released"),
                _ => None,
            };
            if let Some(name) = release {
                dunit::println!("{}", format!("[SCROLLXY] {name} OK"))?;
            }
        }
        let moved = matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_MOVE);
        let down = matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_DOWN);
        match stage {
            0 if ui.view().children()[0].interaction().focused => {
                dunit::println!("[SCROLLXY] focus OK")?;
                stage = 1;
            }
            1 if current.0 > 0. => {
                dunit::println!("[SCROLLXY] right OK")?;
                stage = 2;
            }
            2 if current.1 > 0. => {
                dunit::println!("[SCROLLXY] end OK")?;
                stage = 3;
            }
            3 if current.1 == 0. => {
                last = current;
                dunit::println!("[SCROLLXY] home OK")?;
                geometry(&ui)?;
                stage = 4;
            }
            4 if down && current.0 > last.0 => {
                last = current;
                dunit::println!("[SCROLLXY] horizontal track OK")?;
                geometry(&ui)?;
                stage = 5;
            }
            5 if moved && current.0 < last.0 => {
                assert_eq!(current.1, last.1);
                last = current;
                dunit::println!("[SCROLLXY] horizontal drag OK")?;
                geometry(&ui)?;
                stage = 6;
            }
            6 if moved && current.1 > last.1 => {
                assert_eq!(current.0, last.0);
                dunit::println!("[SCROLLXY] vertical drag OK")?;
                stage = 7;
            }
            7 if ui.view().children()[1].interaction().focused => {
                dunit::println!("[SCROLLXY] editor focus OK")?;
                stage = 8;
            }
            8 if matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_KEY && i.value == 0 && i.x == 0x47) =>
            {
                assert_eq!(ui.view().children()[1].text_editor().unwrap().cursor(), 0);
                assert_eq!(edit, (0., 0.));
                dunit::println!("[SCROLLXY] editor home OK")?;
                stage = 9;
            }
            9 if !ui.view().children()[1]
                .text_editor()
                .unwrap()
                .selection()
                .is_empty() =>
            {
                selection = ui.view().children()[1].text_editor().unwrap().selection();
                assert_eq!(selection, 0..1);
                last = edit;
                dunit::println!("[SCROLLXY] selected OK")?;
                geometry(&ui)?;
                stage = 10;
            }
            10 if moved && edit.1 > last.1 => {
                assert_eq!(edit.0, last.0);
                last = edit;
                assert_eq!(
                    ui.view().children()[1].text_editor().unwrap().selection(),
                    selection
                );
                dunit::println!("[SCROLLXY] editor vertical drag OK")?;
                geometry(&ui)?;
                stage = 11;
            }
            11 if moved && edit.0 > last.0 => {
                assert_eq!(edit.1, last.1);
                assert_eq!(
                    ui.view().children()[1].text_editor().unwrap().selection(),
                    selection
                );
                retained = edit;
                timer = Some(Instant::now()?);
                dunit::println!("[SCROLLXY] editor horizontal drag OK")?;
                stage = 12;
            }
            12 if timer.as_ref().unwrap().elapsed()? >= Duration::from_millis(700) => {
                assert_eq!(edit, retained);
                assert_eq!(
                    ui.view().children()[1].text_editor().unwrap().selection(),
                    selection
                );
                assert_eq!(ui.component().text, original);
                dunit::println!("[SCROLLXY] blink retained OK")?;
                stage = 13;
            }
            13 if edit == (0., 0.)
                && ui.view().children()[1]
                    .text_editor()
                    .unwrap()
                    .selection()
                    .is_empty() =>
            {
                assert_eq!(ui.view().children()[1].text_editor().unwrap().cursor(), 0);
                dunit::println!("[SCROLLXY] caret revealed OK")?;
                stage = 14;
            }
            14 if ui.component().text == format!("z{original}") => {
                dunit::println!("[SCROLLXY] edited after bars OK")?;
                stage = 15;
            }
            15 if ui.component().text == original => {
                assert_eq!(ui.component().text.lines().count(), 50);
                dunit::println!("[SCROLLXY] undo LF OK")?;
                stage = 16;
            }
            16 if ui.component().small => {
                assert_eq!(current, (0., 0.));
                assert_eq!(edit, (0., 0.));
                for key in ["scroll", "editor"] {
                    assert!(ui.scroll_thumb(key).is_none());
                    assert!(ui.scroll_thumb_x(key).is_none());
                }
                assert_eq!(ui.component().text, "short");
                dunit::println!("[SCROLLXY] shrink clamped OK")?;
                stage = 17;
            }
            17 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 480);
                assert_eq!(current, (0., 0.));
                assert_eq!(edit, (0., 0.));
                assert_eq!(ui.component().text, "short");
                dunit::println!("[SCROLLXY] resize OK")?;
                stage = 18;
            }
            _ => {}
        }
        if ui.component().exit {
            break;
        }
    }
    assert_eq!(stage, 18);
    ui.close()?;
    dunit::println!("[SCROLLXY] ok")?;
    Ok(())
}
