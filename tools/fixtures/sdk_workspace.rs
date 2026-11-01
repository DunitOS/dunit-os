extern crate alloc;
use alloc::{format, string::String, vec};
use dunit::ui::{
    Attrs, Component, ComponentWindow, DialogAction as D, InputOptions, Key, Sizing, SplitAxis,
    SplitterOptions, Tab, View,
};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_KEY, IN_UP},
};
struct Demo {
    tab: &'static str,
    ratio: i64,
    text: String,
    name: String,
    modal: u8,
    accepted: u32,
    cancelled: u32,
    acked: u32,
    messages: u32,
    done: bool,
}
#[derive(Clone)]
enum Msg {
    Tab(&'static str),
    Ratio(i64),
    Text(String),
    Name(String),
    Open,
    Message,
    Accept,
    Cancel,
    Ack,
    Done,
}
fn fixed(v: View<Msg>, width: f32) -> View<Msg> {
    v.layout(Attrs {
        width: Sizing::Px(width),
        height: Sizing::Px(32.),
        ..Default::default()
    })
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Tab(t) => self.tab = t,
            Msg::Ratio(v) => self.ratio = v,
            Msg::Text(t) => self.text = t,
            Msg::Name(n) => self.name = n,
            Msg::Open => self.modal = 1,
            Msg::Message => {
                self.modal = 2;
                self.messages += 1;
            }
            Msg::Accept => {
                self.accepted += 1;
                self.modal = 0;
            }
            Msg::Cancel => {
                self.cancelled += 1;
                self.modal = 0;
            }
            Msg::Ack => {
                self.acked += 1;
                self.modal = 0;
            }
            Msg::Done => self.done = true,
        }
    }
    fn view(&self) -> View<Msg> {
        let tools = View::toolbar(
            "tools",
            "Workspace commands",
            vec![
                fixed(View::button("open", "Open dialog", Msg::Open), 120.),
                fixed(
                    View::button("disabled", "Disabled", Msg::Done).disabled(true),
                    80.,
                ),
                View::toolbar_separator("tool-sep"),
                fixed(View::button("message", "Message", Msg::Message), 120.),
            ],
        );
        let pages = View::tabs(
            "tabs",
            "Documents",
            self.tab,
            vec![
                Tab::new(
                    "a",
                    "Editor",
                    Msg::Tab("a"),
                    View::column(
                        "editor",
                        vec![
                            View::text("editor-label", "Editable document"),
                            View::input(
                                "edit",
                                "Document",
                                self.text.clone(),
                                Msg::Text,
                                InputOptions::default(),
                            )
                            .layout(Attrs {
                                height: Sizing::Px(32.),
                                ..Default::default()
                            }),
                        ],
                    ),
                ),
                Tab::new(
                    "locked",
                    "Locked",
                    Msg::Tab("locked"),
                    View::text("locked-body", "Locked"),
                )
                .disabled(true),
                Tab::new(
                    "b",
                    "Preview",
                    Msg::Tab("b"),
                    View::text("preview", "Preview page"),
                ),
            ],
        );
        let split = View::splitter(
            "split",
            "Pane sizes",
            self.ratio,
            SplitterOptions::new(SplitAxis::Horizontal),
            pages,
            View::column(
                "side",
                vec![
                    View::text("side-title", "Side pane"),
                    View::text("side-text", "Drag divider / arrows"),
                ],
            ),
            Msg::Ratio,
        )
        .layout(Attrs {
            height: Sizing::Px(200.),
            ..Default::default()
        });
        let base = View::column(
            "base",
            vec![
                tools,
                split,
                View::status_bar(
                    "status",
                    "Workspace status",
                    format!(
                        "Page {} | split {} | text {}",
                        self.tab, self.ratio, self.text
                    ),
                ),
            ],
        );
        let dialog = match self.modal {
            1 => Some(View::dialog(
                "settings",
                "Settings",
                View::input(
                    "name",
                    "Name",
                    self.name.clone(),
                    Msg::Name,
                    InputOptions::default(),
                ),
                vec![
                    D::new("save", "Save", Msg::Accept),
                    D::new("cancel", "Cancel", Msg::Cancel),
                ],
                Some("save"),
                Some(Msg::Cancel),
            )),
            2 => Some(View::message_box(
                "notice",
                "Message",
                "The document is ready.\nInput stays inside this dialog.",
                vec![D::new("ack", "OK", Msg::Ack)],
                Some("ack"),
                Some(Msg::Ack),
            )),
            _ => None,
        };
        View::modal("host", base, dialog).on_key(Key::plain('q'), Msg::Done)
    }
}
fn find<'a>(v: &'a View<Msg>, key: &str) -> Option<&'a View<Msg>> {
    if v.key() == key {
        Some(v)
    } else {
        v.children().iter().find_map(|c| find(c, key))
    }
}
fn focused(ui: &ComponentWindow<Demo>, key: &str) -> bool {
    find(ui.view(), key).is_some_and(|v| v.interaction().focused)
}
fn pressed(ui: &ComponentWindow<Demo>) -> bool {
    find(ui.view(), "split/handle")
        .unwrap()
        .interaction()
        .pressed
}
fn marker(s: &str) -> dunit::Result<()> {
    use dunit::io::Write;
    dunit::io::stdout().write_all(format!("[WORKSPACE] {s}\n").as_bytes())
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        640,
        400,
        Demo {
            tab: "a",
            ratio: 500,
            text: "Hello".into(),
            name: "Name".into(),
            modal: 0,
            accepted: 0,
            cancelled: 0,
            acked: 0,
            messages: 0,
            done: false,
        },
    )?;
    marker("ready")?;
    let mut stage = 0;
    loop {
        let event = ui.next_event(Duration::from_millis(100))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let status = match stage {
            0 if focused(&ui, "open") => Some("toolbar focus OK"),
            1 if focused(&ui, "message") => {
                assert_eq!(ui.component().modal, 0);
                Some("toolbar disabled skip OK")
            }
            2 if focused(&ui, "open") => Some("toolbar back OK"),
            3 if focused(&ui, "tabs/tab/a") => Some("tabs one stop OK"),
            4 if ui.component().tab == "b" => {
                assert!(find(ui.view(), "edit").is_none());
                Some("tab arrow skip OK")
            }
            5 if ui.component().tab == "a" => Some("tab home OK"),
            6 if focused(&ui, "edit") => Some("page input focus OK"),
            7 if ui.component().text == "x" => Some("page edit OK"),
            8 if ui.component().tab == "b" => Some("ctrl tab OK"),
            9 if ui.component().tab == "a" => {
                assert_eq!(ui.component().text, "x");
                assert_eq!(find(ui.view(), "edit").unwrap().content(), Some("x"));
                Some("page model restore OK")
            }
            10 if focused(&ui, "split/handle") => Some("splitter focus OK"),
            11 if ui.component().ratio == 0 => Some("splitter home OK"),
            12 if ui.component().ratio == 1000 => Some("splitter end OK"),
            13 if ui.component().ratio == 900 => Some("splitter page OK"),
            14 if ui.component().ratio == 0 => Some("splitter reset start"),
            15 if ui.component().ratio == 500 => Some("splitter center ready"),
            16 if pressed(&ui) => {
                assert_eq!(ui.component().ratio, 500);
                Some("grab no jump OK")
            }
            17 if ui.component().ratio == 660 => {
                assert!(pressed(&ui));
                Some("splitter drag OK")
            }
            18 if ui.component().ratio == 1000 => {
                assert!(pressed(&ui));
                Some("splitter clamp OK")
            }
            19 if !pressed(&ui) && matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_UP) => {
                Some("splitter release OK")
            }
            20 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 640);
                assert!(focused(&ui, "split/handle"));
                Some("resize workspace OK")
            }
            21 if matches!(event, Some(WindowEvent::Resized { .. })) && ui.size().0 == 640 => {
                Some("restore workspace OK")
            }
            22 if ui.component().ratio == 500 => Some("ratio restored OK"),
            23 if ui.active_dialog() == Some("settings") => {
                assert!(focused(&ui, "save"));
                Some("dialog default focus OK")
            }
            24 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_UP) => {
                assert_eq!(ui.component().messages, 0);
                assert_eq!(ui.component().modal, 1);
                assert!(focused(&ui, "save"));
                Some("modal outside blocked OK")
            }
            25 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_KEY&&i.value==b'q' as u32) =>
            {
                assert!(!ui.component().done);
                Some("modal shortcut blocked OK")
            }
            26 if focused(&ui, "name") => Some("modal tab trap OK"),
            27 if ui.component().name == "n" => Some("dialog input OK"),
            28 if ui.component().accepted == 1 => {
                assert_eq!(ui.active_dialog(), None);
                assert!(focused(&ui, "open"));
                Some("default restores opener OK")
            }
            29 if ui.active_dialog() == Some("settings") => Some("cancel dialog ready"),
            30 if ui.component().cancelled == 1 => {
                assert!(focused(&ui, "open"));
                Some("escape restores opener OK")
            }
            31 if ui.active_dialog() == Some("notice") => {
                assert!(focused(&ui, "ack"));
                Some("message box OK")
            }
            32 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 640);
                assert_eq!(ui.active_dialog(), Some("notice"));
                assert!(focused(&ui, "ack"));
                Some("dialog resize focus OK")
            }
            33 if ui.component().acked == 1 => {
                assert_eq!(ui.active_dialog(), None);
                assert!(focused(&ui, "message"));
                Some("message acknowledge OK")
            }
            34 if ui.component().done => break,
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
