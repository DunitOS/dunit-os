#![no_std]
#![no_main]

//! Source-preserving desktop editor over retained SDK controls.
//! The application validates and saves documents; SDK owns input and painting.

extern crate alloc;

use dunit::ui::{Attrs, Component, ComponentWindow, InputOptions, Sizing, View};
use dunit::window::WindowEvent;

mod document_editor;
use document_editor::{Editor, Section};

use alloc::string::String;
use alloc::vec::Vec;

use dwm_settings::{Effects, Settings, Theme};

fn log(message: &str) {
    let _ = dunit::println!("{}", message);
}

const W: u32 = 600;
const H: u32 = 600;
const NCOLORS: usize = 17;
const COLOR_LABELS: [&str; NCOLORS] = [
    "Desktop background",
    "Active title bar",
    "Inactive title bar",
    "Active border",
    "Inactive border",
    "Close button",
    "Panel background",
    "Panel text",
    "Accent color",
    "Task button",
    "Active task button",
    "Menu background",
    "Menu selection",
    "Active title text",
    "Inactive title text",
    "Minimize button",
    "Maximize button",
];

const NTOGGLES: usize = 3;
const TOGGLE_LABELS: [&str; NTOGGLES] = ["Background blur", "Gradients", "Animations"];

/// One geometry stepper: which field, the increment, and a sane clamp range.
struct IntSpec {
    label: &'static str,
    step: i32,
    min: i32,
    max: i32,
}
const NINTS: usize = 6;
const INT_SPECS: [IntSpec; NINTS] = [
    IntSpec {
        label: "Panel height",
        step: 2,
        min: 18,
        max: 64,
    },
    IntSpec {
        label: "Title bar height",
        step: 2,
        min: 16,
        max: 48,
    },
    IntSpec {
        label: "Dock width",
        step: 4,
        min: 32,
        max: 96,
    },
    IntSpec {
        label: "Corner radius",
        step: 2,
        min: 0,
        max: 32,
    },
    IntSpec {
        label: "Shadow size",
        step: 1,
        min: 0,
        max: 24,
    },
    IntSpec {
        label: "Animation duration (ms)",
        step: 20,
        min: 0,
        max: 600,
    },
];

/// Preset display modes offered by the resolution stepper. Index 0 is the
/// baseline `0/0` = "keep the boot resolution" (what the desktop does today); the
/// rest are common 16:9 modes the Bochs/DISPI mode-set backend accepts. Cycling
/// `<`/`>` walks this list; the CHOSEN mode is policy that lands in `[display]`
/// (single source of truth) — the compositor applies it on the next session
/// re-entry, and on a fixed GOP framebuffer the request degrades to a no-op.
const NRES: usize = 9;
const RES_PRESETS: [(u32, u32); NRES] = [
    (0, 0),
    (1280, 720),
    (1366, 768),
    (1600, 900),
    (1920, 1080),
    (2560, 1440),
    (2560, 1600),
    (2880, 1800),
    (3072, 1920),
];

// --- Settings field accessors (index -> field), read + mutable variants ------

fn color_ref(t: &mut Theme, i: usize) -> &mut u32 {
    match i {
        0 => &mut t.desktop,
        1 => &mut t.title_focused,
        2 => &mut t.title_unfocused,
        3 => &mut t.border_focused,
        4 => &mut t.border_unfocused,
        5 => &mut t.close,
        6 => &mut t.panel,
        7 => &mut t.panel_text,
        8 => &mut t.launcher,
        9 => &mut t.taskbtn,
        10 => &mut t.taskbtn_focused,
        11 => &mut t.menu,
        12 => &mut t.menu_hover,
        13 => &mut t.title_text_focused,
        14 => &mut t.title_text_unfocused,
        15 => &mut t.minimize,
        _ => &mut t.maximize,
    }
}
fn color_get(t: &Theme, i: usize) -> u32 {
    match i {
        0 => t.desktop,
        1 => t.title_focused,
        2 => t.title_unfocused,
        3 => t.border_focused,
        4 => t.border_unfocused,
        5 => t.close,
        6 => t.panel,
        7 => t.panel_text,
        8 => t.launcher,
        9 => t.taskbtn,
        10 => t.taskbtn_focused,
        11 => t.menu,
        12 => t.menu_hover,
        13 => t.title_text_focused,
        14 => t.title_text_unfocused,
        15 => t.minimize,
        _ => t.maximize,
    }
}

fn toggle_get(fx: &Effects, i: usize) -> bool {
    match i {
        0 => fx.blur,
        1 => fx.gradient,
        _ => fx.anim,
    }
}
fn toggle_flip(fx: &mut Effects, i: usize) {
    match i {
        0 => fx.blur = !fx.blur,
        1 => fx.gradient = !fx.gradient,
        _ => fx.anim = !fx.anim,
    }
}

