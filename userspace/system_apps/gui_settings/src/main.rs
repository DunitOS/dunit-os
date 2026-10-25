#![no_std]
#![no_main]

//! DWM settings client for the M4 userspace DWM (Stack B), slice C.
//!
//! A gui-v1 client (same capability + wire path as `gui_files`) that edits the
//! live compositor config. It loads `dwm_settings::Settings`, paints clickable
//! color swatches, effect toggles and geometry steppers straight onto its shared
//! surface with the M4 text runtime, and on every change writes the config back
//! (`dwm_settings::save`) and pings the compositor to reload — the GUI<->TOML
//! round-trip the concept calls for. RAM-only this session (disk is DunitFS/M5).

use core::panic::PanicInfo;

extern crate alloc;
use gui_protocol_v1::control::{IN_DOWN, IN_KEY, IN_SCROLL, IN_QUIT};
mod document_editor;
use document_editor::{Editor, Section};

use alloc::string::String;
use alloc::vec::Vec;

use gui_protocol_v1::wire::{Request, FEATURE_ARGB8888};

use dunit_render::Surface;
use dunit_style::value::Color;
use dunit_text::Font;

use dwm_settings::{Effects, Settings, Theme};

const SURFACE: u64 = 1;
const BUFFER: u64 = 2;
const W: u32 = 600;
const H: u32 = 600;
const FOOTER_H: u32 = 72;
const FMT_XRGB8888: u32 = 1;

/// Load the configured TTF (`[desktop] font`) from the VFS, falling back to the
/// shipped /assets default font on any error — the desktop font is a live config knob.
fn load_font() -> Result<Font, ()> {
    let cfg = dwm_settings::load();
    if let Some(bytes) = libdunit::read_binary(cfg.desktop.font.as_str(), 4 * 1024 * 1024) {
        if let Ok(f) = Font::parse(bytes) {
            return Ok(f);
        }
    }
    libdunit::read_binary(dwm_settings::Desktop::baseline().font.as_str(), 4 * 1024 * 1024)
        .ok_or(())
        .and_then(|b| Font::parse(b).map_err(|_| ()))
}

// UI geometry (client-local px).
const ROW_H: i32 = 24;
const Y0: i32 = 100;


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
const RES_PRESETS: [(u32, u32); NRES] =
    [(0, 0), (1280, 720), (1366, 768), (1600, 900), (1920, 1080),
     (2560, 1440), (2560, 1600), (2880, 1800), (3072, 1920)];

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
    Scroll(i32, i32),
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
    FocusProfilePath,
    ImportProfile,
    EditLine(usize),
    InsertLine,
    NewTable,
    DeleteLine,
    EditorPage(i32),
    SaveDocument,
    RevertDocument,
}

