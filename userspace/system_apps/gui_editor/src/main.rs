#![no_std]
#![no_main]
//! SDK GUI Text Editor backed by the shared `dunit_editor::DocumentBuffer`.

extern crate alloc;

use alloc::{borrow::ToOwned, format, string::String, vec, vec::Vec};
use dunit::{
    args, errno, fs,
    process::Child,
    time::Duration,
    ui::{
        Component, ComponentWindow, DialogAction, InputOptions, Key, MenuItem, View,
    },
    window::{WindowEvent, KEYMOD_CTRL},
    Error, Result,
};
use dunit_editor::{DocumentBuffer, Language, SHORTCUT_HINTS};

const W: u32 = 680;
const H: u32 = 460;
const STYLE: &str = "\
Column#root_body { gap: 4; padding: 4; background: $window-bg; } \
Row#toolbar { gap: 4; padding: 4; } \
Textarea#editor_body { padding: 8; font-size: 13; height: fill; } \
Text#status { padding: 4; font-size: 12; } \
Column#help_box { gap: 4; padding: 6; }";

#[derive(Clone, Debug)]
enum DialogMode {
    None,
    Open(String),
    SaveAs(String),
    Find(String),
    Replace { query: String, with: String },
    GotoLine(String),
    Help,
}

#[derive(Clone)]
enum Msg {
    TextChanged(String),
    OpenPrompt,
    Save,
    SaveAsPrompt,
    FindPrompt,
    FindNext,
    ReplacePrompt,
    GotoPrompt,
    Undo,
    Redo,
    SelectAll,
    CompileC,
    CompileFinished {
        saved_path: Option<String>,
        note: String,
    },
    HelpPrompt,
    DialogField1(String),
    DialogField2(String),
    DialogConfirm,
    ReplaceAllConfirm,
    DialogCancel,
    Quit,
}

struct EditorApp {
    doc: DocumentBuffer,
    dialog: DialogMode,
    last_query: String,
    note: String,
    should_quit: bool,
    pending_compile: bool,
}

impl EditorApp {
    fn new(doc: DocumentBuffer) -> Self {
        let note = doc
            .path()
            .map(|p| format!("Opened {}", p))
            .unwrap_or_else(|| String::from("New document"));
        Self {
            doc,
            dialog: DialogMode::None,
            last_query: String::new(),
            note,
            should_quit: false,
            pending_compile: false,
        }
    }

    fn save_current(&mut self, path: &str) {
        if path.trim().is_empty() {
            self.note = String::from("Save failed: empty path");
            return;
        }
        let n = self.doc.bytes_len();
        let tmp = format!("{}.ed.tmp", path);
        let _ = fs::remove_file(&tmp);
        let res = if fs::write(&tmp, self.doc.text().as_bytes()).is_ok() {
            let _ = fs::remove_file(path);
            fs::rename(&tmp, path).or_else(|_| fs::write(path, self.doc.text().as_bytes()))
        } else {
            fs::write(path, self.doc.text().as_bytes())
        };
        match res {
            Ok(()) => {
                self.doc.set_path(String::from(path));
                self.doc.mark_saved();
                let hash = self.doc.content_sha256_hex();
                self.note = format!("Saved {} ({} B, sha256:{})", path, n, &hash[..12]);
            }
            Err(e) => self.note = format!("Save error: {}", e.name()),
        }
    }

    fn open_path(&mut self, path: &str) {
        match fs::read(path) {
            Ok(bytes) => match DocumentBuffer::from_bytes(Some(String::from(path)), &bytes) {
                Ok(doc) => {
                    self.doc = doc;
                    self.note = format!("Loaded {}", path);
                }
                Err(_) => self.note = String::from("Unsupported UTF-8 or file too large"),
            },
            Err(e) => self.note = format!("Open error: {}", e.name()),
        }
    }
}