fn int_ref(s: &mut Settings, i: usize) -> &mut i32 {
    match i {
        0 => &mut s.layout.panel_h,
        1 => &mut s.layout.title_h,
        2 => &mut s.layout.dock_w,
        3 => &mut s.effects.corner_radius,
        4 => &mut s.effects.shadow,
        _ => &mut s.effects.anim_ms,
    }
}
fn int_get(s: &Settings, i: usize) -> i32 {
    match i {
        0 => s.layout.panel_h,
        1 => s.layout.title_h,
        2 => s.layout.dock_w,
        3 => s.effects.corner_radius,
        4 => s.effects.shadow,
        _ => s.effects.anim_ms,
    }
}

/// A click action bound to a screen rectangle.
#[derive(Clone, Copy)]
enum Action {
    SelectPage(usize),
    ColorPrev(usize),
    ColorNext(usize),
    Toggle(usize),
    IntDelta(usize, i32),
    ResPrev,
    ResNext,
    ScriptsToggle,
    ScriptLayout(i32),
    ScriptTimeout(i32),
    ImportTheme(usize),
    ImportDesktop(usize),
    SelectProfile(usize),
    ImportProfile,
    InsertLine,
    NewTable,
    DeleteLine,
    EditorPage(i32),
    SaveDocument,
    RevertDocument,
}

/// Editor model: the working settings plus the palette index chosen per color.
struct App {
    s: Settings,
    color_idx: [usize; NCOLORS],
    palette: Vec<u32>,
    res_idx: usize,
    page: usize,
    status: &'static str,
    document_error: String,
    error: bool,
    script_layouts: Vec<String>,
    scripts_directory: String,
    scripts_listing_status: &'static str,
    themes: Vec<String>,
    themes_listing_status: &'static str,
    editor: Option<Editor>,
    editor_page: usize,
    profiles: Vec<String>,
    profile_path: String,
    reload: bool,
    pending: Option<Action>,
}

impl App {
    fn editor_section(&self) -> Option<Section> {
        match self.page {
            6 => Some(Section::Panels),
            7 => Some(Section::Typography),
            8 => Some(Section::Widgets),
            9 => Some(Section::Bindings),
            10 => Some(Section::Rules),
            11 => Some(Section::Scripts),
            _ => None,
        }
    }
    fn refresh_document(&mut self) {
        match dwm_settings::load_document() {
            Ok(text) => {
                self.editor = Some(Editor::new(text));
                self.editor_page = 0;
            }
            Err(error) => {
                self.editor = None;
                self.status = error.reason;
                self.error = true;
            }
        }
    }
    fn new() -> App {
        let s = dwm_settings::load();
        let loaded_palette = dwm_settings::load_palette(s.desktop.palette.as_str());
        let valid_palette = loaded_palette.is_some();
        let mut palette = loaded_palette.unwrap_or_default();
        for i in 0..NCOLORS {
            let color = color_get(&s.theme, i);
            if !palette.contains(&color) {
                palette.push(color);
            }
        }
        let mut color_idx = [0usize; NCOLORS];
        for (i, slot) in color_idx.iter_mut().enumerate() {
            let cur = color_get(&s.theme, i);
            *slot = palette.iter().position(|&p| p == cur).unwrap_or(0);
        }
        let res_idx = RES_PRESETS
            .iter()
            .position(|&(w, h)| w == s.display.width && h == s.display.height)
            .unwrap_or(0);
        let mut app = App {
            s,
            color_idx,
            palette,
            res_idx,
            page: 0,
            status: if valid_palette {
                "Settings saved to the active desktop profile."
            } else {
                "Cannot read palette: using current theme colors. Set desktop.palette in setup."
            },
            error: !valid_palette,
            document_error: String::new(),
            script_layouts: Vec::new(),
            scripts_directory: String::new(),
            scripts_listing_status: "",
            themes: Vec::new(),
            themes_listing_status: "",
            editor: None,
            editor_page: 0,
            profiles: Vec::new(),
            reload: false,
            pending: None,
            profile_path: String::new(),
        };
        app.refresh_scripts();
        app
    }

