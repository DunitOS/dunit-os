extern crate alloc;
use alloc::{format, string::String, vec};
use core::ops::Range;
use dunit::ui::{Attrs, Component, ComponentWindow, InputOptions, Key, Sizing, View};
use dunit::{time::{Duration, Instant}, window::{WindowEvent, IN_DOWN, IN_KEY, IN_MOVE, IN_UP}};

fn document() -> String {
    let mut value = String::from("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijkl\n");
    for row in 1..=50 { value.push_str(&format!("line {row:02}\n")); }
    value
}
struct Demo { document: String, single: String, exit: bool }
#[derive(Clone)]
enum Msg { Document(String), Single(String), Exit }
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Document(value) => self.document = value,
            Msg::Single(value) => self.single = value,
            Msg::Exit => self.exit = true,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column("root", vec![
            View::text_area("editor", "Document", self.document.clone(), Msg::Document, InputOptions::default())
                .layout(Attrs { height: Sizing::Px(160.), ..Attrs::default() }),
            View::input("single", "Single line", self.single.clone(), Msg::Single, InputOptions::default())
                .layout(Attrs { height: Sizing::Px(40.), ..Attrs::default() }),
            View::text("footer", "Drag selection, then click here and X to exit"),
        ]).on_key(Key::plain('x'), Msg::Exit)
    }
}
fn state(ui: &ComponentWindow<Demo>, index: usize) -> (f32, f32, Range<usize>) {
    let key = if index == 0 { "editor" } else { "single" };
    (ui.input_scroll_x(key).unwrap(), ui.input_scroll_y(key).unwrap(),
        ui.view().children()[index].text_editor().unwrap().selection())
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let original = document();
    let single = String::from("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789");
    let mut ui = ComponentWindow::open_themed_with_style(480, 400,
        Demo { document: original.clone(), single: single.clone(), exit: false },
        "Input, Textarea { caret-blink: 0ms; }")?;
    dunit::println!("[AUTOSCROLL] ready")?;
    let mut stage = -1;
    let mut previous = state(&ui, 0);
    let mut changes = (0u32, 0u32, 0u32);
    let mut released = None;
    loop {
        let event = ui.next_event(Duration::from_millis(50))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let down = matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_DOWN);
        let moved = matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_MOVE);
        let up = matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_UP);
        let home = matches!(&event, Some(WindowEvent::Input(i)) if i.kind == IN_KEY && i.value == 0 && i.x == 0x47);
        let index = if stage >= 8 { 1 } else { 0 };
        let current = state(&ui, index);
        match stage {
            -1 if home && ui.view().children()[0].interaction().focused => {
                assert_eq!(current, (0., 0., 0..0));
                assert_eq!(ui.view().children()[0].text_editor().unwrap().cursor(), 0);
                dunit::println!("[AUTOSCROLL] textarea initialized zero OK")?; stage = 0;
            }
            8 if home && ui.view().children()[1].interaction().focused => {
                assert_eq!(current, (0., 0., 0..0));
                assert_eq!(ui.view().children()[1].text_editor().unwrap().cursor(), 0);
                dunit::println!("[AUTOSCROLL] input initialized zero OK")?; stage = 9;
            }
            0 if down && ui.view().children()[0].interaction().focused => {
                dunit::println!("[AUTOSCROLL] textarea down OK")?; stage = 1;
            }
            1 if moved && matches!(&event, Some(WindowEvent::Input(i)) if i.x > 468 && i.y > 172) => {
                assert!(!current.2.is_empty()); previous = current.clone(); changes = (0, 0, 0);
                dunit::println!("{}", format!("[AUTOSCROLL] textarea arm state={current:?} event={event:?}"))?;
                dunit::println!("[AUTOSCROLL] textarea edge armed OK")?; stage = 2;
            }
            2 if event.is_none() => {
                assert_eq!(ui.component().document, original);
                if current.0 > previous.0 { changes.0 += 1; }
                if current.1 > previous.1 { changes.1 += 1; }
                if current.2 != previous.2 { changes.2 += 1; }
                previous = current.clone();
                if changes.0 >= 2 && changes.1 >= 2 && changes.2 >= 2 {
                    assert!(current.0 > 0. && current.1 > 0.);
                    dunit::println!("[AUTOSCROLL] textarea stationary xy ticks OK")?; stage = 3;
                }
            }
            3 if up => {
                previous = current.clone(); released = Some(Instant::now()?);
                dunit::println!("[AUTOSCROLL] textarea released OK")?; stage = 4;
            }
            4 => {
                assert_eq!(current, previous);
                if released.unwrap().elapsed()? >= Duration::from_millis(250) {
                    dunit::println!("[AUTOSCROLL] textarea stopped OK")?; stage = 5;
                }
            }
            5 if home => {
                assert_eq!(current, (0., 0., 0..0));
                dunit::println!("[AUTOSCROLL] document home OK")?; stage = 6;
            }
            6 if ui.component().document == format!("z{original}") => {
                dunit::println!("[AUTOSCROLL] edit preserved OK")?; stage = 7;
            }
            7 if ui.component().document == original => {
                assert_eq!(ui.component().document.lines().count(), 51);
                assert_eq!(ui.view().children()[0].text_editor().unwrap().cursor(), 0);
                dunit::println!("[AUTOSCROLL] undo LF history OK")?; stage = 8;
            }
            9 if down && ui.view().children()[1].interaction().focused => {
                dunit::println!("[AUTOSCROLL] input down OK")?; stage = 10;
            }
            10 if moved && matches!(&event, Some(WindowEvent::Input(i)) if i.x > 468) => {
                assert!(!current.2.is_empty()); previous = current.clone(); changes = (0, 0, 0);
                dunit::println!("{}", format!("[AUTOSCROLL] input arm state={current:?} event={event:?}"))?;
                dunit::println!("[AUTOSCROLL] input edge armed OK")?; stage = 11;
            }
            11 if event.is_none() => {
                assert_eq!(ui.component().single, single);
                if current.0 > previous.0 { changes.0 += 1; }
                if current.2 != previous.2 { changes.2 += 1; }
                previous = current.clone();
                if changes.0 >= 2 && changes.2 >= 2 {
                    assert!(current.0 > 0.); assert_eq!(current.1, 0.);
                    dunit::println!("[AUTOSCROLL] input stationary ticks OK")?; stage = 12;
                }
            }
            12 if up => {
                previous = current.clone(); released = Some(Instant::now()?);
                dunit::println!("[AUTOSCROLL] input released OK")?; stage = 13;
            }
            13 => {
                assert_eq!(current, previous);
                if released.unwrap().elapsed()? >= Duration::from_millis(250) {
                    assert_eq!(ui.component().document, original); assert_eq!(ui.component().single, single);
                    dunit::println!("[AUTOSCROLL] input stopped OK")?; stage = 14;
                }
            }
            _ => {}
        }
        if ui.component().exit { break; }
    }
    assert_eq!(stage, 14);
    ui.close()?;
    dunit::println!("[AUTOSCROLL] ok")?;
    Ok(())
}
