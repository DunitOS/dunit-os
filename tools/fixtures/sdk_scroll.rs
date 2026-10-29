extern crate alloc;
use alloc::{format, vec, vec::Vec};
use dunit::ui::{Attrs, Component, ComponentWindow, Key, Rect, Sizing, View};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_KEY},
};

struct Demo {
    clicks: u32,
    last: Option<usize>,
    small: bool,
    exit: bool,
}
#[derive(Clone)]
enum Msg {
    Click(usize),
    Noop,
    Shrink,
    Expand,
    Exit,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Click(id) => {
                self.clicks += 1;
                self.last = Some(id);
            }
            Msg::Noop => {}
            Msg::Shrink => self.small = true,
            Msg::Expand => self.small = false,
            Msg::Exit => self.exit = true,
        }
    }
    fn view(&self) -> View<Msg> {
        let rows: Vec<_> = (0..if self.small { 2 } else { 30 })
            .map(|id| {
                View::button(format!("row-{id}"), format!("Row {id}"), Msg::Click(id)).layout(
                    Attrs {
                        height: Sizing::Px(32.),
                        ..Attrs::default()
                    },
                )
            })
            .collect();
        View::column(
            "root",
            vec![
                View::scroll("scroll", "Thirty actions", rows).layout(Attrs {
                    height: Sizing::Px(200.),
                    ..Attrs::default()
                }),
                View::text(
                    "status",
                    format!("Clicks {} last {:?}", self.clicks, self.last),
                ),
            ],
        )
        .on_key(Key::plain('n'), Msg::Noop)
        .on_key(Key::plain('s'), Msg::Shrink)
        .on_key(Key::plain('e'), Msg::Expand)
        .on_key(Key::plain('x'), Msg::Exit)
    }
}
fn offset(ui: &ComponentWindow<Demo>) -> f32 {
    ui.scroll_offset("scroll").unwrap()
}
fn focused(ui: &ComponentWindow<Demo>, key: &str) -> bool {
    let scroll = &ui.view().children()[0];
    if key == "scroll" {
        scroll.interaction().focused
    } else {
        scroll
            .children()
            .iter()
            .any(|v| v.key() == key && v.interaction().focused)
    }
}
fn geometry(ui: &ComponentWindow<Demo>) -> dunit::Result<()> {
    let Rect { x, y, w, h } = ui.scroll_viewport("scroll").unwrap();
    dunit::println!(
        "{}",
        format!(
            "[SCROLL] geometry viewport={x},{y},{w},{h} thumb={:?} offset={}",
            ui.scroll_thumb("scroll"),
            offset(ui)
        )
    )
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        480,
        320,
        Demo {
            clicks: 0,
            last: None,
            small: false,
            exit: false,
        },
    )?;
    geometry(&ui)?;
    assert_eq!(offset(&ui), 0.);
    assert!(ui.scroll_thumb("scroll").is_some());
    dunit::println!("[SCROLL] ready")?;
    let mut stage = 0;
    let mut previous = 0.;
    let mut retained = 0.;
    loop {
        let event = ui.next_event(Duration::from_secs(30))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let current = offset(&ui);
        match stage {
            0 if focused(&ui, "scroll") => {
                dunit::println!("[SCROLL] tab focus OK")?;
                stage = 1;
            }
            1 if current > 0. => {
                assert!(focused(&ui, "scroll"));
                dunit::println!("[SCROLL] end OK")?;
                stage = 2;
            }
            2 if current == 0. => {
                dunit::println!("[SCROLL] home OK")?;
                stage = 3;
            }
            3 if current > 0. => {
                previous = current;
                dunit::println!("[SCROLL] page down OK")?;
                stage = 4;
            }
            4 if current < previous => {
                assert!(current > 0.);
                previous = current;
                dunit::println!("[SCROLL] up OK")?;
                stage = 5;
            }
            5 if current > previous => {
                previous = current;
                dunit::println!("[SCROLL] wheel down OK")?;
                stage = 6;
            }
            6 if current < previous => {
                previous = current;
                dunit::println!("[SCROLL] wheel up OK")?;
                geometry(&ui)?;
                stage = 7;
            }
            7 if current > previous => {
                previous = current;
                dunit::println!("[SCROLL] track page OK")?;
                geometry(&ui)?;
                stage = 8;
            }
            8 if current > previous => {
                previous = current;
                dunit::println!("[SCROLL] thumb drag OK")?;
                geometry(&ui)?;
                stage = 9;
            }
            9 if current > previous => {
                dunit::println!("[SCROLL] end before click OK")?;
                geometry(&ui)?;
                stage = 10;
            }
            10 if ui.component().clicks == 1 => {
                assert_eq!(ui.component().last, Some(29));
                assert!(focused(&ui, "row-29"));
                retained = current;
                dunit::println!("[SCROLL] scrolled button OK")?;
                stage = 11;
            }
            11 if matches!(&event, Some(WindowEvent::Input(input)) if input.kind == IN_KEY && input.value == 'n' as u32) =>
            {
                assert_eq!(current, retained);
                assert!(focused(&ui, "row-29"));
                assert_eq!(ui.component().clicks, 1);
                assert_eq!(ui.component().last, Some(29));
                dunit::println!("[SCROLL] noop retained OK")?;
                stage = 12;
            }
            12 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 480);
                assert_eq!(current, retained);
                assert!(focused(&ui, "row-29"));
                assert_eq!(ui.component().clicks, 1);
                dunit::println!("[SCROLL] resize retained OK")?;
                stage = 13;
            }
            13 if ui.component().small => {
                assert_eq!(current, 0.);
                assert!(ui.scroll_thumb("scroll").is_none());
                assert_eq!(ui.component().clicks, 1);
                dunit::println!("[SCROLL] shrink clamped OK")?;
                stage = 14;
            }
            14 if !ui.component().small => {
                assert_eq!(current, 0.);
                assert!(ui.scroll_thumb("scroll").is_some());
                assert!(!focused(&ui, "row-29"));
                assert_eq!(ui.component().clicks, 1);
                dunit::println!("[SCROLL] expand fresh OK")?;
                stage = 15;
            }
            _ => {}
        }
        if ui.component().exit {
            break;
        }
    }
    assert_eq!(stage, 15);
    ui.close()?;
    dunit::println!("[SCROLL] ok")?;
    Ok(())
}