    /// Discover scripts in the configured layout's parent directory. Only real
    /// regular files whose full paths round-trip through Settings are offered.
    fn refresh_scripts(&mut self) {
        self.script_layouts.clear();
        self.scripts_directory.clear();
        let path = self.s.scripts.layout.as_str();
        let Some((parent, _)) = path.rsplit_once('/') else {
            self.scripts_listing_status = "Set an absolute scripts.layout path in setup.";
            return;
        };
        let parent = if parent.is_empty() { "/" } else { parent };
        if !path.starts_with('/')
            || parent.split('/').any(|part| part == "..")
            || parent
                .bytes()
                .any(|b| b == b'"' || b == b'\\' || b.is_ascii_control())
        {
            self.scripts_listing_status = "Script folder is invalid; edit scripts.layout in setup.";
            return;
        }
        self.scripts_directory.push_str(parent);
        let entries = match dunit::fs::read_dir(parent) {
            Ok(entries) => entries,
            Err(_) => {
                self.scripts_listing_status = "Cannot read script folder; edit its path in setup.";
                return;
            }
        };
        for entry in &entries {
            let name = entry.name.as_str();
            if entry.file_type != dunit::fs::FileType::File
                || !name.ends_with(".lua")
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                continue;
            }
            let path = alloc::format!("{}/{}", parent.trim_end_matches('/'), name);
            // Empty drafts cannot return a layout. Keep them off the selector;
            // the source editor still accepts arbitrary configured script paths.
            let Ok(source) = dunit::fs::read_bounded(&path, 64 * 1024) else {
                continue;
            };
            if source.iter().all(|byte| byte.is_ascii_whitespace()) {
                continue;
            }
            let mut candidate = self.s;
            dwm_settings::parse_into(
                &alloc::format!("[scripts]\nlayout = \"{}\"\n", path),
                &mut candidate,
            );
            if candidate.scripts.layout.as_str() == path && dwm_settings::roundtrip_ok(&candidate) {
                self.script_layouts.push(path);
            }
        }
        self.script_layouts.sort_unstable();
        self.script_layouts.dedup();
        self.scripts_listing_status = if self.script_layouts.is_empty() {
            "No selectable .lua files in this folder."
        } else {
            "Folder and runner can be changed in terminal setup."
        };
    }

    fn refresh_profiles(&mut self) {
        self.profiles.clear();
        let entries = match dunit::fs::read_dir("/assets/dwm") {
            Ok(entries) => entries,
            Err(_) => {
                self.status =
                    "Cannot read bundled profile directory. Enter an absolute profile path.";
                self.error = true;
                return;
            }
        };
        for entry in &entries {
            let name = entry.name.as_str();
            if entry.file_type == dunit::fs::FileType::File
                && name.ends_with(".toml")
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                let path = alloc::format!("/assets/dwm/{}", name);
                if dwm_settings::load_document_from(&path).is_ok() {
                    self.profiles.push(path);
                }
            }
        }
        self.profiles.sort_unstable();
        self.profiles.dedup();
    }

    fn refresh_themes(&mut self) {
        self.themes.clear();
        let directory = self.s.desktop.themes.as_str();
        if !directory.starts_with('/') || directory.split('/').any(|p| p == "..") {
            self.themes_listing_status = "Invalid theme folder; edit desktop.themes in setup.";
            return;
        }
        let entries = match dunit::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(_) => {
                self.themes_listing_status = "Cannot read theme folder.";
                return;
            }
        };
        for entry in &entries {
            let name = entry.name.as_str();
            if entry.file_type == dunit::fs::FileType::File
                && name.ends_with(".toml")
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                self.themes.push(alloc::format!(
                    "{}/{}",
                    directory.trim_end_matches('/'),
                    name
                ));
            }
        }
        self.themes.sort_unstable();
        self.themes.dedup();
        self.themes_listing_status = if self.themes.is_empty() {
            "No .toml themes in this folder."
        } else {
            "Choose a theme to validate, save and apply it."
        };
    }

    fn refresh_color_choices(&mut self) {
        self.palette =
            dwm_settings::load_palette(self.s.desktop.palette.as_str()).unwrap_or_default();
        for i in 0..NCOLORS {
            let color = color_get(&self.s.theme, i);
            if !self.palette.contains(&color) {
                self.palette.push(color);
            }
            self.color_idx[i] = self.palette.iter().position(|&p| p == color).unwrap_or(0);
        }
        self.res_idx = RES_PRESETS
            .iter()
            .position(|&(w, h)| w == self.s.display.width && h == self.s.display.height)
            .unwrap_or(0);
    }

    fn apply(&mut self, act: Action) {
        match act {
            Action::ImportTheme(_)
            | Action::ImportDesktop(_)
            | Action::ImportProfile
            | Action::SaveDocument => {}
            Action::SelectProfile(index) => {
                if let Some(path) = self.profiles.get(index) {
                    self.profile_path = path.clone();

                    self.status = "Profile selected. Import complete desktop validates before replacing configuration.";
                    self.error = false;
                }
            }
            Action::InsertLine | Action::NewTable => {
                if let Some(section) = self.editor_section() {
                    if let Some(editor) = &mut self.editor {
                        if matches!(act, Action::NewTable) {
                            editor.new_table(section);
                        } else {
                            editor.insert_line(section);
                        }
                        let rows = editor.rows(section);
                        self.editor_page = rows
                            .iter()
                            .position(|i| Some(*i) == editor.selected)
                            .unwrap_or(0)
                            / 12;
                        self.status =
                            "Draft changed. Apply validates the complete desktop document.";
                        self.error = false;
                    }
                }
            }
            Action::DeleteLine => {
                if let Some(editor) = &mut self.editor {
                    editor.remove_line();
                }
            }
            Action::EditorPage(delta) => {
                if let (Some(section), Some(editor)) = (self.editor_section(), &self.editor) {
                    self.editor_page = (self.editor_page as i32 + delta)
                        .clamp(0, editor.rows(section).len().saturating_sub(1) as i32 / 12)
                        as usize;
                }
            }
            Action::RevertDocument => self.refresh_document(),
            Action::SelectPage(page) => {
                self.page = page;

                if let Some(editor) = &mut self.editor {
                    editor.selected = None;
                }

                if page == 4 {
                    self.refresh_scripts();
                }
                if page == 5 {
                    self.refresh_themes();
                }
                if page == 12 {
                    self.refresh_profiles();
                }
                if self.editor_section().is_some() && self.editor.is_none() {
                    self.refresh_document();
                }
            }
            Action::ScriptsToggle => self.s.scripts.enabled = !self.s.scripts.enabled,
            Action::ScriptTimeout(delta) => {
                self.s.scripts.timeout_ms =
                    (self.s.scripts.timeout_ms as i32 + delta).clamp(50, 5000) as u32;
            }
            Action::ScriptLayout(direction) => {
                if !self.script_layouts.is_empty() {
                    let len = self.script_layouts.len();
                    let current = self
                        .script_layouts
                        .iter()
                        .position(|path| path == self.s.scripts.layout.as_str());
                    let index = match (current, direction < 0) {
                        (Some(i), true) => (i + len - 1) % len,
                        (Some(i), false) => (i + 1) % len,
                        (None, true) => len - 1,
                        (None, false) => 0,
                    };
                    dwm_settings::parse_into(
                        &alloc::format!("[scripts]\nlayout = \"{}\"\n", self.script_layouts[index]),
                        &mut self.s,
                    );
                }
            }
            Action::ColorPrev(i) => {
                self.color_idx[i] =
                    (self.color_idx[i] + self.palette.len() - 1) % self.palette.len();
                *color_ref(&mut self.s.theme, i) = self.palette[self.color_idx[i]];
            }
            Action::ColorNext(i) => {
                self.color_idx[i] = (self.color_idx[i] + 1) % self.palette.len();
                *color_ref(&mut self.s.theme, i) = self.palette[self.color_idx[i]];
            }
            Action::Toggle(i) => toggle_flip(&mut self.s.effects, i),
            Action::IntDelta(i, d) => {
                let (min, max) = (INT_SPECS[i].min, INT_SPECS[i].max);
                let r = int_ref(&mut self.s, i);
                *r = (*r + d).clamp(min, max);
            }
            Action::ResPrev => {
                self.res_idx = (self.res_idx + NRES - 1) % NRES;
                let (w, h) = RES_PRESETS[self.res_idx];
                self.s.display.width = w;
                self.s.display.height = h;
            }
            Action::ResNext => {
                self.res_idx = (self.res_idx + 1) % NRES;
                let (w, h) = RES_PRESETS[self.res_idx];
                self.s.display.width = w;
                self.s.display.height = h;
            }
        }
    }
}

