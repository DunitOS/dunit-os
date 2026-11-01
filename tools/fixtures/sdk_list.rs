extern crate alloc;
use alloc::{format, vec, vec::Vec};
use dunit::ui::{
    Attrs, Component, ComponentWindow, Key, ListItem, ListSelection, SelectionMode, Sizing, View,
};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_DOWN, IN_SCROLL},
};
struct Demo {
    rows: Vec<ListItem>,
    selection: ListSelection,
    reject: bool,
    calls: usize,
    done: bool,
}
#[derive(Clone)]
enum Msg {
    Select(ListSelection),
    Reject,
    Shrink,
    Reverse,
    Done,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Select(s) => {
                self.calls += 1;
                if !self.reject {
                    self.selection = s;
                }
            }
            Msg::Reject => self.reject = true,
            Msg::Shrink => {
                self.rows.truncate(5);
                self.selection.retain(&self.rows);
            }
            Msg::Reverse => self.rows.reverse(),
            Msg::Done => self.done = true,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "root",
            vec![
                View::text("title", "Virtual list: 10,000 documents").layout(Attrs {
                    height: Sizing::Px(28.),
                    ..Default::default()
                }),
                View::list(
                    "list",
                    "Documents",
                    self.rows.clone(),
                    self.selection.clone(),
                    SelectionMode::Multiple,
                    Msg::Select,
                )
                .layout(Attrs {
                    width: Sizing::Grow(1.),
                    height: Sizing::Grow(1.),
                    ..Default::default()
                }),
                View::status_bar(
                    "status",
                    "Selection status",
                    format!(
                        "{} selected | current {} | {} rows",
                        self.selection.selected.len(),
                        self.selection.current.as_deref().unwrap_or("none"),
                        self.rows.len()
                    ),
                )
                .layout(Attrs {
                    height: Sizing::Px(28.),
                    ..Default::default()
                }),
            ],
        )
        .on_key(Key::plain('r'), Msg::Reject)
        .on_key(Key::plain('s'), Msg::Shrink)
        .on_key(Key::plain('o'), Msg::Reverse)
        .on_key(Key::plain('q'), Msg::Done)
    }
}
fn marker(s: &str) -> dunit::Result<()> {
    use dunit::io::Write;
    dunit::io::stdout().write_all(format!("[LIST] {s}\n").as_bytes())
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        640,
        400,
        Demo {
            rows: (0..10000)
                .map(|i| {
                    ListItem::new(format!("r{i}"), format!("Document {i:05} - workspace item"))
                        .disabled(i == 2)
                })
                .collect(),
            selection: ListSelection::default(),
            reject: false,
            calls: 0,
            done: false,
        },
    )?;
    assert!(ui.list_visible_rows("list").unwrap() < 20);
    marker("ready virtualized")?;
    let mut stage = 0;
    let mut wheel_from = 0.;
    let mut click_expected = None;
    let mut calls = 0;
    loop {
        let before = ui.list_geometry("list").unwrap();
        let event = ui.next_event(Duration::from_millis(100))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let m = ui.component();
        let current = m.selection.current.as_deref();
        let status = match stage {
            0 if ui.view().children()[1].interaction().focused => Some("tab focus OK"),
            1 if current == Some("r0") => Some("first selection OK"),
            2 if current == Some("r1") => {
                assert_eq!(m.selection.selected.len(), 2);
                Some("shift range OK")
            }
            3 if current == Some("r3") => {
                assert_eq!(m.selection.selected.len(), 3);
                Some("disabled skip OK")
            }
            4 if current == Some("r4") => {
                assert_eq!(m.selection.selected.len(), 3);
                Some("ctrl current OK")
            }
            5 if m.selection.selected.len() == 4 => Some("space toggle OK"),
            6 if m.selection.selected.len() == 9999 => Some("select all OK"),
            7 if current == Some("r9999") => {
                assert_eq!(m.selection.selected.len(), 1);
                assert!(ui.list_geometry("list").unwrap().visible().contains(&9999));
                assert!(ui.list_visible_rows("list").unwrap() < 20);
                Some("end reveal virtualized OK")
            }
            8 if current.is_some_and(|c| c != "r9999") => {
                assert_eq!(current, Some(format!("r{}", 9999 - before.page()).as_str()));
                assert!(ui
                    .list_geometry("list")
                    .unwrap()
                    .visible()
                    .contains(&(9999 - before.page())));
                assert!(ui.scroll_offset("list").unwrap() <= before.offset);
                wheel_from = ui.scroll_offset("list").unwrap();
                Some("page up OK")
            }
            9 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_SCROLL) => {
                assert!(ui.scroll_offset("list").unwrap() < wheel_from);
                Some("wheel OK")
            }
            10 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_DOWN&&i.value==0) => {
                if let Some(WindowEvent::Input(i)) = event {
                    click_expected = before
                        .row_at(i.x as f32, i.y as f32)
                        .map(|n| format!("r{n}"));
                }
                assert_eq!(m.selection.current, click_expected);
                assert!(click_expected.is_some());
                assert_eq!(m.selection.selected.len(), 1);
                Some("pointer row OK")
            }
            11 if m.rows[0].key == "r9999" => {
                assert_eq!(m.selection.current, click_expected);
                Some("reorder stable key OK")
            }
            12 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 640);
                assert_eq!(m.selection.current, click_expected);
                assert!(ui.list_visible_rows("list").unwrap() < 40);
                Some("resize OK")
            }
            13 if m.rows.len() == 5 => {
                assert_eq!(ui.scroll_offset("list"), Some(0.));
                assert_eq!(ui.list_visible_rows("list"), Some(5));
                assert!(m.selection.current.is_none());
                Some("shrink clamp OK")
            }
            14 if m.reject => {
                calls = m.calls;
                Some("reject ready")
            }
            15 if m.calls > calls => {
                assert_eq!(m.selection, ListSelection::default());
                Some("model rejection OK")
            }
            16 if m.done => break,
            _ => None,
        };
        if let Some(s) = status {
            marker(s)?;
            stage += 1;
        }
    }
    ui.close()?;
    marker("ok")?;
    Ok(())
}