#[inline(never)]
fn editor_menu() -> View<Msg> {
    View::menu_bar(
        "menubar",
        vec![
            View::menu(
                "menu_file",
                "File",
                vec![
                    MenuItem::action("mi_open", "Open...", Msg::OpenPrompt).mnemonic('o'),
                    MenuItem::action("mi_save", "Save", Msg::Save).mnemonic('s'),
                    MenuItem::action("mi_save_as", "Save As...", Msg::SaveAsPrompt).mnemonic('a'),
                    MenuItem::separator("mi_fsep"),
                    MenuItem::action("mi_compile", "Compile & Run C", Msg::CompileC).mnemonic('c'),
                    MenuItem::separator("mi_qsep"),
                    MenuItem::action("mi_quit", "Exit", Msg::Quit).mnemonic('x'),
                ],
            )
            .menu_mnemonic('f'),
            View::menu(
                "menu_edit",
                "Edit",
                vec![
                    MenuItem::action("mi_undo", "Undo", Msg::Undo).mnemonic('u'),
                    MenuItem::action("mi_redo", "Redo", Msg::Redo).mnemonic('r'),
                    MenuItem::separator("mi_esep"),
                    MenuItem::action("mi_selall", "Select All", Msg::SelectAll).mnemonic('l'),
                ],
            )
            .menu_mnemonic('e'),
            View::menu(
                "menu_search",
                "Search",
                vec![
                    MenuItem::action("mi_find", "Find...", Msg::FindPrompt).mnemonic('f'),
                    MenuItem::action("mi_next", "Find Next", Msg::FindNext).mnemonic('n'),
                    MenuItem::action("mi_replace", "Replace...", Msg::ReplacePrompt).mnemonic('h'),
                    MenuItem::action("mi_goto", "Go to Line...", Msg::GotoPrompt).mnemonic('g'),
                ],
            )
            .menu_mnemonic('s'),
            View::menu(
                "menu_help",
                "Help",
                vec![MenuItem::action("mi_help", "Shortcuts", Msg::HelpPrompt).mnemonic('k')],
            )
            .menu_mnemonic('h'),
        ],
    )
}

#[inline(never)]
fn editor_toolbar(can_undo: bool, can_redo: bool) -> View<Msg> {
    View::toolbar(
        "toolbar",
        "Editor Actions",
        vec![
            View::button("btn_open", "Open", Msg::OpenPrompt).mnemonic('o'),
            View::button("btn_save", "Save", Msg::Save).class("primary").mnemonic('w'),
            View::toolbar_separator("tsep1"),
            View::button("btn_undo", "Undo", Msg::Undo)
                .disabled(!can_undo)
                .mnemonic('z'),
            View::button("btn_redo", "Redo", Msg::Redo)
                .disabled(!can_redo)
                .mnemonic('y'),
            View::toolbar_separator("tsep2"),
            View::button("btn_find", "Find", Msg::FindPrompt).mnemonic('n'),
            View::button("btn_replace", "Replace", Msg::ReplacePrompt).mnemonic('r'),
            View::button("btn_compile", "Run C", Msg::CompileC).mnemonic('b'),
        ],
    )
}