impl App {
    #[inline(never)]
    fn activate(&mut self, a: Action) {
        self.document_error.clear();
        if matches!(
            a,
            Action::SaveDocument | Action::ImportDesktop(_) | Action::ImportProfile
        ) {
            let import_path = match a {
                Action::ImportDesktop(index) => {
                    Some(self.themes.get(index).cloned().unwrap_or_default())
                }
                Action::ImportProfile => Some(self.profile_path.clone()),
                _ => None,
            };
            let result = if let Some(path) = &import_path {
                dwm_settings::load_document_from(path)
                            .and_then(|text| {
                                let config = dwm_settings::validate_installed_document(&text)?;
                                if !config.valid || config.apps.apps.is_empty() {
                                    return Err(dwm_settings::ConfigError { line: 0, field: String::new(),
                                        reason: "Full desktop import requires an application registry. Use theme overlay instead." });
                                }
                                if matches!(a,Action::ImportProfile){dwm_settings::save_user_document(&text)}else{dwm_settings::save_document(&text)}
                            })
            } else if let Some(editor) = &self.editor {
                dwm_settings::load_document().and_then(|current| {
                    if current != editor.original {
                        return Err(dwm_settings::ConfigError {
                            line: 0,
                            field: String::new(),
                            reason: "Desktop changed elsewhere. Revert before applying this draft.",
                        });
                    }
                    dwm_settings::save_document(&editor.text())
                })
            } else {
                return;
            };
            match result {
                Err(error) => {
                    self.status = error.reason;
                    self.error = true;
                    self.document_error = alloc::format!(
                        "{} line {} {}: {}",
                        import_path.as_deref().unwrap_or("saved desktop"),
                        error.line,
                        error.field,
                        error.reason
                    );
                    log(&alloc::format!(
                        "gui_settings: document rejected file={} line={} field={} reason={}",
                        import_path.as_deref().unwrap_or("saved desktop"),
                        error.line,
                        error.field,
                        error.reason
                    ));
                }
                Ok(()) => {
                    self.refresh_document();
                    self.s = dwm_settings::load();
                    self.refresh_color_choices();
                    self.reload = true;
                    self.error = false;
                    self.status = "Document validated and saved; desktop reload requested.";
                    log("gui_settings: complete document saved");
                    if let Some(path) = &import_path {
                        log(&alloc::format!(
                            "gui_settings: desktop profile saved {}",
                            path
                        ));
                    }
                }
            }

            return;
        }
        if matches!(
            a,
            Action::SelectPage(_)
                | Action::InsertLine
                | Action::NewTable
                | Action::DeleteLine
                | Action::EditorPage(_)
                | Action::RevertDocument
                | Action::SelectProfile(_)
        ) {
            self.apply(a);

            return;
        }
        let previous = self.s;
        let previous_colors = self.color_idx;
        let previous_resolution = self.res_idx;
        let mut explicit_overlay = None;
        if let Action::ImportTheme(index) = a {
            let candidate = self
                .themes
                .get(index)
                .ok_or("Theme selection is no longer available.")
                .and_then(|path| dwm_settings::load_overlay_patch(&self.s, path));
            match candidate {
                Ok((candidate, source)) => {
                    self.s = candidate;
                    explicit_overlay = Some(source);
                }
                Err(error) => {
                    self.status = error;
                    self.error = true;
                    log(&alloc::format!(
                        "gui_settings: theme rejected; previous settings retained: {}",
                        error
                    ));

                    return;
                }
            }
        } else {
            self.apply(a);
        }
        // Persist to the live config and ask the compositor to reload,
        // so the desktop reflects the new theme/geometry immediately.
        let saved = if let Some(source) = &explicit_overlay {
            dwm_settings::save_overlay(source)
        } else {
            dwm_settings::save_changes(&previous, &self.s)
        };
        if !saved {
            self.s = previous;
            self.color_idx = previous_colors;
            self.res_idx = previous_resolution;
            self.status = "Save failed. Previous settings restored.";
            self.error = true;
            log("gui_settings: save failed; previous settings retained");

            return;
        }
        if self.editor.as_ref().map(|e| !e.dirty).unwrap_or(false) {
            self.refresh_document();
        }
        if let Action::ImportTheme(index) = a {
            self.refresh_color_choices();
            log(&alloc::format!(
                "gui_settings: theme saved {}",
                self.themes[index]
            ));
        }
        self.reload = true;
        self.status = "Saved and desktop reload requested (session RAM).";
        self.error = false;
    }
}

