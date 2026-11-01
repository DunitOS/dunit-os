extern crate alloc;
use alloc::{format, string::String, vec};
use dunit::ui::*;
use dunit::{fs, time::Duration, window::WindowEvent};
struct Demo {
    gestures: bool,
    wrap: bool,
    text: String,
    captures: usize,
    bubbles: usize,
    model: TableModel,
    state: TableState,
    rename: Option<RenameState>,
    renamed: bool,
    picker: Option<FileDialog<NativeDirectorySource>>,
    accepted: bool,
    cancelled: bool,
    done: bool,
}
#[derive(Clone)]
enum Msg {
    Table(TableState),
    Rename(RenameEvent),
    Open,
    CancelPicker,
    Picker(FileDialogMessage),
    Gestures,
    Wrap,
    Text(String),
    Route(EventPhase),
    Done,
}
fn labels() -> FileDialogLabels {
    FileDialogLabels {
        location: "Location".into(),
        name: "File name".into(),
        files: "Files".into(),
        back: "Back".into(),
        forward: "Forward".into(),
        up: "Up".into(),
        refresh: "Refresh".into(),
        enter: "Open folder".into(),
        accept: "Save".into(),
        cancel: "Cancel".into(),
        hidden: "Hidden".into(),
        filter: "Type".into(),
        name_column: "Name".into(),
        kind_column: "Kind".into(),
        size_column: "Bytes".into(),
        file_kind: "File".into(),
        directory_kind: "Folder".into(),
        device_kind: "Device".into(),
        replace_title: "Replace file?".into(),
        replace_message: "The file already exists. Replace it?".into(),
        replace_accept: "Replace".into(),
        invalid_path: "Invalid path".into(),
        invalid_name: "Invalid name".into(),
        missing_file: "Missing file".into(),
        not_directory: "Not a folder".into(),
        invalid_entries: "Invalid directory entries".into(),
    }
}
fn picker() -> FileDialog<NativeDirectorySource> {
    FileDialog::new(
        NativeDirectorySource,
        FileDialogOptions {
            key: "picker".into(),
            title: "Save project".into(),
            directory: "/tmp/sdk-collections".into(),
            filename: "project.txt".into(),
            mode: FileDialogMode::Save,
            filters: vec![FileFilter::new("text", "Text files", vec!["txt".into()])],
            labels: labels(),
        },
    )
    .unwrap()
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::Gestures => self.gestures = true,
            Msg::Wrap => {
                self.wrap = true;
                self.text = "alpha beta gamma delta epsilon zeta eta theta ".repeat(12);
            }
            Msg::Text(s) => self.text = s,
            Msg::Route(EventPhase::Capture) => self.captures += 1,
            Msg::Route(EventPhase::Bubble) => self.bubbles += 1,
            Msg::Table(s) => self.state = s,
            Msg::Rename(e) => match e {
                RenameEvent::Begin { row, column } => {
                    let i = self.model.rows().iter().position(|r| r.key == row).unwrap();
                    self.rename = Some(RenameState {
                        row,
                        column,
                        value: self.model.rows()[i].cells[0].text(),
                    });
                }
                RenameEvent::Change(s) => self.rename.as_mut().unwrap().value = s,
                RenameEvent::Cancel => self.rename = None,
                RenameEvent::Commit => {
                    let r = self.rename.take().unwrap();
                    let mut rows = self.model.rows().to_vec();
                    let i = self
                        .model
                        .rows()
                        .iter()
                        .position(|item| item.key == r.row)
                        .unwrap();
                    rows[i].cells[0] = r.value.into();
                    self.model = TableModel::new(self.model.columns().to_vec(), rows).unwrap();
                    self.renamed = true;
                }
            },
            Msg::Open | Msg::CancelPicker => self.picker = Some(picker()),
            Msg::Picker(m) => {
                let p = self.picker.as_mut().unwrap();
                p.update(m);
                if let Some(outcome) = p.outcome() {
                    match outcome {
                        FileDialogOutcome::Accepted(path) => {
                            assert_eq!(path, "/tmp/sdk-collections/project.txt");
                            self.accepted = true;
                        }
                        FileDialogOutcome::Cancelled => self.cancelled = true,
                    };
                    self.picker = None;
                }
            }
            Msg::Done => self.done = true,
        }
    }
    fn view(&self) -> View<Msg> {
        let children = if self.gestures {
            vec![
                View::text_area(
                    "gesture",
                    "Text gestures",
                    &self.text,
                    Msg::Text,
                    InputOptions {
                        soft_wrap: self.wrap,
                        ..Default::default()
                    },
                )
                .layout(Attrs {
                    width: if self.wrap {
                        Sizing::Pct(0.5)
                    } else {
                        Sizing::Px(640.)
                    },
                    height: Sizing::Px(if self.wrap { 64. } else { 120. }),
                    ..Default::default()
                }),
                View::text(
                    "gesture-help",
                    "Double click selects a word; triple click selects a line.",
                ),
            ]
        } else {
            vec![
                View::text("title", "Virtual table - 10,000 project files"),
                View::table(
                    "table",
                    "Project files",
                    self.model.clone(),
                    self.state.clone(),
                    SelectionMode::Multiple,
                    Msg::Table,
                )
                .renaming(self.rename.clone(), Msg::Rename)
                .layout(Attrs {
                    width: Sizing::Grow(1.),
                    height: Sizing::Grow(1.),
                    ..Default::default()
                }),
                View::status_bar(
                    "status",
                    "Selection",
                    format!(
                        "Current: {}",
                        self.state.selection.current.as_deref().unwrap_or("none")
                    ),
                ),
            ]
        };
        View::modal(
            "root",
            View::column("content", children),
            self.picker
                .as_ref()
                .map(|p| p.view().map_messages(Msg::Picker)),
        )
        .on_key(Key::plain('o'), Msg::Open)
        .on_key(Key::plain('c'), Msg::CancelPicker)
        .on_key(Key::plain('t'), Msg::Gestures)
        .on_key(
            Key {
                character: 'q',
                modifiers: 2,
            },
            Msg::Done,
        )
        .on_key(
            Key {
                character: 'w',
                modifiers: 2,
            },
            Msg::Wrap,
        )
        .on_capture(capture)
        .on_bubble(bubble)
    }
}
fn routed(e: UiEvent) -> EventResponse<Msg> {
    if matches!(e.kind, UiEventKind::NativeKey { scancode: 0x3e, .. }) {
        EventResponse::message(Msg::Route(e.phase))
    } else {
        EventResponse::default()
    }
}
fn capture(e: UiEvent) -> EventResponse<Msg> {
    routed(e)
}
fn bubble(e: UiEvent) -> EventResponse<Msg> {
    routed(e)
}
fn marker(s: &str) -> dunit::Result<()> {
    use dunit::io::Write;
    dunit::io::stdout().write_all(format!("[COLLECTIONS] {s}\n").as_bytes())
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    fs::create_dir("/tmp/sdk-collections")?;
    fs::create_dir("/tmp/sdk-collections/src")?;
    fs::write("/tmp/sdk-collections/project.txt", b"original")?;
    fs::write("/tmp/sdk-collections/hidden.bin", b"filtered")?;
    let mut ui = ComponentWindow::open_themed(
        720,
        560,
        Demo {
            gestures: false,
            wrap: false,
            text: "alpha beta\nthird line".into(),
            captures: 0,
            bubbles: 0,
            model: TableModel::new(
                vec![
                    TableColumn::new("name", "Name").editable(true).width(320.),
                    TableColumn::new("size", "Bytes").width(220.),
                    TableColumn::new("type", "Type").width(220.),
                ],
                (0..10000)
                    .map(|i| {
                        TableRow::new(
                            format!("r{i}"),
                            vec![
                                format!("File {i:05}.rs").into(),
                                TableValue::Integer(i),
                                "Rust source".into(),
                            ],
                        )
                    })
                    .collect(),
            )
            .unwrap(),
            state: TableState::default(),
            rename: None,
            renamed: false,
            picker: None,
            accepted: false,
            cancelled: false,
            done: false,
        },
    )?;
    marker("ready")?;
    let mut stage = -1;
    let mut visual_end = 0;
    loop {
        let event = ui.next_event(Duration::from_millis(100))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let m = ui.component();
        let s = match stage {
            -1 if ui.scrollbars_visible("table") == Some(false) => Some("idle bars hidden"),
            0 if ui.view().children()[0].children()[1].interaction().focused => {
                assert_eq!(ui.scrollbars_visible("table"), Some(true));
                Some("focus")
            }
            1 if m.state.selection.current.as_deref() == Some("r0") => Some("selection"),
            2 if m.state.sort.is_some() => {
                assert_eq!((m.captures, m.bubbles), (1, 1));
                Some("sort")
            }
            3 if m
                .state
                .sort
                .as_ref()
                .is_some_and(|s| s.order == SortOrder::Descending) =>
            {
                Some("pointer header descending")
            }
            4 if m
                .state
                .sort
                .as_ref()
                .is_some_and(|s| s.order == SortOrder::Ascending) =>
            {
                Some("pointer header ascending")
            }
            5 if m.rename.is_some() => Some("rename begin"),
            6 if m.rename.as_ref().is_some_and(|r| r.value == "z") => Some("rename edit"),
            7 if m.renamed => {
                assert_eq!(m.model.rows()[0].cells[0].text(), "z");
                Some("rename commit")
            }
            8 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==dunit::window::IN_KEY&&i.x==0x4f)
                && m.state.selection.current.as_deref() == Some("r0")
                && ui.scroll_offset("table").unwrap() > 0. =>
            {
                assert!(ui.list_visible_rows("table").unwrap() < 30);
                Some("virtual end")
            }
            9 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 720);
                Some("resize")
            }
            10 if m.picker.is_some() => {
                let p = m.picker.as_ref().unwrap();
                assert_eq!(p.entries().len(), 2);
                assert!(p.entries().iter().any(|e| e.kind == FileKind::Directory));
                Some("picker")
            }
            11 if ui.view().children().last().unwrap().key() == "picker/replace" => {
                Some("overwrite")
            }
            12 if ui.view().children().last().unwrap().key() == "picker" => Some("decline"),
            13 if ui.view().children().last().unwrap().key() == "picker/replace" => {
                Some("overwrite again")
            }
            14 if m.accepted => {
                assert_eq!(
                    fs::read_to_string("/tmp/sdk-collections/project.txt")?,
                    "original"
                );
                Some("accepted")
            }
            15 if m.picker.is_some() => Some("cancel picker"),
            16 if m.cancelled => Some("cancelled"),
            17 if m.gestures => Some("gestures ready"),
            18 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==dunit::window::IN_UP) => {
                assert!(ui.view().children()[0].children()[0]
                    .text_editor()
                    .unwrap()
                    .selection()
                    .is_empty());
                Some("single click")
            }
            19 if ui.view().children()[0].children()[0]
                .text_editor()
                .is_some_and(|e| e.selection() == (0..5)) =>
            {
                Some("double word")
            }
            20 if ui.view().children()[0].children()[0]
                .text_editor()
                .is_some_and(|e| e.selection() == (0..11)) =>
            {
                Some("triple line")
            }
            21 if m.text == "zthird line" => Some("selection replaced"),
            22 if m.text == "alpha beta\nthird line" => Some("selection undo"),
            23 if m.wrap => Some("wrap ready"),
            24 if ui.view().children()[0].children()[0]
                .text_editor()
                .unwrap()
                .cursor()
                == 0 =>
            {
                Some("wrap home")
            }
            25 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==dunit::window::IN_KEY&&i.x==0x4f) =>
            {
                let e = ui.view().children()[0].children()[0].text_editor().unwrap();
                assert!(e.cursor() > 0 && e.cursor() < m.text.len());
                assert_eq!(ui.input_scroll_x("gesture"), Some(0.));
                visual_end = e.cursor();
                Some("wrap visual end")
            }
            26 if ui.view().children()[0].children()[0]
                .text_editor()
                .unwrap()
                .cursor()
                > visual_end =>
            {
                Some("wrap visual down")
            }
            27 if !ui.view().children()[0].children()[0]
                .text_editor()
                .unwrap()
                .selection()
                .is_empty() =>
            {
                Some("wrap visual selection")
            }
            28 if ui.view().children()[0].children()[0]
                .text_editor()
                .unwrap()
                .cursor()
                == m.text.len() =>
            {
                assert!(ui.input_scroll_y("gesture").unwrap() > 0.);
                assert_eq!(ui.input_scroll_x("gesture"), Some(0.));
                Some("wrap document end")
            }
            29 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert_eq!(ui.size().0, 720);
                assert_eq!(
                    ui.view().children()[0].children()[0]
                        .text_editor()
                        .unwrap()
                        .cursor(),
                    m.text.len()
                );
                assert_eq!(ui.input_scroll_x("gesture"), Some(0.));
                Some("wrap resize")
            }
            30 if m.done => break,
            _ => None,
        };
        if let Some(s) = s {
            marker(s)?;
            stage += 1;
        }
    }
    ui.close()?;
    marker("ok")?;
    dunit::println!("[WIDGETS] ok")?;
    Ok(())
}