#[inline(never)]
fn editor_dialog(dialog: &DialogMode) -> Option<View<Msg>> {
    match dialog {
        DialogMode::None => None,
        DialogMode::Open(val) => Some(View::dialog(
            "dlg_open",
            "Open File Path",
            View::input("dlg_open_in", "Path", val.clone(), Msg::DialogField1, InputOptions::default())
                .on_submit(Msg::DialogConfirm),
            vec![
                DialogAction::new("ok", "Open", Msg::DialogConfirm),
                DialogAction::new("cancel", "Cancel", Msg::DialogCancel),
            ],
            Some("ok"),
            Some(Msg::DialogCancel),
        )),
        DialogMode::SaveAs(val) => Some(View::dialog(
            "dlg_save",
            "Save File As",
            View::input("dlg_save_in", "Path", val.clone(), Msg::DialogField1, InputOptions::default())
                .on_submit(Msg::DialogConfirm),
            vec![
                DialogAction::new("ok", "Save", Msg::DialogConfirm),
                DialogAction::new("cancel", "Cancel", Msg::DialogCancel),
            ],
            Some("ok"),
            Some(Msg::DialogCancel),
        )),
        DialogMode::Find(val) => Some(View::dialog(
            "dlg_find",
            "Find Text",
            View::input("dlg_find_in", "Query", val.clone(), Msg::DialogField1, InputOptions::default())
                .on_submit(Msg::DialogConfirm),
            vec![
                DialogAction::new("ok", "Find", Msg::DialogConfirm),
                DialogAction::new("cancel", "Cancel", Msg::DialogCancel),
            ],
            Some("ok"),
            Some(Msg::DialogCancel),
        )),
        DialogMode::Replace { query, with } => Some(View::dialog(
            "dlg_replace",
            "Search & Replace",
            View::column(
                "dlg_rep_col",
                vec![
                    View::input("dlg_rep_q", "Find", query.clone(), Msg::DialogField1, InputOptions::default()),
                    View::input("dlg_rep_w", "Replace", with.clone(), Msg::DialogField2, InputOptions::default())
                        .on_submit(Msg::DialogConfirm),
                ],
            ),
            vec![
                DialogAction::new("one", "Replace Next", Msg::DialogConfirm),
                DialogAction::new("all", "Replace All", Msg::ReplaceAllConfirm),
                DialogAction::new("cancel", "Cancel", Msg::DialogCancel),
            ],
            Some("one"),
            Some(Msg::DialogCancel),
        )),
        DialogMode::GotoLine(val) => Some(View::dialog(
            "dlg_goto",
            "Go to Line",
            View::input("dlg_goto_in", "Line number", val.clone(), Msg::DialogField1, InputOptions::default())
                .on_submit(Msg::DialogConfirm),
            vec![
                DialogAction::new("ok", "Go", Msg::DialogConfirm),
                DialogAction::new("cancel", "Cancel", Msg::DialogCancel),
            ],
            Some("ok"),
            Some(Msg::DialogCancel),
        )),
        DialogMode::Help => Some(View::dialog(
            "dlg_help",
            "Keyboard Shortcuts",
            View::column(
                "help_box",
                SHORTCUT_HINTS
                    .iter()
                    .enumerate()
                    .map(|(i, h)| View::text(format!("help_{}", i), format!("{}: {}", h.key, h.label)))
                    .collect(),
            ),
            vec![DialogAction::new("ok", "Close", Msg::DialogCancel)],
            Some("ok"),
            Some(Msg::DialogCancel),
        )),
    }
}

