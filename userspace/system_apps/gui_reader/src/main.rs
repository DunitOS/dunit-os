#![no_std]
#![no_main]
//! Read-only Text & Markdown Viewer built strictly on standard SDK typography and Scroll controls.

extern crate alloc;

use alloc::{format, string::String, vec, vec::Vec};
use dunit::{
    args, errno, fs,
    process::Child,
    time::Duration,
    ui::{Component, ComponentWindow, DialogAction, InputOptions, Key, MenuItem, View},
    window::{WindowEvent, KEYMOD_CTRL},
    Error, Result,
};
use dunit_editor::{parse_markdown, DocBlock};

const W: u32 = 640;
const H: u32 = 460;
const STYLE: &str = "\
Column#root { gap: 4; padding: 6; background: $window-bg; } \
Row#toolbar { gap: 6; padding: 4; } \
Column#doc_col { gap: 6; padding: 8; } \
Text.h1 { font-size: 20; color: $accent; } \
Text.h2 { font-size: 16; color: $accent; } \
Text.h3 { font-size: 14; color: $text; } \
Text.para { font-size: 13; } \
Text.bullet { font-size: 13; padding: 0 0 0 14; } \
Text.quote { font-size: 13; padding: 0 0 0 12; color: $accent; } \
Text.code { font-size: 12; background: $surface; padding: 6; } \
Text.rule { font-size: 11; color: $accent; } \
Text.hit { background: $accent; } \
Text#status { font-size: 12; padding: 4; }";

#[derive(Clone, Debug)]
enum DialogMode {
    None,
    Open(String),
}

#[derive(Clone)]
enum Msg {
    OpenPrompt,
    FilterChanged(String),
    ClearFilter,
    EditInEditor,
    DialogPath(String),
    DialogConfirm,
    DialogCancel,
    Quit,
}

struct ReaderApp {
    path: String,
    blocks: Vec<DocBlock>,
    raw_len: usize,
    filter: String,
    status: String,
    dialog: DialogMode,
    launch_editor: bool,
    should_quit: bool,
}

impl ReaderApp {
    fn new(path: String) -> Self {
        let mut app = Self {
            path: path.clone(),
            blocks: Vec::new(),
            raw_len: 0,
            filter: String::new(),
            status: String::new(),
            dialog: DialogMode::None,
            launch_editor: false,
            should_quit: false,
        };
        app.load_file(&path);
        app
    }

    fn load_file(&mut self, path: &str) {
        self.path = String::from(path);
        match fs::read_to_string(path) {
            Ok(text) => {
                self.raw_len = text.len();
                self.blocks = parse_markdown(&text);
                self.status = format!("{} | {} blocks ({} B)", path, self.blocks.len(), self.raw_len);
            }
            Err(e) => {
                self.blocks = vec![DocBlock::Paragraph(format!("Unable to open {}: {}", path, e.name()))];
                self.status = format!("Open failed: {}", e.name());
            }
        }
    }
}