/// A clickable region and the action it fires.
struct Hotspot {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    act: Action,
}
impl Hotspot {
    fn hit(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Editor model: the working settings plus the palette index chosen per color.
struct App {
    s: Settings,
    color_idx: [usize; NCOLORS],
    palette: Vec<u32>,
    res_idx: usize,
    page: usize,
    width: u32,
    height: u32,
    scroll_x: i32,
    scroll_y: i32,
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
    profile_path: Editor,
    profile_focused: bool,
}


impl App {
    fn editor_section(&self) -> Option<Section> {
        match self.page {
            6 => Some(Section::Panels), 7 => Some(Section::Typography),
            8 => Some(Section::Widgets), 9 => Some(Section::Bindings),
            10 => Some(Section::Rules), 11 => Some(Section::Scripts), _ => None,
        }
    }
    fn refresh_document(&mut self) {
        match dwm_settings::load_document() {
            Ok(text) => { self.editor = Some(Editor::new(text)); self.editor_page = 0; },
            Err(error) => { self.editor = None; self.status = error.reason; self.error = true; },
        }
    }
    fn content_height(&self) -> u32 {
        if self.editor_section().is_some() { return 720; }
        if self.page == 0 {
            (Y0 + NCOLORS as i32 * (2 * ROW_H + 8) + 24) as u32
        } else if self.page == 12 {
            (Y0 + 160 + self.profiles.len() as i32 * 36).max(H as i32) as u32
        } else if self.page == 5 {
            (Y0 + 96 + self.themes.len() as i32 * 40).max(H as i32) as u32
        } else { H }
    }

    fn new() -> App {
        let s = dwm_settings::load();
        let loaded_palette = dwm_settings::load_palette(s.desktop.palette.as_str());
        let valid_palette = loaded_palette.is_some();
        let mut palette = loaded_palette.unwrap_or_default();
        for i in 0..NCOLORS {
            let color = color_get(&s.theme, i);
            if !palette.contains(&color) { palette.push(color); }
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
            width: W,
            height: H,
            scroll_x: 0,
            scroll_y: 0,
            status: if valid_palette { "Settings saved to the active desktop profile." }
                else { "Cannot read palette: using current theme colors. Set desktop.palette in setup." },
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
            profile_path: Editor::new(String::new()),
            profile_focused: false,
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
        if !path.starts_with('/') || parent.split('/').any(|part| part == "..")
            || parent.bytes().any(|b| b == b'"' || b == b'\\' || b.is_ascii_control()) {
            self.scripts_listing_status = "Script folder is invalid; edit scripts.layout in setup.";
            return;
        }
        self.scripts_directory.push_str(parent);
        let mut entries = alloc::vec![libdunit::DirEntry::empty(); 1024];
        let count = libdunit::readdir(parent, &mut entries);
        if count < 0 {
            self.scripts_listing_status = "Cannot read script folder; edit its path in setup.";
            return;
        }
        for entry in entries.iter().take((count as usize).min(entries.len())) {
            let name = entry.name();
            if entry.file_type != libdunit::FILE_TYPE_FILE || !name.ends_with(".lua")
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')) {
                continue;
            }
            let path = alloc::format!("{}/{}", parent.trim_end_matches('/'), name);
            // Empty drafts cannot return a layout. Keep them off the selector;
            // the source editor still accepts arbitrary configured script paths.
            let Some(source) = libdunit::read_binary(&path, 64 * 1024) else { continue; };
            if source.iter().all(|byte| byte.is_ascii_whitespace()) { continue; }
            let mut candidate = self.s;
            dwm_settings::parse_into(&alloc::format!("[scripts]\nlayout = \"{}\"\n", path), &mut candidate);
            if candidate.scripts.layout.as_str() == path && dwm_settings::roundtrip_ok(&candidate) {
                self.script_layouts.push(path);
            }
        }
        self.script_layouts.sort_unstable();
        self.script_layouts.dedup();
        self.scripts_listing_status = if self.script_layouts.is_empty() {
            "No selectable .lua files in this folder."
        } else { "Folder and runner can be changed in terminal setup." };
    }

    fn refresh_profiles(&mut self) {
        self.profiles.clear();
        let mut entries = alloc::vec![libdunit::DirEntry::empty(); 1024];
        let count = libdunit::readdir("/assets/dwm", &mut entries);
        if count < 0 {
            self.status = "Cannot read bundled profile directory. Enter an absolute profile path.";
            self.error = true;
            return;
        }
        for entry in entries.iter().take((count as usize).min(entries.len())) {
            let name = entry.name();
            if entry.file_type == libdunit::FILE_TYPE_FILE && name.ends_with(".toml")
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')) {
                let path = alloc::format!("/assets/dwm/{}", name);
                if dwm_settings::load_document_from(&path).is_ok() { self.profiles.push(path); }
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
        let mut entries = alloc::vec![libdunit::DirEntry::empty(); 1024];
        let count = libdunit::readdir(directory, &mut entries);
        if count < 0 {
            self.themes_listing_status = "Cannot read theme folder.";
            return;
        }
        for entry in entries.iter().take((count as usize).min(entries.len())) {
            let name = entry.name();
            if entry.file_type == libdunit::FILE_TYPE_FILE && name.ends_with(".toml")
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')) {
                self.themes.push(alloc::format!("{}/{}", directory.trim_end_matches('/'), name));
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
        self.palette = dwm_settings::load_palette(self.s.desktop.palette.as_str()).unwrap_or_default();
        for i in 0..NCOLORS {
            let color = color_get(&self.s.theme, i);
            if !self.palette.contains(&color) { self.palette.push(color); }
            self.color_idx[i] = self.palette.iter().position(|&p| p == color).unwrap_or(0);
        }
        self.res_idx = RES_PRESETS.iter()
            .position(|&(w, h)| w == self.s.display.width && h == self.s.display.height)
            .unwrap_or(0);
    }

    fn apply(&mut self, act: Action) {
        match act {
            Action::ImportTheme(_) | Action::ImportDesktop(_) | Action::ImportProfile | Action::SaveDocument => {},
            Action::SelectProfile(index) => {
                if let Some(path) = self.profiles.get(index) {
                    self.profile_path = Editor::new(path.clone()); self.profile_path.select(0);
                    self.profile_focused = true;
                    self.status = "Profile selected. Import complete desktop validates before replacing configuration.";
                    self.error = false;
                }
            },
            Action::FocusProfilePath => {
                if self.profile_path.lines.is_empty() { self.profile_path.lines.push(String::new()); }
                self.profile_path.select(0); self.profile_focused = true;
            },
            Action::EditLine(index) => if let Some(editor) = &mut self.editor { editor.select(index); },
            Action::InsertLine | Action::NewTable => {
                if let Some(section) = self.editor_section() {
                    if let Some(editor) = &mut self.editor {
                        if matches!(act, Action::NewTable) { editor.new_table(section); }
                        else { editor.insert_line(section); }
                        let rows = editor.rows(section);
                        self.editor_page = rows.iter().position(|i| Some(*i) == editor.selected).unwrap_or(0) / 12;
                        self.status = "Draft changed. Apply validates the complete desktop document.";
                        self.error = false;
                    }
                }
            },
            Action::DeleteLine => if let Some(editor) = &mut self.editor { editor.remove_line(); },
            Action::EditorPage(delta) => {
                if let (Some(section), Some(editor)) = (self.editor_section(), &self.editor) {
                    self.editor_page = (self.editor_page as i32 + delta).clamp(0,
                        editor.rows(section).len().saturating_sub(1) as i32 / 12) as usize;
                }
            },
            Action::RevertDocument => self.refresh_document(),
            Action::SelectPage(page) => {
                self.page = page;
                self.profile_focused = false;
                if let Some(editor) = &mut self.editor { editor.selected = None; }
                self.scroll_x = 0;
                self.scroll_y = 0;
                if page == 4 { self.refresh_scripts(); }
                if page == 5 { self.refresh_themes(); }
                if page == 12 { self.refresh_profiles(); }
                if self.editor_section().is_some() && self.editor.is_none() { self.refresh_document(); }
            }
            Action::ScriptsToggle => self.s.scripts.enabled = !self.s.scripts.enabled,
            Action::ScriptTimeout(delta) => {
                self.s.scripts.timeout_ms = (self.s.scripts.timeout_ms as i32 + delta).clamp(50, 5000) as u32;
            }
            Action::ScriptLayout(direction) => {
                if !self.script_layouts.is_empty() {
                    let len = self.script_layouts.len();
                    let current = self.script_layouts.iter().position(|path| path == self.s.scripts.layout.as_str());
                    let index = match (current, direction < 0) {
                        (Some(i), true) => (i + len - 1) % len,
                        (Some(i), false) => (i + 1) % len,
                        (None, true) => len - 1,
                        (None, false) => 0,
                    };
                    dwm_settings::parse_into(&alloc::format!("[scripts]\nlayout = \"{}\"\n", self.script_layouts[index]), &mut self.s);
                }
            }
            Action::Scroll(x, y) => {
                self.scroll_x = self
                    .scroll_x
                    .saturating_add(x)
                    .clamp(0, (if self.editor_section().is_some() { 1200u32 } else { W }).saturating_sub(self.width) as i32);
                self.scroll_y = self.scroll_y.saturating_add(y).clamp(
                    0,
                    self.content_height().saturating_sub(self.height.saturating_sub(FOOTER_H)) as i32,
                );
            }
            Action::ColorPrev(i) => {
                self.color_idx[i] = (self.color_idx[i] + self.palette.len() - 1) % self.palette.len();
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

// --- Low-level drawing (straight onto the shared surface) --------------------

/// ARGB8888 -> render Color.
fn col(argb: u32) -> Color {
    Color::rgba(
        ((argb >> 16) & 0xFF) as u8,
        ((argb >> 8) & 0xFF) as u8,
        (argb & 0xFF) as u8,
        ((argb >> 24) & 0xFF) as u8,
    )
}

/// Draw one line of text with its top-left at (x, y_top). Mirrors the runtime
/// painter's baseline math so it looks identical to DUI-rendered text.
fn draw_text(
    surf: &mut Surface,
    font: &Font,
    x: i32,
    y_top: i32,
    px: f32,
    color: Color,
    text: &str,
) {
    let baseline = y_top as f32 + font.line_metrics(px).ascent;
    let (glyphs, _adv) = font.layout_line(text, px);
    for g in glyphs {
        if let Some(bmp) = font.rasterize(g.glyph, px) {
            let ox = (x as f32 + g.x + 0.5) as i32 + bmp.left;
            let oy = (baseline + 0.5) as i32 - bmp.top;
            surf.blit_glyph(&bmp, ox, oy, color);
        }
    }
}

/// Draw a small labelled button; returns nothing (hotspot is registered by the
/// caller so hit rects stay in one place).
fn button(surf: &mut Surface, font: &Font, theme: &Theme, x: i32, y: i32, w: i32, label: &str) {
    let h = ROW_H - 6;
    surf.fill_rect(x as f32, y as f32, w as f32, h as f32, col(theme.taskbtn));
    surf.stroke_rect(
        x as f32,
        y as f32,
        w as f32,
        h as f32,
        1.0,
        col(theme.border_unfocused),
    );
    draw_text(surf, font, x + 6, y + 2, 13.0, col(theme.panel_text), label);
}


fn push_hex_byte(s: &mut String, v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    s.push(HEX[((v >> 4) & 0xF) as usize] as char);
    s.push(HEX[(v & 0xF) as usize] as char);
}
fn hex_argb(argb: u32) -> String {
    let mut s = String::from("#");
    push_hex_byte(&mut s, (argb >> 24) & 0xFF);
    push_hex_byte(&mut s, (argb >> 16) & 0xFF);
    push_hex_byte(&mut s, (argb >> 8) & 0xFF);
    push_hex_byte(&mut s, argb & 0xFF);
    s
}
fn push_int(s: &mut String, mut v: i32) {
    if v < 0 {
        s.push('-');
        v = -v;
    }
    if v == 0 {
        s.push('0');
        return;
    }
    let mut tmp = [0u8; 12];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        s.push(tmp[n] as char);
    }
}


/// Paint the whole editor into the shared surface and return the hit rects.
fn render(px_ptr: *mut u8, font: &Font, app: &App) -> Vec<Hotspot> {
    // Maintain the full form on small surfaces and expose its clipped portion
    // through scrolling. Large surfaces expand the backdrop without stretching text.
    let canvas_w = app.width.max(if app.editor_section().is_some() { 1200 } else { W });
    let canvas_h = app.height.saturating_sub(FOOTER_H).max(app.content_height());
    let mut canvas = alloc::vec![0u32; canvas_w as usize * canvas_h as usize];
    let mut surf = Surface::new(&mut canvas, canvas_w as usize, canvas_h as usize);
    let mut spots: Vec<Hotspot> = Vec::new();

    let theme = &app.s.theme;
    let foreground = col(theme.panel_text);
    let accent = col(theme.border_focused);
    let sidebar = 154;
    let content = sidebar + 22;
    surf.fill_rect(
        0.0,
        0.0,
        canvas_w as f32,
        canvas_h as f32,
        col(theme.menu),
    );
    surf.fill_rect(0.0, 0.0, sidebar as f32, canvas_h as f32, col(theme.panel));
    surf.fill_rect(
        sidebar as f32,
        0.0,
        1.0,
        canvas_h as f32,
        col(theme.border_unfocused),
    );
    draw_text(&mut surf, font, 16, 22, 18.0, foreground, "Settings");
    const PAGES: [&str; 13] = ["Appearance", "Animations", "Desktop geometry", "Display", "Lua Scripts", "Themes", "Panels", "Typography", "Widgets", "Keybindings", "Window Rules", "Lua event hook", "Desktop profiles"];
    for (page, label) in PAGES.iter().enumerate() {
        let y = if page < 6 { 76 + page as i32 * 40 }
            else { 316 + (page as i32 - 6) * 30 };
        let nav_height = if page < 6 { 32 } else { 28 };
        if app.page == page {
            surf.fill_rect(
                8.0,
                y as f32,
                (sidebar - 16) as f32,
                nav_height as f32,
                col(theme.taskbtn_focused),
            );
            surf.fill_rect(8.0, y as f32, 3.0, nav_height as f32, accent);
        }
        draw_text(&mut surf, font, 18, y + 8, 13.0, foreground, label);
        spots.push(Hotspot {
            x: 8,
            y,
            w: sidebar - 16,
            h: nav_height,
            act: Action::SelectPage(page),
        });
    }
    draw_text(
        &mut surf,
        font,
        content,
        28,
        23.0,
        foreground,
        PAGES[app.page],
    );
    draw_text(
        &mut surf,
        font,
        content,
        65,
        12.0,
        foreground,
        if app.editor_section().is_some() { "Edit a draft, then Apply to validate and reload." }
            else { "Changes save and apply to the current desktop." },
    );
    let mut y = Y0;
    if app.page == 0 {
        for i in 0..NCOLORS {
            draw_text(
                &mut surf,
                font,
                content,
                y,
                14.0,
                foreground,
                COLOR_LABELS[i],
            );
            y += ROW_H;
            button(&mut surf, font, theme, content, y, 26, "<");
            spots.push(Hotspot {
                x: content,
                y,
                w: 26,
                h: ROW_H,
                act: Action::ColorPrev(i),
            });
            let c = color_get(theme, i);
            surf.fill_rect((content + 34) as f32, y as f32, 40.0, 18.0, col(c));
            surf.stroke_rect(
                (content + 34) as f32,
                y as f32,
                40.0,
                18.0,
                1.0,
                col(theme.border_unfocused),
            );
            button(&mut surf, font, theme, content + 82, y, 26, ">");
            spots.push(Hotspot {
                x: content + 82,
                y,
                w: 26,
                h: ROW_H,
                act: Action::ColorNext(i),
            });
            draw_text(
                &mut surf,
                font,
                content + 124,
                y + 2,
                14.0,
                foreground,
                &hex_argb(c),
            );
            y += ROW_H + 8;
        }
    } else if app.page == 1 {
        for (i, label) in TOGGLE_LABELS.iter().enumerate() {
            let on = toggle_get(&app.s.effects, i);
            button(
                &mut surf,
                font,
                theme,
                content,
                y,
                62,
                if on { "[x] on" } else { "[ ] off" },
            );
            spots.push(Hotspot {
                x: content,
                y,
                w: 62,
                h: ROW_H,
                act: Action::Toggle(i),
            });
            draw_text(
                &mut surf,
                font,
                content + 84,
                y + 2,
                14.0,
                foreground,
                label,
            );
            y += 38;
        }
    }
    // Effect geometry belongs beside its toggles; window/shell geometry has
    // its own category. Each existing control appears in exactly one page.
    for (i, spec) in INT_SPECS.iter().enumerate() {
        if !((app.page == 1 && i >= 3) || (app.page == 2 && i < 3)) {
            continue;
        }
        draw_text(&mut surf, font, content, y, 14.0, foreground, spec.label);
        y += ROW_H;
        button(&mut surf, font, theme, content, y, 26, "-");
        spots.push(Hotspot {
            x: content,
            y,
            w: 26,
            h: ROW_H,
            act: Action::IntDelta(i, -spec.step),
        });
        let mut val = String::new();
        push_int(&mut val, int_get(&app.s, i));
        draw_text(&mut surf, font, content + 42, y + 2, 14.0, foreground, &val);
        button(&mut surf, font, theme, content + 100, y, 26, "+");
        spots.push(Hotspot {
            x: content + 100,
            y,
            w: 26,
            h: ROW_H,
            act: Action::IntDelta(i, spec.step),
        });
        y += ROW_H + 16;
    }
    if app.page == 3 {
        draw_text(
            &mut surf,
            font,
            content,
            y,
            14.0,
            foreground,
            "Desktop resolution",
        );
        y += 38;
        button(&mut surf, font, theme, content, y, 26, "<");
        spots.push(Hotspot {
            x: content,
            y,
            w: 26,
            h: ROW_H,
            act: Action::ResPrev,
        });
        button(&mut surf, font, theme, content + 42, y, 26, ">");
        spots.push(Hotspot {
            x: content + 42,
            y,
            w: 26,
            h: ROW_H,
            act: Action::ResNext,
        });
        let (rw, rh) = RES_PRESETS[app.res_idx];
        let mut label = String::new();
        if rw == 0 || rh == 0 {
            label.push_str("Keep boot mode");
        } else {
            push_int(&mut label, rw as i32);
            label.push('x');
            push_int(&mut label, rh as i32);
        }
        draw_text(
            &mut surf,
            font,
            content + 90,
            y + 2,
            16.0,
            foreground,
            &label,
        );
        draw_text(
            &mut surf,
            font,
            content,
            y + 48,
            12.0,
            foreground,
            "The desktop rebuilds buffers after a mode change.",
        );
        draw_text(
            &mut surf,
            font,
            content,
            y + 68,
            12.0,
            foreground,
            "Fixed framebuffer backends keep their boot mode.",
        );
    }
    if app.page == 4 {
        button(&mut surf, font, theme, content, y, 64,
            if app.s.scripts.enabled { "[x] on" } else { "[ ] off" });
        spots.push(Hotspot { x: content, y, w: 64, h: ROW_H, act: Action::ScriptsToggle });
        draw_text(&mut surf, font, content + 80, y + 2, 14.0, foreground, "Lua window policy");
        y += 44;
        draw_text(&mut surf, font, content, y, 14.0, foreground, "Layout script");
        y += ROW_H;
        if !app.script_layouts.is_empty() {
            for (offset, label, direction) in [(0, "<", -1), (38, ">", 1)] {
                button(&mut surf, font, theme, content + offset, y, 26, label);
                spots.push(Hotspot { x: content + offset, y, w: 26, h: ROW_H,
                    act: Action::ScriptLayout(direction) });
            }
        }
        let filename = app.s.scripts.layout.as_str().rsplit('/').next().unwrap_or("");
        draw_text(&mut surf, font, content + 80, y + 2, 12.0, foreground, filename);
        y += 30;
        draw_text(&mut surf, font, content, y, 11.0, foreground, &app.scripts_directory);
        y += 24;
        draw_text(&mut surf, font, content, y, 11.0, foreground, app.scripts_listing_status);
        y += 36;
        draw_text(&mut surf, font, content, y, 14.0, foreground, "Runner timeout (ms)");
        y += ROW_H;
        for (offset, label, delta) in [(0, "-", -50), (100, "+", 50)] {
            button(&mut surf, font, theme, content + offset, y, 26, label);
            spots.push(Hotspot { x: content + offset, y, w: 26, h: ROW_H,
                act: Action::ScriptTimeout(delta) });
        }
        let mut timeout = String::new();
        push_int(&mut timeout, app.s.scripts.timeout_ms as i32);
        draw_text(&mut surf, font, content + 38, y + 2, 14.0, foreground, &timeout);
        y += 42;
        draw_text(&mut surf, font, content, y, 14.0, foreground, "Runner (read-only)");
        y += ROW_H;
        draw_text(&mut surf, font, content, y, 12.0, foreground, app.s.scripts.runner.as_str());
        y += 28;
        draw_text(&mut surf, font, content, y, 11.0, foreground, "Use setup to edit scripts.runner and the script folder.");
        y += 24;
        draw_text(&mut surf, font, content, y, 11.0, foreground, "Lua applies placement and fade policy asynchronously.");
    }
    if app.page == 5 {
        draw_text(&mut surf, font, content, y, 12.0, foreground,
            app.s.desktop.themes.as_str());
        y += 28;
        draw_text(&mut surf, font, content, y, 11.0, foreground,
            app.themes_listing_status);
        y += 40;
        for (index, path) in app.themes.iter().enumerate() {
            let filename = path.rsplit('/').next().unwrap_or(path);
            button(&mut surf, font, theme, content, y, 240, filename);
            spots.push(Hotspot { x: content, y, w: 240, h: ROW_H,
                act: Action::ImportTheme(index) });
            button(&mut surf, font, theme, content + 248, y, 120, "Full desktop");
            spots.push(Hotspot { x: content + 248, y, w: 120, h: ROW_H,
                act: Action::ImportDesktop(index) });
            y += 40;
        }
    }
    if app.page == 12 {
        draw_text(&mut surf, font, content, y, 12.0, foreground,
            "Complete profiles replace panels, widgets, registry, rules and settings.");
        y += 28;
        draw_text(&mut surf, font, content, y, 12.0, foreground,
            "Choose a bundled profile or type any absolute TOML file path.");
        y += 32;
        surf.fill_rect(content as f32, y as f32, 390.0, 28.0,
            col(if app.profile_focused { theme.taskbtn_focused } else { theme.panel }));
        let path = app.profile_path.lines.first().map(String::as_str).unwrap_or("");
        let cursor = app.profile_path.cursor.min(path.len());
        let start = path[..cursor].char_indices().rev().nth(42).map(|p| p.0).unwrap_or(0);
        let end = path[start..].char_indices().nth(50).map(|p| start + p.0).unwrap_or(path.len());
        let shown = &path[start..end];
        let label = if app.profile_focused && cursor <= end {
            alloc::format!("{}|{}", &shown[..cursor-start], &shown[cursor-start..])
        } else { String::from(shown) };
        draw_text(&mut surf, font, content + 4, y + 6, 12.0, foreground, &label);
        spots.push(Hotspot { x: content, y, w: 390, h: 28, act: Action::FocusProfilePath });
        y += 40;
        button(&mut surf, font, theme, content, y, 232, "Import complete desktop");
        spots.push(Hotspot { x: content, y, w: 232, h: ROW_H, act: Action::ImportProfile });
        y += 40;
        for (index, path) in app.profiles.iter().enumerate() {
            let filename = path.rsplit('/').next().unwrap_or(path);
            button(&mut surf, font, theme, content, y, 370, filename);
            spots.push(Hotspot { x: content, y, w: 370, h: ROW_H, act: Action::SelectProfile(index) });
            y += 36;
        }
    }
    if let Some(section) = app.editor_section() {
        let hints = match section {
            Section::Panels => "Panel containers: edge, size, children, autohide and reservation.",
            Section::Widgets => "Components: kind, children, format, entries, action and class.",
            Section::Bindings => "Bindings: scancode, modifiers and checked action string.",
            Section::Rules => "Rules: app matcher, geometry, opacity, floating and animation.",
            Section::Typography => "Font path and panel/title sizes; values use their original tables.",
            Section::Scripts => "Layout and event hooks run outside the compositor frame loop.",
        };
        draw_text(&mut surf, font, content, y, 12.0, foreground, hints);
        y += 28;
        draw_text(&mut surf, font, content, y, 11.0, foreground,
            "Click a line; type to edit. Arrows/Home/End move; Backspace/Delete erase.");
        y += 28;
        for (offset, width, label, act) in [
            (0, 76, "Apply", Action::SaveDocument),
            (84, 76, "Revert", Action::RevertDocument),
            (168, 82, "+ line", Action::InsertLine),
            (258, 90, "+ table", Action::NewTable),
            (356, 90, "- line", Action::DeleteLine),
        ] {
            if matches!(act, Action::NewTable) && matches!(section, Section::Typography | Section::Scripts) { continue; }
            button(&mut surf, font, theme, content + offset, y, width, label);
            spots.push(Hotspot { x: content + offset, y, w: width, h: ROW_H, act });
        }
        y += 40;
        if let Some(editor) = &app.editor {
            let rows = editor.rows(section);
            let pages = (rows.len().saturating_sub(1) / 12) + 1;
            draw_text(&mut surf, font, content, y, 12.0, foreground,
                &alloc::format!("Page {}/{} — {}", app.editor_page.min(pages - 1) + 1, pages,
                    if editor.dirty { "unsaved draft" } else { "saved document" }));
            for (offset, label, delta) in [(270, "<", -1), (310, ">", 1)] {
                button(&mut surf, font, theme, content + offset, y - 4, 28, label);
                spots.push(Hotspot { x: content + offset, y: y - 4, w: 28, h: ROW_H,
                    act: Action::EditorPage(delta) });
            }
            y += 32;
            if rows.is_empty() {
                draw_text(&mut surf, font, content, y, 12.0, foreground,
                    "No tables configured. Add a table, then add its fields as lines.");
            }
            for &index in rows.iter().skip(app.editor_page.min(pages - 1) * 12).take(12) {
                let selected = editor.selected == Some(index);
                if selected { surf.fill_rect(content as f32, (y - 3) as f32, 1000.0, 26.0,
                    col(theme.taskbtn_focused)); }
                let line = &editor.lines[index];
                let start = if selected { line[..editor.cursor].char_indices().rev().nth(100)
                    .map(|p| p.0).unwrap_or(0) } else { 0 };
                let end = line[start..].char_indices().nth(120).map(|p| start + p.0).unwrap_or(line.len());
                let visible = &line[start..end];
                let display = if selected && editor.cursor >= start && editor.cursor <= end {
                    let local = editor.cursor - start;
                    alloc::format!("{} | {}", &visible[..local], &visible[local..])
                } else { String::from(visible) };
                draw_text(&mut surf, font, content, y, 13.0, foreground, &display);
                spots.push(Hotspot { x: content, y: y - 3, w: 1000, h: 26,
                    act: Action::EditLine(index) });
                y += 30;
            }
        }
    }
    drop(surf);
    let pixels = unsafe {
        core::slice::from_raw_parts_mut(
            px_ptr as *mut u32,
            app.width as usize * app.height as usize,
        )
    };
    let mut target = Surface::new(pixels, app.width as usize, app.height as usize);
    target.clear(col(theme.menu));
    drop(target);
    let visible_h = app.height.saturating_sub(FOOTER_H) as usize;
    let sx = app
        .scroll_x
        .clamp(0, canvas_w.saturating_sub(app.width) as i32) as usize;
    let sy = app
        .scroll_y
        .clamp(0, canvas_h.saturating_sub(visible_h as u32) as i32) as usize;
    for row in 0..visible_h {
        let source = (row + sy) * canvas_w as usize + sx;
        let destination = row * app.width as usize;
        if app.width as i32 >= sidebar + 160 {
            let fixed = sidebar as usize;
            let fixed_source = row * canvas_w as usize;
            pixels[destination..destination + fixed]
                .copy_from_slice(&canvas[fixed_source..fixed_source + fixed]);
            pixels[destination + fixed..destination + app.width as usize]
                .copy_from_slice(&canvas[source + fixed..source + app.width as usize]);
        } else {
            pixels[destination..destination + app.width as usize]
                .copy_from_slice(&canvas[source..source + app.width as usize]);
        }
    }
    for spot in &mut spots {
        let fixed_sidebar = app.width as i32 >= sidebar + 160;
        let navigation = matches!(spot.act, Action::SelectPage(_));
        if !(fixed_sidebar && navigation) {
            spot.x -= sx as i32;
            spot.y -= sy as i32;
        }
        // Match the fixed navigation and scrolling content viewports.
        let right = (spot.x + spot.w).min(app.width as i32);
        let bottom = (spot.y + spot.h).min(visible_h as i32);
        spot.x = spot.x.max(if fixed_sidebar && !navigation { sidebar } else { 0 });
        spot.y = spot.y.max(0);
        spot.w = (right - spot.x).max(0);
        spot.h = (bottom - spot.y).max(0);
    }
    let mut footer = Surface::new(pixels, app.width as usize, app.height as usize);
    let footer_y = app.height.saturating_sub(FOOTER_H) as i32;
    footer.fill_rect(
        0.0,
        footer_y as f32,
        app.width as f32,
        FOOTER_H as f32,
        col(theme.panel),
    );
    for (i, (label, action)) in [
        ("<", Action::Scroll(-100, 0)),
        (">", Action::Scroll(100, 0)),
        ("^", Action::Scroll(0, -100)),
        ("v", Action::Scroll(0, 100)),
    ]
    .iter()
    .enumerate()
    {
        let x = 8 + i as i32 * 30;
        button(&mut footer, font, theme, x, footer_y + 3, 24, label);
        spots.push(Hotspot {
            x,
            y: footer_y + 3,
            w: 24,
            h: 20,
            act: *action,
        });
    }
    let mut status_line = String::new();
    let mut status_y = footer_y + 27;
    let status_color = col(if app.error {
        theme.close
    } else {
        theme.panel_text
    });
    let status = if app.document_error.is_empty() { app.status } else { &app.document_error };
    for word in status.split_whitespace() {
        let candidate = if status_line.is_empty() {
            String::from(word)
        } else {
            alloc::format!("{} {}", status_line, word)
        };
        if font.layout_line(&candidate, 11.0).1 > app.width.saturating_sub(16) as f32
            && !status_line.is_empty()
        {
            draw_text(
                &mut footer,
                font,
                8,
                status_y,
                11.0,
                status_color,
                &status_line,
            );
            status_y += 13;
            status_line.clear();
        }
        if !status_line.is_empty() {
            status_line.push(' ');
        }
        status_line.push_str(word);
    }
    draw_text(
        &mut footer,
        font,
        8,
        status_y,
        11.0,
        status_color,
        &status_line,
    );

    spots
}

/// Respond to a server CONFIGURE with a fresh backing buffer and object id.
/// Keep the old local surface intact until the replacement has been queued.
#[allow(clippy::too_many_arguments)]
fn resize_surface(
    compositor: u32,
    font: &Font,
    app: &mut App,
    width: u32,
    height: u32,
    token: u64,
    object: u64,
    serial: &mut u64,
) -> Option<(u32, *mut u8)> {
    let bytes = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    // Bound allocations and ensure the footer can remain visible. This is a
    // buffer safety limit, independent of desktop resolution preferences.
    let canvas_bytes = (width.max(W) as usize)
        .checked_mul(height.saturating_sub(FOOTER_H).max(app.content_height()) as usize)?
        .checked_mul(4)?;
    if width < 144
        || height < FOOTER_H + 16
        || bytes > 64 * 1024 * 1024
        || canvas_bytes > 64 * 1024 * 1024
    {
        return None;
    }
    let handle = libdunit::handle_create_shared(bytes);
    if handle <= 0 {
        libdunit::println(&alloc::format!("gui_settings: resize create bytes={} errno={}", bytes, handle));
        return None;
    }
    let handle = handle as u32;
    let mapping = libdunit::handle_map(handle, 0, bytes);
    if mapping <= 0 {
        libdunit::handle_close(handle);
        return None;
    }
    let duplicate = libdunit::handle_dup(
        handle,
        libdunit::RIGHT_READ | libdunit::RIGHT_MAP | libdunit::RIGHT_TRANSFER,
    );
    if duplicate <= 0 {
        libdunit::shared_buffer::release(handle, mapping as usize, bytes);
        return None;
    }
    let remote = libdunit::handle_transfer(duplicate as u32, compositor);
    if remote <= 0 {
        libdunit::handle_close(duplicate as u32);
        libdunit::shared_buffer::release(handle, mapping as usize, bytes);
        return None;
    }
    let announcement = gui_protocol_v1::control::Capability {
        handle: remote as u32,
        advertised_bytes: bytes as u32,
        object: object as u32,
    }
    .encode();
    let ack = Request::AckConfigure { configure: token }.encode(SURFACE, *serial);
    *serial += 1;
    let import = Request::ImportBuffer {
        width,
        height,
        stride: width * 4,
        format: FMT_XRGB8888,
        offset: 0,
    }
    .encode(object, *serial);
    *serial += 1;
    let attach = Request::AttachBuffer {
        buffer: object,
        damage: Vec::new(),
    }
    .encode(SURFACE, *serial);
    *serial += 1;
    let commit = Request::Commit {
        configure: token,
        frame_callback: 1,
    }
    .encode(SURFACE, *serial);
    *serial += 1;
    let destroy = Request::DestroyBuffer.encode(object - 1, *serial);
    *serial += 1;
    let previous = (app.width, app.height, app.scroll_x, app.scroll_y);
    app.width = width;
    app.height = height;
    app.scroll_x = app.scroll_x.min((if app.editor_section().is_some() { 1200u32 } else { W }).saturating_sub(width) as i32);
    app.scroll_y = app
        .scroll_y
        .min(app.content_height().saturating_sub(height.saturating_sub(FOOTER_H)) as i32);
    render(mapping as usize as *mut u8, font, app);
    for packet in [
        &ack[..],
        &announcement[..],
        &import[..],
        &attach[..],
        &commit[..],
        &destroy[..],
    ] {
        if libdunit::ipc_send(compositor, packet) < 0 {
            app.width = previous.0;
            app.height = previous.1;
            app.scroll_x = previous.2;
            app.scroll_y = previous.3;
            libdunit::shared_buffer::release(handle, mapping as usize, bytes);
            return None;
        }
    }
    Some((handle, mapping as usize as *mut u8))
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::println("gui_settings: PANIC");
    libdunit::exit(101)
}

fn u64_at(p: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&p[off..off + 8]);
    u64::from_le_bytes(a)
}

// APPEND_START

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // 1) Handshake: compositor pid + our client id (id is a tint hint only).
    let mut m = [0u8; 8];
    if libdunit::ipc_recv_blocking(&mut m, 0) < 8 {
        libdunit::println("gui_settings: FAIL recv handshake");
        libdunit::exit(1);
    }
    let compositor = u32::from_le_bytes([m[0], m[1], m[2], m[3]]);

    // 2) Create + map our own shared buffer and paint the initial editor.
    let bytes = (W * H * 4) as usize;
    let buf = libdunit::handle_create_shared(bytes);
    if buf <= 0 {
        libdunit::println("gui_settings: FAIL create buffer");
        libdunit::exit(2);
    }
    let mut buf = buf as u32;
    let mapped = libdunit::handle_map(buf, 0, bytes);
    if mapped <= 0 {
        libdunit::println("gui_settings: FAIL map buffer");
        libdunit::exit(3);
    }
    let mut px = mapped as usize as *mut u8;
    let font = match load_font() {
        Ok(f) => f,
        Err(_) => {
            libdunit::println("gui_settings: FAIL font parse");
            libdunit::exit(7);
        }
    };
    let mut app = App::new();
    let mut spots = render(px, &font, &app);

    let mut rx = [0u8; 256];

    // 3) HELLO -> WELCOME.
    let hello = Request::Hello {
        min_minor: 0,
        max_minor: 0,
        offered_features: FEATURE_ARGB8888,
        required_features: 0,
    }
    .encode(0, 1);
    libdunit::ipc_send(compositor, &hello);
    if libdunit::ipc_recv_blocking(&mut rx, 0) <= 0 {
        libdunit::println("gui_settings: FAIL welcome");
        libdunit::exit(4);
    }
    // APPEND_START2

    // 4) CREATE_SURFACE -> RESULT + CONFIGURE (capture the configure token).
    let create = Request::CreateSurface {
        role: 1,
        width: W,
        height: H,
        format: FMT_XRGB8888,
    }
    .encode(SURFACE, 2);
    libdunit::ipc_send(compositor, &create);
    libdunit::ipc_recv_blocking(&mut rx, 0); // RESULT
    let n = libdunit::ipc_recv_blocking(&mut rx, 0); // CONFIGURE
    let token = if n >= 32 { u64_at(&rx, 24) } else { 0 };

    // 5) ACK_CONFIGURE -> RESULT.
    let ack = Request::AckConfigure { configure: token }.encode(SURFACE, 3);
    libdunit::ipc_send(compositor, &ack);
    libdunit::ipc_recv_blocking(&mut rx, 0);

    // 6) Transfer the buffer capability (read-only) and announce it.
    let dup = libdunit::handle_dup(
        buf,
        libdunit::RIGHT_READ | libdunit::RIGHT_MAP | libdunit::RIGHT_TRANSFER,
    );
    let ch = if dup > 0 {
        libdunit::handle_transfer(dup as u32, compositor)
    } else {
        -1
    };
    if ch <= 0 {
        libdunit::println("gui_settings: FAIL cap transfer");
        libdunit::exit(5);
    }
    let ann = gui_protocol_v1::control::Capability {
        handle: ch as u32,
        advertised_bytes: bytes as u32,
        object: BUFFER as u32,
    }
    .encode();
    libdunit::ipc_send(compositor, &ann);

    // 7) IMPORT / ATTACH / COMMIT (each -> RESULT).
    let import = Request::ImportBuffer {
        width: W,
        height: H,
        stride: W * 4,
        format: FMT_XRGB8888,
        offset: 0,
    }
    .encode(BUFFER, 4);
    libdunit::ipc_send(compositor, &import);
    libdunit::ipc_recv_blocking(&mut rx, 0);
    let attach = Request::AttachBuffer {
        buffer: BUFFER,
        damage: alloc::vec::Vec::new(),
    }
    .encode(SURFACE, 5);
    libdunit::ipc_send(compositor, &attach);
    libdunit::ipc_recv_blocking(&mut rx, 0);
    let commit = Request::Commit {
        configure: token,
        frame_callback: 1,
    }
    .encode(SURFACE, 6);
    libdunit::ipc_send(compositor, &commit);
    libdunit::ipc_recv_blocking(&mut rx, 0);
    // APPEND_START3

    // 8) FRAME_DONE from the compositor's composition tick.
    let n = libdunit::ipc_recv_blocking(&mut rx, 0);
    let ok = n >= 32 && u16::from_le_bytes([rx[8], rx[9]]) == 0x8021;
    if ok {
        libdunit::println("gui_settings: surface presented OK");
    } else {
        libdunit::println("gui_settings: FAIL no frame done");
        libdunit::handle_close(buf);
        libdunit::exit(6);
    }

    // 8.5) Headless resolution-picker proof (config-gated `[startup] self_test`,
    //      true only in test.toml — NEVER on the real desktop). The automated
    //      harness has no mouse, so we actuate the SAME `apply(ResNext)` path a
    //      `>` click drives, walking the picker to a concrete non-boot mode, then
    //      persist it (`dwm_settings::save`) and ping the compositor to reload —
    //      exactly what a click does. This drives the full GUI -> [display] ->
    //      live-resolution round trip: the compositor diffs the new `[display]`,
    //      re-enters its session and asks the kernel backend to mode-set. One-shot
    //      (fires once before the input loop), so the compositor re-enters exactly
    //      once — no reload storm.
    if dwm_settings::load_config().apps.self_test {
        let previous = app.s;
        let previous_resolution = app.res_idx;
        let target = (1600u32, 900u32);
        let mut guard = 0;
        while RES_PRESETS[app.res_idx] != target && guard < NRES {
            app.apply(Action::ResNext);
            guard += 1;
        }
        if dwm_settings::save(&app.s) {
            let sig = gui_protocol_v1::control::RELOAD;
            libdunit::ipc_send(compositor, &sig);
            spots = render(px, &font, &app);
            let (tw, th) = RES_PRESETS[app.res_idx];
            let mut msg = String::from("gui_settings: res picker self-test picked ");
            push_int(&mut msg, tw as i32);
            msg.push('x');
            push_int(&mut msg, th as i32);
            msg.push_str(" (saved, reload pinged)");
            libdunit::println(&msg);
        } else {
            app.s = previous;
            app.res_idx = previous_resolution;
            app.status = "Save failed. Previous settings restored.";
            app.error = true;
            spots = render(px, &font, &app);
            libdunit::println("gui_settings: FAIL resolution picker save");
        }
    }

    // 9) Interactive loop: a pointer-down inside a hotspot mutates the working
    //    Settings, writes them back to the config (RAM this session) and pings
    //    the compositor to reload — the GUI<->TOML round-trip applied live. We
    //    then repaint our shared buffer so the swatch/value reflects the change.
    let mut serial = 7u64;
    let mut next_buffer = BUFFER + 1;
    let mut draft_repaint = false;
    loop {
        // Drain queued edits before rasterizing the complete source editor.
        // A burst of keystrokes must not fill the IPC mailbox behind repaints.
        let n = if draft_repaint {
            let pending = libdunit::ipc_recv(&mut rx);
            if pending > 0 { pending } else {
                spots = render(px, &font, &app);
                draft_repaint = false;
                libdunit::ipc_recv_blocking(&mut rx, 0)
            }
        } else { libdunit::ipc_recv_blocking(&mut rx, 0) };
        if n >= 48
            && u32::from_le_bytes([rx[0], rx[1], rx[2], rx[3]]) == gui_protocol_v1::wire::MAGIC
            && u16::from_le_bytes([rx[8], rx[9]]) == 0x8010
        {
            let width = u32::from_le_bytes([rx[32], rx[33], rx[34], rx[35]]);
            let height = u32::from_le_bytes([rx[36], rx[37], rx[38], rx[39]]);
            let token = u64_at(&rx, 24);
            let previous_bytes = app.width as usize * app.height as usize * 4;
            if let Some((new_buf, new_px)) = resize_surface(
                compositor,
                &font,
                &mut app,
                width,
                height,
                token,
                next_buffer,
                &mut serial,
            ) {
                libdunit::shared_buffer::release(buf, px as usize, previous_bytes);
                buf = new_buf;
                px = new_px;
                next_buffer += 1;
                spots = render(px, &font, &app);
                libdunit::println("gui_settings: reconfigured OK");
            } else {
                app.status = "Resize failed; previous surface retained.";
                app.error = true;
                spots = render(px, &font, &app);
            }
            continue;
        }
        if n >= 32
            && u32::from_le_bytes([rx[0], rx[1], rx[2], rx[3]]) == gui_protocol_v1::wire::MAGIC
            && u16::from_le_bytes([rx[8], rx[9]]) == 0x8003
        {
            app.status = "Compositor rejected a surface request.";
            app.error = true;
            spots = render(px, &font, &app);
            libdunit::println("gui_settings: surface protocol error");
            continue;
        }
        let Some(input) = gui_protocol_v1::control::Input::decode(&rx[..n.max(0) as usize]) else {
            continue;
        };
        let kind = input.kind;
        if kind == IN_QUIT {
            break;
        }
        if kind == IN_SCROLL && n >= 12 {
            let delta = input.x;
            app.apply(Action::Scroll(0, delta.saturating_mul(-32)));
            spots = render(px, &font, &app);
        }
        if kind == IN_KEY && n >= 17 && app.page == 12 && app.profile_focused {
            app.profile_path.key(input.x as u8, input.value as u8);
            app.document_error.clear();
            app.status = "Path edited. Import complete desktop validates the file before saving.";
            app.error = false;
            draft_repaint = true;
        }
        if kind == IN_KEY && n >= 17 && app.editor_section().is_some() {
            app.document_error.clear();
            if let Some(editor) = &mut app.editor {
                editor.key(input.x as u8, input.value as u8);
                app.status = "Draft only. Apply validates and saves; Revert reloads the saved document.";
                app.error = false;
                draft_repaint = true;
            }
        }
        if kind == IN_DOWN && n >= 16 {
            let lx = input.x;
            let ly = input.y;
            let mut act = None;
            for hs in spots.iter() {
                if hs.hit(lx, ly) {
                    act = Some(hs.act);
                    break;
                }
            }
            if let Some(a) = act {
                app.document_error.clear();
                if matches!(a, Action::SaveDocument | Action::ImportDesktop(_) | Action::ImportProfile) {
                    let import_path = match a {
                        Action::ImportDesktop(index) => Some(app.themes.get(index).cloned().unwrap_or_default()),
                        Action::ImportProfile => Some(app.profile_path.lines.first().cloned().unwrap_or_default()),
                        _ => None,
                    };
                    let result = if let Some(path) = &import_path {
                        dwm_settings::load_document_from(path)
                            .and_then(|text| {
                                let config = dwm_settings::validate_document(&text)?;
                                if !config.valid || config.apps.apps.is_empty() {
                                    return Err(dwm_settings::ConfigError { line: 0, field: String::new(),
                                        reason: "Full desktop import requires an application registry. Use theme overlay instead." });
                                }
                                if matches!(a,Action::ImportProfile){dwm_settings::save_user_document(&text)}else{dwm_settings::save_document(&text)}
                            })
                    } else if let Some(editor) = &app.editor {
                        dwm_settings::load_document().and_then(|current| {
                            if current != editor.original {
                                return Err(dwm_settings::ConfigError { line: 0, field: String::new(),
                                    reason: "Desktop changed elsewhere. Revert before applying this draft." });
                            }
                            dwm_settings::save_document(&editor.text())
                        })
                    } else { continue; };
                    match result {
                        Err(error) => {
                            app.status = error.reason; app.error = true;
                            app.document_error = alloc::format!("{} line {} {}: {}",
                                import_path.as_deref().unwrap_or("saved desktop"), error.line, error.field, error.reason);
                            libdunit::println(&alloc::format!("gui_settings: document rejected file={} line={} field={} reason={}",
                                import_path.as_deref().unwrap_or("saved desktop"), error.line, error.field, error.reason));
                        },
                        Ok(()) => {
                            app.refresh_document(); app.s = dwm_settings::load(); app.refresh_color_choices();
                            let signal = gui_protocol_v1::control::RELOAD;
                            app.error = libdunit::ipc_send(compositor, &signal) < 0;
                            app.status = if app.error { "Document saved; reload request failed." }
                                else { "Document validated and saved; desktop reload requested." };
                            libdunit::println("gui_settings: complete document saved");
                            if let Some(path) = &import_path {
                                libdunit::println(&alloc::format!("gui_settings: desktop profile saved {}",path));
                            }
                        },
                    }
                    spots = render(px, &font, &app);
                    continue;
                }
                if matches!(a, Action::SelectPage(_) | Action::Scroll(_, _) | Action::EditLine(_)
                    | Action::InsertLine | Action::NewTable | Action::DeleteLine | Action::EditorPage(_)
                    | Action::RevertDocument | Action::SelectProfile(_) | Action::FocusProfilePath) {
                    app.apply(a);
                    spots = render(px, &font, &app);
                    match a {
                        Action::SelectPage(page) => libdunit::println(&alloc::format!("gui_settings: page {}", page)),
                        Action::Scroll(_, _) => libdunit::println(&alloc::format!("gui_settings: scroll {},{}", app.scroll_x, app.scroll_y)),
                        _ => {},
                    }
                    continue;
                }
                let previous = app.s;
                let previous_colors = app.color_idx;
                let previous_resolution = app.res_idx;
                if let Action::ImportTheme(index) = a {
                    let candidate = app.themes.get(index)
                        .ok_or("Theme selection is no longer available.")
                        .and_then(|path| dwm_settings::load_overlay(&app.s, path));
                    match candidate {
                        Ok(candidate) => app.s = candidate,
                        Err(error) => {
                            app.status = error;
                            app.error = true;
                            libdunit::println(&alloc::format!("gui_settings: theme rejected; previous settings retained: {}",error));
                            spots = render(px, &font, &app);
                            continue;
                        }
                    }
                } else {
                    app.apply(a);
                }
                // Persist to the live config and ask the compositor to reload,
                // so the desktop reflects the new theme/geometry immediately.
                if !dwm_settings::save(&app.s) {
                    app.s = previous;
                    app.color_idx = previous_colors;
                    app.res_idx = previous_resolution;
                    app.status = "Save failed. Previous settings restored.";
                    app.error = true;
                    libdunit::println("gui_settings: save failed; previous settings retained");
                    spots = render(px, &font, &app);
                    continue;
                }
                if app.editor.as_ref().map(|e| !e.dirty).unwrap_or(false) { app.refresh_document(); }
                if let Action::ImportTheme(index) = a {
                    app.refresh_color_choices();
                    libdunit::println(&alloc::format!("gui_settings: theme saved {}", app.themes[index]));
                }
                let sig = gui_protocol_v1::control::RELOAD;
                if libdunit::ipc_send(compositor, &sig) < 0 {
                    app.status = "Saved; desktop reload could not be requested.";
                    app.error = true;
                } else {
                    app.status = "Saved and desktop reload requested (session RAM).";
                    app.error = false;
                }
                spots = render(px, &font, &app);
            }
        }
    }

    libdunit::shared_buffer::release(buf, px as usize, app.width as usize * app.height as usize * 4);
    libdunit::exit(0)
}