impl Component for EditorApp {
    type Message = Msg;

    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::TextChanged(text) => {
                if self.doc.bytes_len() <= 64 * 1024 && self.doc.apply_full_text(&text).is_ok() {
                    self.note = String::from("Editing");
                }
            }
            Msg::OpenPrompt => {
                self.dialog = DialogMode::Open(String::from(self.doc.path().unwrap_or("/")));
            }
            Msg::Save => {
                if let Some(p) = self.doc.path().map(String::from) {
                    self.save_current(&p);
                } else {
                    self.dialog = DialogMode::SaveAs(String::new());
                }
            }
            Msg::SaveAsPrompt => {
                self.dialog = DialogMode::SaveAs(String::from(self.doc.path().unwrap_or("")));
            }
            Msg::FindPrompt => {
                self.dialog = DialogMode::Find(self.last_query.clone());
            }
            Msg::FindNext => {
                if !self.last_query.is_empty() {
                    let q = self.last_query.clone();
                    if let Some(r) = self.doc.find_next(&q, false) {
                        let p = self.doc.pos_at(r.start);
                        self.note = format!("Match '{}' at Ln {}, Col {}", q, p.line + 1, p.col + 1);
                    } else {
                        self.note = format!("Not found: {}", q);
                    }
                }
            }
            Msg::ReplacePrompt => {
                self.dialog = DialogMode::Replace {
                    query: self.last_query.clone(),
                    with: String::new(),
                };
            }
            Msg::GotoPrompt => {
                self.dialog = DialogMode::GotoLine(String::new());
            }
            Msg::Undo => {
                self.note = if self.doc.undo() {
                    String::from("Undone")
                } else {
                    String::from("Nothing to undo")
                };
            }
            Msg::Redo => {
                self.note = if self.doc.redo() {
                    String::from("Redone")
                } else {
                    String::from("Nothing to redo")
                };
            }
            Msg::SelectAll => {
                self.doc.select_all();
                self.note = format!("Selected {} bytes", self.doc.selected_text().len());
            }
            Msg::CompileC => {
                self.pending_compile = true;
            }
            Msg::CompileFinished { saved_path, note } => {
                self.pending_compile = false;
                if let Some(p) = saved_path {
                    self.doc.set_path(p);
                    self.doc.mark_saved();
                }
                self.note = note;
            }
            Msg::HelpPrompt => {
                self.dialog = DialogMode::Help;
            }
            Msg::DialogField1(v) => match &mut self.dialog {
                DialogMode::Open(p) | DialogMode::SaveAs(p) | DialogMode::Find(p) | DialogMode::GotoLine(p) => {
                    *p = v;
                }
                DialogMode::Replace { query, .. } => *query = v,
                _ => {}
            },
            Msg::DialogField2(v) => {
                if let DialogMode::Replace { with, .. } = &mut self.dialog {
                    *with = v;
                }
            }
            Msg::DialogConfirm => match core::mem::replace(&mut self.dialog, DialogMode::None) {
                DialogMode::Open(p) => self.open_path(&p),
                DialogMode::SaveAs(p) => self.save_current(&p),
                DialogMode::Find(q) => {
                    self.last_query = q.clone();
                    if let Some(r) = self.doc.find_next(&q, false) {
                        let p = self.doc.pos_at(r.start);
                        self.note = format!("Found '{}' at Ln {}, Col {}", q, p.line + 1, p.col + 1);
                    } else {
                        self.note = format!("Not found: {}", q);
                    }
                }
                DialogMode::Replace { query, with } => {
                    self.last_query = query.clone();
                    match self.doc.replace_next(&query, &with, false) {
                        Ok(true) => self.note = String::from("Replaced occurrence"),
                        Ok(false) => self.note = format!("Not found: {}", query),
                        Err(_) => self.note = String::from("Replace failed"),
                    }
                }
                DialogMode::GotoLine(digits) => {
                    if let Ok(line) = digits.trim().parse::<usize>() {
                        let pos = self.doc.goto_line(line.max(1));
                        self.note = format!("Moved to line {}", pos.line + 1);
                    }
                }
                DialogMode::Help | DialogMode::None => {}
            },
            Msg::ReplaceAllConfirm => {
                if let DialogMode::Replace { query, with } =
                    core::mem::replace(&mut self.dialog, DialogMode::None)
                {
                    self.last_query = query.clone();
                    let n = self.doc.replace_all(&query, &with, false).unwrap_or(0);
                    self.note = format!("Replaced {} occurrence(s)", n);
                }
            }
            Msg::DialogCancel => {
                self.dialog = DialogMode::None;
            }
            Msg::Quit => {
                self.should_quit = true;
            }
        }
    }

    fn view(&self) -> View<Msg> {
        let too_large = self.doc.bytes_len() > 64 * 1024;
        let field_text = sanitize_for_textarea(self.doc.text());
        let body_area = View::text_area(
            "editor_body",
            "Document",
            field_text,
            Msg::TextChanged,
            InputOptions {
                placeholder: String::from("Start typing or File -> Open..."),
                soft_wrap: false,
                read_only: too_large,
                ..Default::default()
            },
        );
        let lang = Language::from_path_and_text(self.doc.path(), self.doc.text());
        let status_note = if too_large {
            format!("{} (read-only >64 KiB)", self.note)
        } else {
            self.note.clone()
        };
        let status = View::status_bar(
            "status",
            "Status",
            format!("[{}] {} | {}", lang.display_name(), self.doc.status_line(), status_note),
        );
        let main = View::column(
            "root_body",
            vec![
                editor_menu(),
                editor_toolbar(self.doc.can_undo(), self.doc.can_redo()),
                body_area,
                status,
            ],
        );

        View::modal("modal_root", main, editor_dialog(&self.dialog))
            .on_key(Key { character: 's', modifiers: KEYMOD_CTRL }, Msg::Save)
            .on_key(Key { character: 'o', modifiers: KEYMOD_CTRL }, Msg::OpenPrompt)
            .on_key(Key { character: 'f', modifiers: KEYMOD_CTRL }, Msg::FindPrompt)
            .on_key(Key { character: 'h', modifiers: KEYMOD_CTRL }, Msg::ReplacePrompt)
            .on_key(Key { character: 'z', modifiers: KEYMOD_CTRL }, Msg::Undo)
            .on_key(Key { character: 'y', modifiers: KEYMOD_CTRL }, Msg::Redo)
            .on_key(Key { character: 'g', modifiers: KEYMOD_CTRL }, Msg::GotoPrompt)
            .on_key(Key { character: 'q', modifiers: KEYMOD_CTRL }, Msg::Quit)
    }
}

