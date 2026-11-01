extern crate alloc;
use alloc::{format, vec};
use dunit::ui::{
    Attrs, Component, ComponentWindow, Key, SelectionMode, Sizing, TreeItem as I, TreeModel,
    TreeState, View,
};
use dunit::{
    time::Duration,
    window::{WindowEvent, IN_UP},
};
struct Demo {
    model: TreeModel,
    state: TreeState,
    reject: bool,
    calls: usize,
    done: bool,
}
#[derive(Clone)]
enum Msg {
    State(TreeState),
    Reject,
    CollapseRoot,
    Done,
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, m: Msg) {
        match m {
            Msg::State(s) => {
                self.calls += 1;
                if !self.reject {
                    self.state = s;
                }
            }
            Msg::Reject => self.reject = true,
            Msg::CollapseRoot => {
                self.state.expanded.remove("root");
                self.state.retain(&self.model);
            }
            Msg::Done => self.done = true,
        }
    }
    fn view(&self) -> View<Msg> {
        View::column(
            "host",
            vec![
                View::text("title", "Project tree - 10,000 workspace files").layout(Attrs {
                    height: Sizing::Px(28.),
                    ..Default::default()
                }),
                View::tree_view(
                    "tree",
                    "Project",
                    self.model.clone(),
                    self.state.clone(),
                    SelectionMode::Multiple,
                    Msg::State,
                )
                .layout(Attrs {
                    width: Sizing::Grow(1.),
                    height: Sizing::Grow(1.),
                    ..Default::default()
                }),
                View::status_bar(
                    "status",
                    "Tree status",
                    format!(
                        "{} selected | current {} | {} expanded",
                        self.state.selection.selected.len(),
                        self.state.selection.current.as_deref().unwrap_or("none"),
                        self.state.expanded.len()
                    ),
                )
                .layout(Attrs {
                    height: Sizing::Px(28.),
                    ..Default::default()
                }),
            ],
        )
        .on_key(Key::plain('r'), Msg::Reject)
        .on_key(Key::plain('c'), Msg::CollapseRoot)
        .on_key(Key::plain('q'), Msg::Done)
    }
}
fn marker(s: &str) -> dunit::Result<()> {
    use dunit::io::Write;
    dunit::io::stdout().write_all(format!("[TREE] {s}\n").as_bytes())
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut items = vec![
        I::new("root", "Workspace"),
        I::new("src", "Sources").under("root"),
        I::new("main", "main.rs").under("src"),
        I::new("ui", "ui.rs").under("src"),
        I::new("disabled", "Disabled branch")
            .under("root")
            .disabled(true),
        I::new("blocked", "Disabled descendant").under("disabled"),
        I::new("assets", "Assets").under("root"),
        I::new("font", "font.ttf").under("assets"),
        I::new("many", "Generated files").under("root"),
    ];
    items.extend(
        (0..10000).map(|i| I::new(format!("f{i}"), format!("File {i:05}.rs")).under("many")),
    );
    let mut ui = ComponentWindow::open_themed(
        640,
        400,
        Demo {
            model: TreeModel::new(items).unwrap(),
            state: TreeState::default(),
            reject: false,
            calls: 0,
            done: false,
        },
    )?;
    assert_eq!(ui.list_visible_rows("tree"), Some(1));
    marker("ready")?;
    let mut stage = 0;
    let mut calls = 0;
    let mut wheel_from = 0.;
    loop {
        let before = ui.list_geometry("tree").unwrap();
        let event = ui.next_event(Duration::from_millis(100))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let m = ui.component();
        let current = m.state.selection.current.as_deref();
        let status = match stage {
            0 if ui.view().children()[1].interaction().focused => Some("tab focus OK"),
            1 if current == Some("root") => Some("root selection OK"),
            2 if m.state.expanded.contains("root") => {
                assert_eq!(ui.list_visible_rows("tree"), Some(5));
                Some("root expand OK")
            }
            3 if current == Some("src") => Some("right enters child OK"),
            4 if m.state.expanded.contains("src") => Some("nested expand OK"),
            5 if current == Some("main") => {
                let a = ui.view().children()[1].tree_item("main").unwrap();
                assert_eq!(a.level, 3);
                Some("nested child metadata OK")
            }
            6 if current == Some("ui") => {
                assert_eq!(m.state.selection.selected.len(), 2);
                Some("shift range OK")
            }
            7 if current == Some("assets") => {
                assert_eq!(m.state.selection.selected.len(), 3);
                assert!(!m.state.selection.selected.contains("disabled"));
                Some("disabled branch skip OK")
            }
            8 if current == Some("root") => Some("left parent OK"),
            9 if !m.state.expanded.contains("root") => {
                assert_eq!(ui.list_visible_rows("tree"), Some(1));
                Some("left collapse OK")
            }
            10 if m.state.expanded.contains("root") => Some("pointer disclosure OK"),
            11 if matches!(event,Some(WindowEvent::Input(i)) if i.kind==IN_UP) => {
                assert_eq!(m.state.selection.current.as_deref(), Some("root"));
                Some("pointer release OK")
            }
            12 if current == Some("many") => Some("end visible branch OK"),
            13 if m.state.expanded.contains("many") => {
                assert!(ui.list_visible_rows("tree").unwrap() < 20);
                Some("large branch expand OK")
            }
            14 if current == Some("f9999") => {
                assert!(ui.list_geometry("tree").unwrap().visible().contains(&10006));
                assert!(ui.list_visible_rows("tree").unwrap() < 20);
                wheel_from = ui.scroll_offset("tree").unwrap();
                Some("last file reveal virtualized OK")
            }
            15 if ui.scroll_offset("tree").unwrap() < wheel_from => Some("wheel OK"),
            16 if matches!(event, Some(WindowEvent::Resized { .. })) => {
                assert!(ui.size().0 > 640);
                assert!(ui.list_visible_rows("tree").unwrap() < 40);
                assert_eq!(current, Some("f9999"));
                Some("resize OK")
            }
            17 if !m.state.expanded.contains("root") => {
                assert_eq!(current, Some("root"));
                assert_eq!(m.state.selection.selected.len(), 1);
                assert_eq!(ui.list_visible_rows("tree"), Some(1));
                assert_eq!(ui.scroll_offset("tree"), Some(0.));
                Some("collapse promotes current and clamps OK")
            }
            18 if m.reject => {
                calls = m.calls;
                Some("reject ready")
            }
            19 if m.calls > calls => {
                assert!(!m.state.expanded.contains("root"));
                assert_eq!(ui.list_visible_rows("tree"), Some(1));
                Some("model rejects expansion OK")
            }
            20 if m.done => break,
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