const PAGES: [&str; 13] = [
    "Appearance",
    "Animations",
    "Desktop geometry",
    "Display",
    "Lua Scripts",
    "Themes",
    "Panels",
    "Typography",
    "Widgets",
    "Keybindings",
    "Window Rules",
    "Lua event hook",
    "Desktop profiles",
];
#[derive(Clone)]
enum Message {
    Action(Action),
    Apply(Action),
    Line(usize, String),
    SelectLine(usize),
    Profile(String),
    Open(String),
    Reloaded(bool),
}
fn action(key: impl Into<String>, label: impl Into<String>, a: Action) -> View<Message> {
    View::button(key, label, Message::Action(a))
}
#[derive(Clone)]
enum LineMessage {
    Edit(String),
    Select,
}
fn select_line(event: dunit::ui::UiEvent) -> dunit::ui::EventResponse<LineMessage> {
    if matches!(
        event.kind,
        dunit::ui::UiEventKind::Pointer {
            event: dunit::ui::PointerEvent::Down,
            ..
        } | dunit::ui::UiEventKind::NativeKey { .. }
    ) {
        dunit::ui::EventResponse::message(LineMessage::Select)
    } else {
        Default::default()
    }
}
impl Component for App {
    type Message = Message;
    fn update(&mut self, message: Message) {
        match message {
            Message::Action(a) => self.pending = Some(a),
            Message::Apply(a) => {
                self.pending = None;
                self.activate(a);
            }
            Message::SelectLine(index) => {
                if let Some(editor) = &mut self.editor {
                    editor.select(index);
                }
            }
            Message::Line(index, text) => {
                if let Some(editor) = &mut self.editor {
                    editor.replace_line(index, text);
                }
                self.document_error.clear();
                self.error = false;
                self.status =
                    "Draft only. Apply validates and saves; Revert reloads the saved document.";
            }
            Message::Profile(text) => {
                self.profile_path = text;
                self.document_error.clear();
                self.error = false;
            }
            Message::Open(text) => {
                let baseline = dwm_settings::load_document();
                match baseline {
                    Ok(baseline) => {
                        let mut editor = Editor::new(text);
                        editor.original = baseline;
                        editor.dirty = true;
                        self.editor = Some(editor);
                        self.page = 6;
                        self.editor_page = 0;
                        self.error = false;
                        self.document_error.clear();
                        self.status =
                            "Opened a document draft. Apply explicitly to validate and save.";
                    }
                    Err(error) => {
                        self.status = error.reason;
                        self.error = true;
                    }
                }
            }
            Message::Reloaded(ok) => {
                self.reload = false;
                if !ok {
                    self.error = true;
                    self.status = "Saved; desktop reload could not be requested.";
                }
            }
        }
    }
    fn view(&self) -> View<Message> {
        let nav = PAGES
            .iter()
            .enumerate()
            .map(|(i, label)| {
                action(alloc::format!("page{}", i), *label, Action::SelectPage(i)).class(
                    if self.page == i {
                        "selected"
                    } else {
                        "navigation"
                    },
                )
            })
            .collect();
        let mut rows = alloc::vec![
            View::text("heading", PAGES[self.page]).class("heading"),
            View::text(
                "hint",
                if self.editor_section().is_some() {
                    "Edit a draft, then Apply to validate and reload."
                } else {
                    "Changes save and apply to the current desktop."
                }
            )
        ];
        if self.page == 0 {
            for (i, label) in COLOR_LABELS.iter().enumerate() {
                rows.push(
                    View::column(
                        alloc::format!("color{}", i),
                        alloc::vec![
                            View::text(alloc::format!("colorlabel{}", i), *label),
                            View::row(
                                alloc::format!("colorrow{}", i),
                                alloc::vec![
                                    action(alloc::format!("prev{}", i), "<", Action::ColorPrev(i)),
                                    View::text(
                                        alloc::format!("colorvalue{}", i),
                                        alloc::format!("#{:08X}", color_get(&self.s.theme, i))
                                    ),
                                    action(alloc::format!("next{}", i), ">", Action::ColorNext(i))
                                ]
                            )
                        ],
                    )
                    .class("field"),
                );
            }
        }
        if self.page == 1 {
            for (i, label) in TOGGLE_LABELS.iter().enumerate() {
                rows.push(View::switch(
                    alloc::format!("effect{}", i),
                    *label,
                    toggle_get(&self.s.effects, i),
                    Message::Action(Action::Toggle(i)),
                ));
            }
        }
        for (i, spec) in INT_SPECS.iter().enumerate() {
            if (self.page == 1 && i >= 3) || (self.page == 2 && i < 3) {
                rows.push(
                    View::column(
                        alloc::format!("geometry{}", i),
                        alloc::vec![
                            View::text(alloc::format!("geometrylabel{}", i), spec.label),
                            View::row(
                                alloc::format!("geometryrow{}", i),
                                alloc::vec![
                                    action(
                                        alloc::format!("less{}", i),
                                        "-",
                                        Action::IntDelta(i, -spec.step)
                                    ),
                                    View::text(
                                        alloc::format!("number{}", i),
                                        alloc::format!("{}", int_get(&self.s, i))
                                    ),
                                    action(
                                        alloc::format!("more{}", i),
                                        "+",
                                        Action::IntDelta(i, spec.step)
                                    )
                                ]
                            )
                        ],
                    )
                    .class("field"),
                );
            }
        }
        if self.page == 3 {
            let (w, h) = RES_PRESETS[self.res_idx];
            rows.push(View::text("resolutionlabel", "Desktop resolution"));
            rows.push(View::row(
                "resolution",
                alloc::vec![
                    action("resprev", "<", Action::ResPrev),
                    action("resnext", ">", Action::ResNext),
                    View::text(
                        "resolutionvalue",
                        if w == 0 || h == 0 {
                            String::from("Keep boot mode")
                        } else {
                            alloc::format!("{}x{}", w, h)
                        }
                    )
                ],
            ));
            rows.push(View::text(
                "resolutionhint",
                "The desktop rebuilds buffers after a mode change.",
            ));
        }
        if self.page == 4 {
            rows.push(View::switch(
                "scriptsenabled",
                "Lua window policy",
                self.s.scripts.enabled,
                Message::Action(Action::ScriptsToggle),
            ));
            rows.push(View::text("layoutlabel", "Layout script"));
            rows.push(View::row(
                "layout",
                alloc::vec![
                    action("layoutprev", "<", Action::ScriptLayout(-1)),
                    action("layoutnext", ">", Action::ScriptLayout(1)),
                    View::text(
                        "layoutvalue",
                        self.s
                            .scripts
                            .layout
                            .as_str()
                            .rsplit('/')
                            .next()
                            .unwrap_or("")
                    )
                ],
            ));
            rows.push(View::text(
                "scriptsdirectory",
                self.scripts_directory.clone(),
            ));
            rows.push(View::text("scriptsstatus", self.scripts_listing_status));
            rows.push(View::text("timeoutlabel", "Runner timeout (ms)"));
            rows.push(View::row(
                "timeout",
                alloc::vec![
                    action("timeoutless", "-", Action::ScriptTimeout(-50)),
                    View::text(
                        "timeoutvalue",
                        alloc::format!("{}", self.s.scripts.timeout_ms)
                    ),
                    action("timeoutmore", "+", Action::ScriptTimeout(50))
                ],
            ));
            rows.push(View::text("runnerlabel", "Runner (read-only)"));
            rows.push(View::text("runner", self.s.scripts.runner.as_str()));
        }
        if self.page == 5 {
            rows.push(View::text(
                "themesdirectory",
                self.s.desktop.themes.as_str(),
            ));
            rows.push(View::text("themesstatus", self.themes_listing_status));
            for (i, path) in self.themes.iter().enumerate() {
                rows.push(View::row(
                    alloc::format!("theme{}", i),
                    alloc::vec![
                        action(
                            alloc::format!("overlay{}", i),
                            path.rsplit('/').next().unwrap_or(path),
                            Action::ImportTheme(i)
                        ),
                        action(
                            alloc::format!("import{}", i),
                            "Full desktop",
                            Action::ImportDesktop(i)
                        )
                    ],
                ));
            }
        }
        if self.page == 12 {
            rows.push(View::text(
                "profilehint",
                "Complete profiles replace panels, widgets, registry, rules and settings.",
            ));
            rows.push(View::input(
                "profilepath",
                "Absolute profile path",
                self.profile_path.clone(),
                Message::Profile,
                InputOptions::default(),
            ));
            rows.push(action(
                "importprofile",
                "Import complete desktop",
                Action::ImportProfile,
            ));
            for (i, path) in self.profiles.iter().enumerate() {
                rows.push(action(
                    alloc::format!("profile{}", i),
                    path.rsplit('/').next().unwrap_or(path),
                    Action::SelectProfile(i),
                ));
            }
        }
        if let Some(section) = self.editor_section() {
            let mut buttons = alloc::vec![
                action("apply", "Apply", Action::SaveDocument),
                action("revert", "Revert", Action::RevertDocument),
                action("insert", "+ line", Action::InsertLine),
                action("delete", "- line", Action::DeleteLine)
            ];
            if !matches!(section, Section::Typography | Section::Scripts) {
                buttons.push(action("table", "+ table", Action::NewTable));
            }
            rows.push(View::row("documentactions", buttons));
            if let Some(editor) = &self.editor {
                let indices = editor.rows(section);
                let pages = indices.len().saturating_sub(1) / 12 + 1;
                rows.push(View::row(
                    "pagination",
                    alloc::vec![
                        action("pageprev", "<", Action::EditorPage(-1)),
                        View::text(
                            "pagenumber",
                            alloc::format!(
                                "Page {}/{} - {}",
                                self.editor_page.min(pages - 1) + 1,
                                pages,
                                if editor.dirty {
                                    "unsaved draft"
                                } else {
                                    "saved document"
                                }
                            )
                        ),
                        action("pagenext", ">", Action::EditorPage(1))
                    ],
                ));
                for &index in indices
                    .iter()
                    .skip(self.editor_page.min(pages - 1) * 12)
                    .take(12)
                {
                    rows.push(
                        View::input(
                            alloc::format!("line{}", index),
                            alloc::format!("Line {}", index + 1),
                            editor.lines[index].clone(),
                            LineMessage::Edit,
                            InputOptions {
                                max_chars: document_editor::MAX_BYTES,
                                ..Default::default()
                            },
                        )
                        .on_capture(select_line)
                        .map_messages(move |message| match message {
                            LineMessage::Edit(text) => Message::Line(index, text),
                            LineMessage::Select => Message::SelectLine(index),
                        }),
                    );
                }
                if indices.is_empty() {
                    rows.push(View::text(
                        "empty",
                        "No tables configured. Add a table, then add its fields as lines.",
                    ));
                }
            }
        }
        let content = View::scroll(
            alloc::format!("content{}", self.page),
            "Settings content",
            alloc::vec![View::column("form", rows)],
        )
        .scroll_axes(dunit::ui::ScrollAxes::Both)
        .layout(Attrs {
            width: Sizing::Grow(1.),
            height: Sizing::Grow(1.),
            ..Default::default()
        });
        View::column(
            "settings",
            alloc::vec![
                View::row(
                    "body",
                    alloc::vec![
                        View::scroll(
                            "navigation",
                            "Settings pages",
                            alloc::vec![View::column("pages", nav)]
                        )
                        .layout(Attrs {
                            width: Sizing::Px(154.),
                            height: Sizing::Grow(1.),
                            ..Default::default()
                        }),
                        content
                    ]
                ),
                View::text(
                    "status",
                    if self.document_error.is_empty() {
                        self.status
                    } else {
                        &self.document_error
                    }
                )
                .class(if self.error { "error" } else { "status" })
            ],
        )
    }
}
// Release large configuration/constructor temporaries before the event loop.
#[inline(never)]
fn open_window() -> dunit::Result<ComponentWindow<App>> {
    let mut app = App::new();
    if dwm_settings::load_config().apps.self_test {
        let previous = app.s;
        let previous_resolution = app.res_idx;
        for _ in 0..NRES {
            if RES_PRESETS[app.res_idx] == (1600, 900) {
                break;
            }
            app.apply(Action::ResNext);
        }
        if dwm_settings::save_changes(&previous, &app.s) {
            app.reload = true;
            let (w, h) = RES_PRESETS[app.res_idx];
            dunit::println!(
                "gui_settings: res picker self-test picked {}x{} (saved, reload pinged)",
                w,
                h
            )?;
        } else {
            app.s = previous;
            app.res_idx = previous_resolution;
            app.status = "Save failed. Previous settings restored.";
            app.error = true;
            dunit::println!("gui_settings: FAIL resolution picker save")?;
        }
    }
    let mut ui=ComponentWindow::open_themed_with_style(W,H,app,"Column#settings { height: fill; gap: 0; } Row#body { height: fill; gap: 0; } Column#pages { padding: 8; gap: 4; } Button.navigation { height: 30; font-size: 12; padding: 4; } Column#form { padding: 16; gap: 12; min-width: 420; } Text#heading { font-size: 23; } Text#hint { font-size: 12; } Column.field { gap: 6; } Row { gap: 8; } Button { min-width: 26; } Text#status { height: 52; padding: 8; font-size: 11; } Button.selected { height: 30; font-size: 12; padding: 4; background: $accent; } Text.error { color: #F38BA8; } Input { height: 30; font-size: 13; }")?;
    ui.transfers()?
        .register_opener(dunit::transfer::Offer {
            formats: alloc::vec![dunit::transfer::Format::Files],
            actions: 1,
            preferred: dunit::transfer::Action::Copy,
        })
        .map_err(|e| dunit::Error(e.0))?;
    dunit::println!("gui_settings: surface presented OK")?;
    Ok(ui)
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let mut ui = open_window()?;
    loop {
        let mut imported_theme = false;
        if let Some(action) = ui.component().pending {
            ui.update(Message::Apply(action))?;
            imported_theme = matches!(action, Action::ImportTheme(_)) && !ui.component().error;
            if let Action::SelectPage(page) = action {
                dunit::println!("gui_settings: page {}", page)?;
            }
        }
        if ui.component().reload {
            let ok = ui.reload_desktop().is_ok();
            ui.update(Message::Reloaded(ok))?;
            // Publish our imported profile before acknowledging it. The periodic
            // watcher remains responsible for changes made by other clients.
            if ok && imported_theme {
                match ui.reload_theme() {
                    Ok(_) => dunit::println!(
                        "gui_settings: SDK theme applied {}",
                        ui.component().s.desktop.sdk_theme.as_str()
                    )?,
                    Err(error) => dunit::println!("gui_settings: SDK theme rejected {}", error)?,
                }
            }
        }
        match ui.next_event(dunit::time::Duration::from_millis(40))? {
            Some(WindowEvent::CloseRequested) => break,
            Some(WindowEvent::Resized { .. }) => {
                dunit::println!("gui_settings: reconfigured OK")?;
            }
            Some(WindowEvent::Input(input)) if input.kind == dunit::window::IN_SCROLL => {
                let key = alloc::format!("content{}", ui.component().page);
                let x = ui.scroll_offset_x(&key).unwrap_or(0.) as i32;
                let y = ui.scroll_offset(&key).unwrap_or(0.) as i32;
                dunit::println!("gui_settings: scroll {},{}", x, y)?;
            }
            Some(WindowEvent::Transfer(event))
                if event.message.kind == dunit::transfer::Kind::OpenData =>
            {
                let result = (|| {
                    let payload = event.payload.ok_or(dunit::Error(dunit::errno::EINVAL))?;
                    if payload.files.len() != 1 {
                        return Err(dunit::Error(dunit::errno::EOPNOTSUPP));
                    }
                    let text = String::from_utf8(
                        payload.files[0]
                            .read_all(document_editor::MAX_BYTES)
                            .map_err(|e| dunit::Error(e.0))?,
                    )
                    .map_err(|_| dunit::Error(dunit::errno::EINVAL))?;
                    dwm_settings::validate_installed_document(&text)
                        .map_err(|_| dunit::Error(dunit::errno::EINVAL))?;
                    ui.update(Message::Open(text))?;
                    if ui.component().error {
                        Err(dunit::Error(dunit::errno::EIO))
                    } else {
                        Ok(())
                    }
                })();
                ui.transfers()?
                    .complete(
                        event.message.transfer,
                        if result.is_ok() {
                            dunit::transfer::Status::Ok
                        } else {
                            dunit::transfer::Status::Failed
                        },
                    )
                    .map_err(|e| dunit::Error(e.0))?;
            }
            _ => {}
        }
    }
    ui.close()
}