fn sanitize_for_textarea(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\t' {
            out.push_str("    ");
        } else if ch == '\n' || !ch.is_control() {
            out.push(ch);
        }
    }
    if out.len() > 64 * 1024 {
        out.truncate(64 * 1024);
        while !out.is_empty() && !out.is_char_boundary(out.len()) {
            out.pop();
        }
    }
    out
}

fn load_initial_doc(path: Option<&str>) -> Result<DocumentBuffer> {
    let Some(p) = path else {
        return Ok(DocumentBuffer::new());
    };
    match fs::read(p) {
        Ok(bytes) => DocumentBuffer::from_bytes(Some(String::from(p)), &bytes)
            .map_err(|_| Error(errno::EINVAL)),
        Err(Error(errno::ENOENT)) => {
            let mut d = DocumentBuffer::new();
            d.set_path(p);
            Ok(d)
        }
        Err(e) => Err(e),
    }
}

fn compile_and_run(doc: &DocumentBuffer) -> (Option<String>, String, Option<Child>) {
    let src_path = doc.path().unwrap_or("/tmp/editor_prog.c").to_owned();
    if let Err(e) = fs::write(&src_path, doc.text().as_bytes()) {
        return (None, format!("Write failed: {}", e.name()), None);
    }
    let out_bin = "/tmp/editor_prog.out";
    let tcc_args = [
        "/system/bin/tcc",
        "-I/system/include",
        "-L/system/lib",
        &src_path,
        "-ldunit_ui",
        "-o",
        out_bin,
    ];
    match Child::spawn_with_env("/system/bin/tcc", &tcc_args, &[]) {
        Ok(mut cc) => match cc.wait() {
            Ok(st) if st.success() => match Child::spawn_with_env(out_bin, &[out_bin], &[]) {
                Ok(child) => (Some(src_path), format!("Compiled & launched {}", out_bin), Some(child)),
                Err(e) => (Some(src_path), format!("Launch failed: {}", e.name()), None),
            },
            Ok(st) => (Some(src_path), format!("TCC exited with {:?}", st.code()), None),
            Err(e) => (Some(src_path), format!("Wait failed: {}", e.name()), None),
        },
        Err(e) => (Some(src_path), format!("TCC unavailable: {}", e.name()), None),
    }
}

