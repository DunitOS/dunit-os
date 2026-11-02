extern crate alloc;
use alloc::{collections::BTreeSet, format, string::String, vec};
use dunit::{
    time::Duration,
    ui::*,
    window::{WindowEvent, IN_UP},
};

const TEXT: &str = "alpha beta gamma";
const POINTS: [(f32, f32); 3] = [(24., 16.), (80., 16.), (128., 16.)];
struct Demo {
    mode: usize,
    selection: ListSelection,
}
#[derive(Clone)]
enum Msg {
    Next,
    Text(String),
    List(ListSelection),
    Tree(TreeState),
    Table(TableState),
}
impl Component for Demo {
    type Message = Msg;
    fn update(&mut self, message: Msg) {
        match message {
            Msg::Next => {
                self.mode += 1;
                self.selection = ListSelection::default();
            }
            Msg::Text(value) => assert_eq!(value, TEXT),
            Msg::List(value) => self.selection = value,
            Msg::Tree(value) => self.selection = value.selection,
            Msg::Table(value) => self.selection = value.selection,
        }
    }
    fn view(&self) -> View<Msg> {
        let field = match self.mode {
            0 => View::input("field", "Text", TEXT, Msg::Text, InputOptions::default()),
            1 => View::text_area(
                "field",
                "Multiline",
                TEXT,
                Msg::Text,
                InputOptions::default(),
            ),
            2 => View::list(
                "field",
                "List",
                (0..10000)
                    .map(|i| ListItem::new(format!("r{i}"), format!("Row {i}")))
                    .collect(),
                self.selection.clone(),
                SelectionMode::Multiple,
                Msg::List,
            ),
            3 => View::tree_view(
                "field",
                "Tree",
                TreeModel::new(
                    (0..10000)
                        .map(|i| TreeItem::new(format!("r{i}"), format!("Row {i}")))
                        .collect(),
                )
                .unwrap(),
                TreeState {
                    selection: self.selection.clone(),
                    ..Default::default()
                },
                SelectionMode::Multiple,
                Msg::Tree,
            ),
            4 => View::table(
                "field",
                "Table",
                TableModel::new(
                    vec![TableColumn::new("name", "Name").width(600.)],
                    (0..10000)
                        .map(|i| TableRow::new(format!("r{i}"), vec![format!("Row {i}").into()]))
                        .collect(),
                )
                .unwrap(),
                TableState {
                    selection: self.selection.clone(),
                    ..Default::default()
                },
                SelectionMode::Multiple,
                Msg::Table,
            ),
            _ => View::text("done", "Done"),
        };
        field
            .layout(Attrs {
                width: Sizing::Grow(1.),
                height: Sizing::Grow(1.),
                ..Default::default()
            })
            .on_key(
                Key {
                    character: 'n',
                    modifiers: 2,
                },
                Msg::Next,
            )
    }
}
fn marker(mode: usize, phase: &str) -> dunit::Result<()> {
    use dunit::io::Write;
    dunit::io::stdout().write_all(format!("[MODIFIERS] mode={mode} {phase}\n").as_bytes())
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = ComponentWindow::open_themed(
        640,
        360,
        Demo {
            mode: 0,
            selection: ListSelection::default(),
        },
    )?;
    assert!(ui.input_lifecycle());
    let (theme, font) = load_theme("")?;
    let mut reference = Renderer::new(font, theme.cascade, theme.background);
    let mut mode = usize::MAX;
    let mut stage = 0;
    let mut indices = [0; 3];
    let mut word_b = 0..0;
    let mut word_c = 0..0;
    loop {
        let current = ui.component().mode;
        if current != mode {
            assert!(mode == usize::MAX || stage == 4);
            mode = current;
            stage = 0;
            if mode == 5 {
                break;
            }
            let (w, h) = ui.size();
            reference.render(ui.view(), w, h).unwrap();
            if mode < 2 {
                indices = POINTS.map(|(x, y)| reference.input_index_at("field", x, y).unwrap());
                assert!(indices[0] < indices[1] && indices[1] < indices[2]);
                let mut probe = TextEditor::new(TEXT);
                probe.select_word(indices[1]);
                word_b = probe.selection();
                probe.select_word(indices[2]);
                word_c = probe.selection();
                assert!(word_b.start < word_c.end);
            } else {
                let g = ui.list_geometry("field").unwrap();
                indices = [80., 144., 208.].map(|y| g.row_at(100., y).unwrap());
                assert!(indices[0] < indices[1] && indices[1] < indices[2]);
                assert!(ui.list_visible_rows("field").unwrap() < 30);
            }
            marker(mode, "ready")?;
        }
        let event = ui.next_event(Duration::from_secs(20))?;
        assert!(!matches!(event, Some(WindowEvent::CloseRequested)));
        let Some(WindowEvent::Input(input)) = event else {
            continue;
        };
        if input.kind != IN_UP {
            continue;
        }
        let expected_mods = if mode < 2 { [0, 1, 2, 3] } else { [0, 2, 1, 3] };
        assert_eq!(input.pointer_modifiers(), expected_mods[stage]);
        if mode < 2 {
            let editor = ui.view().text_editor().unwrap();
            let expected = match stage {
                0 => indices[0]..indices[0],
                1 => indices[0]..indices[1],
                2 => word_b.clone(),
                3 => word_b.start..word_c.end,
                _ => panic!("extra click"),
            };
            assert_eq!(editor.selection(), expected);
            assert_eq!(editor.text(), TEXT);
            assert!(!editor.can_undo());
        } else {
            let [a, b, c] = indices;
            let expected: BTreeSet<String> = match stage {
                0 => [a].into_iter().map(|i| format!("r{i}")).collect(),
                1 => [a, b].into_iter().map(|i| format!("r{i}")).collect(),
                2 => (b..=c).map(|i| format!("r{i}")).collect(),
                3 => (a..=c).map(|i| format!("r{i}")).collect(),
                _ => panic!("extra click"),
            };
            assert_eq!(ui.component().selection.selected, expected);
            assert!(ui.list_visible_rows("field").unwrap() < 30);
        }
        stage += 1;
        marker(mode, &format!("click={stage} OK"))?;
    }
    ui.close()?;
    use dunit::io::Write;
    dunit::io::stdout().write_all(b"[MODIFIERS] ok\n")?;
    Ok(())
}