impl Component for ReaderApp {
    type Message = Msg;

    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::OpenPrompt => self.dialog = DialogMode::Open(self.path.clone()),
            Msg::FilterChanged(q) => {
                self.filter = q;
            }
            Msg::ClearFilter => self.filter.clear(),
            Msg::EditInEditor => self.launch_editor = true,
            Msg::DialogPath(p) => {
                if let DialogMode::Open(cur) = &mut self.dialog {
                    *cur = p;
                }
            }
            Msg::DialogConfirm => {
                if let DialogMode::Open(p) = core::mem::replace(&mut self.dialog, DialogMode::None) {
                    self.load_file(&p);
                }
            }
            Msg::DialogCancel => self.dialog = DialogMode::None,
            Msg::Quit => self.should_quit = true,
        }
    }

    fn view(&self) -> View<Msg> {
        let menu = View::menu_bar(
            "menubar",
            vec![
                View::menu(
                    "m_file",
                    "File",
                    vec![
                        MenuItem::action("mi_open", "Open Document...", Msg::OpenPrompt).mnemonic('o'),
                        MenuItem::action("mi_edit", "Open in Editor", Msg::EditInEditor).mnemonic('e'),
                        MenuItem::separator("mi_sep"),
                        MenuItem::action("mi_quit", "Exit", Msg::Quit).mnemonic('x'),
                    ],
                )
                .menu_mnemonic('f'),
            ],
        );

        let toolbar = View::row(
            "toolbar",
            vec![
                View::button("btn_open", "Open", Msg::OpenPrompt).mnemonic('o'),
                View::input(
                    "in_filter",
                    "Filter",
                    self.filter.clone(),
                    Msg::FilterChanged,
                    InputOptions {
                        placeholder: String::from("Search text..."),
                        ..Default::default()
                    },
                ),
                View::button("btn_clear", "Clear", Msg::ClearFilter).mnemonic('c'),
                View::button("btn_edit", "Edit", Msg::EditInEditor).mnemonic('e'),
            ],
        );

        let needle = self.filter.trim();
        let mut nodes = Vec::new();
        let mut matches = 0usize;
        for (i, block) in self.blocks.iter().enumerate() {
            let key = format!("blk_{}", i);
            let (class, content) = match block {
                DocBlock::Heading { level: 1, text } => ("h1", text.clone()),
                DocBlock::Heading { level: 2, text } => ("h2", text.clone()),
                DocBlock::Heading { text, .. } => ("h3", text.clone()),
                DocBlock::Paragraph(text) => ("para", text.clone()),
                DocBlock::Bullet { indent, text } => (
                    "bullet",
                    format!("{}- {}", "  ".repeat(*indent as usize), text),
                ),
                DocBlock::Numbered { index, text } => ("bullet", format!("{}. {}", index, text)),
                DocBlock::Quote(text) => ("quote", format!("> {}", text)),
                DocBlock::CodeBlock { lang, text } => {
                    let hdr = if lang.is_empty() {
                        text.clone()
                    } else {
                        format!("[{}]\n{}", lang, text)
                    };
                    ("code", hdr)
                }
                DocBlock::Rule => ("rule", String::from("----------------------------------------")),
            };
            let hit = !needle.is_empty() && content.to_ascii_lowercase().contains(&needle.to_ascii_lowercase());
            if hit {
                matches += 1;
            }
            let mut node = View::text(key, content).class(class);
            if hit {
                node = node.class("hit");
            }
            nodes.push(node);
        }
        if nodes.is_empty() {
            nodes.push(View::text("empty", "(Empty document)").class("para"));
        }

        let scroll = View::scroll(
            "doc_scroll",
            "Document Content",
            vec![View::column("doc_col", nodes)],
        );

        let filter_info = if needle.is_empty() {
            String::new()
        } else {
            format!(" | matches: {}", matches)
        };
        let status = View::status_bar("status", "Reader Status", format!("{}{}", self.status, filter_info));

        let root = View::column("root", vec![menu, toolbar, scroll, status]);

        let dialog = match &self.dialog {
            DialogMode::None => None,
            DialogMode::Open(p) => Some(View::dialog(
                "dlg_open",
                "Open Document",
                View::input("dlg_open_in", "Document path", p.clone(), Msg::DialogPath, InputOptions::default())
                    .on_submit(Msg::DialogConfirm),
                vec![
                    DialogAction::new("ok", "Open", Msg::DialogConfirm),
                    DialogAction::new("cancel", "Cancel", Msg::DialogCancel),
                ],
                Some("ok"),
                Some(Msg::DialogCancel),
            )),
        };
        View::modal("modal_root", root, dialog)
            .on_key(Key { character: 'o', modifiers: KEYMOD_CTRL }, Msg::OpenPrompt)
            .on_key(Key { character: 'q', modifiers: KEYMOD_CTRL }, Msg::Quit)
    }
}

#[dunit::main]
fn main() -> Result<()> {
    let argv = args::args()?;
    let mut path = if fs::metadata("/m9_doc.md").is_ok() {
        String::from("/m9_doc.md")
    } else {
        String::from("/assets/legal/README")
    };
    let mut self_test = fs::metadata("/m9_run_theme_check").is_ok();
    for arg in argv.iter().skip(1) {
        if arg == "--self-test" {
            self_test = true;
        } else {
            path = arg.clone();
        }
    }
    if fs::metadata(&path).is_err() && path == "/assets/legal/README" {
        path = String::from("/system/share/network/defaults.toml");
    }

    let app = ReaderApp::new(path.clone());
    let mut ui = ComponentWindow::open_themed_with_style(W, H, app, STYLE)?;
    dunit::println!("[GUI-READER] opened {} blocks={}", path, ui.component().blocks.len())?;

    if self_test {
        for _ in 0..5 {
            let _ = ui.next_event(Duration::from_millis(120));
        }
        dunit::println!("[GUI-READER] self-test OK blocks={}", ui.component().blocks.len())?;
        return ui.close();
    }

    loop {
        if ui.component().launch_editor {
            let p = ui.component().path.clone();
            let _ = Child::spawn_with_env("/app/gui_editor", &["/app/gui_editor", &p], &[]);
            ui.update(Msg::DialogCancel)?;
            return ui.close();
        }
        if ui.component().should_quit {
            return ui.close();
        }
        match ui.next_event(Duration::from_millis(200)) {
            Ok(Some(WindowEvent::CloseRequested)) => return ui.close(),
            Ok(_) => {}
            Err(Error(errno::EPIPE)) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}