fn canvas_rgb_fnv1a64(pixels: &[u32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &p in pixels {
        for byte in [
            ((p >> 16) & 0xff) as u8,
            ((p >> 8) & 0xff) as u8,
            (p & 0xff) as u8,
        ] {
            h ^= byte as u64;
            h = h.wrapping_mul(0x1000_0000_1b3);
        }
    }
    h
}

#[inline(never)]
fn open_editor_window(doc: DocumentBuffer) -> Result<ComponentWindow<EditorApp>> {
    let app = EditorApp::new(doc);
    ComponentWindow::open_themed_with_style(W, H, app, STYLE)
}

#[inline(never)]
fn run_self_test(ui: &mut ComponentWindow<EditorApp>) -> Result<()> {
    let dark_hash = canvas_rgb_fnv1a64(ui.pixels());
    let dark_bg = ui.pixels()[0];
    dunit::println!(
        "[GUI-EDITOR] theme dark hash={:016x} bg={:#010x}",
        dark_hash,
        dark_bg
    )?;

    if let Ok(mut st) = Child::spawn_with_env(
        "/app/setup",
        &[
            "/app/setup",
            "set",
            "desktop.sdk_theme",
            "\"/assets/dwm/sdk/light.dss\"",
        ],
        &[],
    ) {
        let _ = st.wait();
    }
    let _ = ui.reload_theme();
    let light_hash = canvas_rgb_fnv1a64(ui.pixels());
    let light_bg = ui.pixels()[0];
    if light_hash == dark_hash || light_bg == dark_bg {
        return Err(Error(errno::EIO));
    }
    dunit::println!(
        "[GUI-EDITOR] theme light hash={:016x} bg={:#010x}",
        light_hash,
        light_bg
    )?;

    if let Ok(mut st) = Child::spawn_with_env(
        "/app/setup",
        &[
            "/app/setup",
            "set",
            "desktop.sdk_theme",
            "\"/assets/dwm/sdk/dark.dss\"",
        ],
        &[],
    ) {
        let _ = st.wait();
    }
    let _ = ui.reload_theme();

    let mut model = ui.component().doc.clone();
    model
        .insert_str("/* gui_editor verified */\n")
        .map_err(|_| Error(errno::EINVAL))?;
    let out_path = "/tmp/gui_editor_probe.txt";
    fs::write(out_path, model.text().as_bytes())?;
    let reread = fs::read(out_path)?;
    let h1 = dunit_editor::sha256_hex(model.text().as_bytes());
    let h2 = dunit_editor::sha256_hex(&reread);
    let _ = fs::remove_file(out_path);
    if h1 != h2 {
        return Err(Error(errno::EIO));
    }
    ui.update(Msg::TextChanged(sanitize_for_textarea(model.text())))?;
    let _ = ui.next_event(Duration::from_millis(50));
    dunit::println!("[GUI-EDITOR] self-test OK sha256={}", h1)
}

#[dunit::main]
fn main() -> Result<()> {
    let argv = args::args()?;
    let mut path: Option<String> = None;
    let mut self_test = fs::metadata("/m9_run_theme_check").is_ok();
    for arg in argv.iter().skip(1) {
        if arg == "--self-test" {
            self_test = true;
        } else if path.is_none() {
            path = Some(arg.clone());
        }
    }
    if path.is_none() && fs::metadata("/tmp/m9_rock.c").is_ok() {
        path = Some(String::from("/tmp/m9_rock.c"));
    }

    let doc = load_initial_doc(path.as_deref())?;
    let mut ui = open_editor_window(doc)?;
    let _ = dunit::println!("[GUI-EDITOR] window opened");

    if self_test {
        run_self_test(&mut ui)?;
        return ui.close();
    }

    let mut children: Vec<Child> = Vec::new();
    loop {
        children.retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_)) | Err(_)));
        if ui.component().pending_compile {
            let (saved_path, note, spawned) = compile_and_run(&ui.component().doc);
            if let Some(child) = spawned {
                children.push(child);
            }
            ui.update(Msg::CompileFinished { saved_path, note })?;
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
