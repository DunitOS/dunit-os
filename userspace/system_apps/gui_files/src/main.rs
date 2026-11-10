#![no_std]
#![no_main]

//! Icon-grid browser with capability-bound file operations and previews.
//! SDK owns window resources, cached text, menus and filename dialogs.

extern crate alloc;
use dunit::ui::{Component, ComponentWindow, DialogAction, InputOptions, MenuItem, Renderer, View};
use dunit::window::{PixelFormat, WindowEvent};
use dunit::window::{
    IN_DOWN, IN_FOCUS_IN, IN_FOCUS_OUT, IN_KEY, IN_LEAVE, IN_MOVE, IN_SCROLL, IN_UP,
};
type Window = ComponentWindow<Controls>;
mod file_model;
use file_model::Files;
mod file_transfer;

use alloc::string::String;
use alloc::vec::Vec;

use dunit::canvas::{Color, Font, Surface};

// Cooked key bytes the compositor forwards in the IN_KEY `button` field.
const KEY_ENTER_LF: u8 = 0x0a;
const KEY_ENTER_CR: u8 = 0x0d;
const KEY_ESC: u8 = 0x1b;

const W: u32 = 520;
const H: u32 = 380;

// --- Grid geometry (client-local pixels) ---
const PAD: i32 = 10;
const HEADER_H: i32 = 30;
const STATUS_H: i32 = 24;
/// Fixed cell size; the column count and row count derive from the live surface
/// size at runtime (see `Files::cols`/`rows`), since the compositor can resize
/// us via a server-pushed CONFIGURE (maximize/restore).
const CELL_W: i32 = 100;
const CELL_H: i32 = 72;
const ICON: i32 = 32;

// --- Palette (config-resolved ARGB via Color) ---
/// Resolved color scheme, built once from `FilesCfg` (config→behavior). Each
/// field is a straight-alpha ARGB `Color`; the chrome colors are forced opaque
/// and the selection wash keeps its configured alpha so it blends over the grid.
#[derive(Clone, Copy)]
struct Palette {
    header_bg: Color,
    status_bg: Color,
    accent: Color,
    label: Color,
    muted: Color,
    /// Selection wash painted behind the icon+label of the active cell.
    sel_bg: Color,
}

/// Unpack a `0xAARRGGBB` config word into a straight-alpha `Color`.
fn col(argb: u32) -> Color {
    Color::rgba(
        ((argb >> 16) & 0xFF) as u8,
        ((argb >> 8) & 0xFF) as u8,
        (argb & 0xFF) as u8,
        ((argb >> 24) & 0xFF) as u8,
    )
}

/// Load the configured TTF (`[desktop] font`) from the VFS, falling back to the
/// shipped /assets default font on any error — the desktop font is a live config knob.
fn load_font() -> dunit::Result<Font> {
    let cfg = dwm_settings::load();
    Font::load(&[cfg.desktop.font.as_str(), "/assets/fonts/DejaVuSans.ttf"])
}
fn log(message: &str) {
    let _ = dunit::println!("{}", message);
}
fn fatal(why: &str, error: isize) -> ! {
    log(&alloc::format!("gui_files: FAIL {} ({})", why, error));
    dunit::process::exit(1)
}
fn checked<T>(result: dunit::Result<T>, why: &str) -> T {
    result.unwrap_or_else(|e| fatal(why, e.0))
}

fn text_width(font: &Font, text: &str, px: f32) -> f32 {
    font.text_width(text, px)
}
fn draw_text(
    s: &mut Surface,
    font: &Font,
    x: i32,
    baseline: i32,
    px: f32,
    text: &str,
    color: Color,
) {
    font.draw_baseline(s, x, baseline, px, text, color)
}
fn fit_label(font: &Font, text: &str, px: f32, width: f32) -> String {
    font.fit(text, px, width)
}

#[derive(Clone, Copy, PartialEq)]
enum IconKind {
    Folder,
    Text,
    Exec,
    Image,
    Archive,
    Unknown,
}

/// The extension after the last dot (empty if none).
fn ext_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() => &name[i + 1..],
        _ => "",
    }
}

/// Bucket a listing entry. `exec_dir` marks `/app`, whose extensionless files
/// are program binaries and get the executable icon.
fn classify(name: &str, is_dir: bool, exec_dir: bool) -> IconKind {
    if is_dir {
        return IconKind::Folder;
    }
    match ext_of(name) {
        "txt" | "toml" | "md" | "rs" | "cfg" | "conf" | "log" | "json" | "c" | "h" => {
            IconKind::Text
        }
        "svg" | "rgba" | "bmp" | "png" | "jpg" | "jpeg" | "gif" | "ppm" => IconKind::Image,
        "zip" | "gz" | "tar" | "xz" | "iso" => IconKind::Archive,
        "" if exec_dir => IconKind::Exec,
        _ => IconKind::Unknown,
    }
}

/// The six mimetype icons, loaded once from the VFS (each 32x32 RGBA, or `None`
/// if the asset is missing/wrong-sized — the cell then draws a colored chip).
struct Icons {
    folder: Option<Vec<u8>>,
    text: Option<Vec<u8>>,
    exec: Option<Vec<u8>>,
    image: Option<Vec<u8>>,
    archive: Option<Vec<u8>>,
    unknown: Option<Vec<u8>>,
}

/// The file manager uses the same SVG decoder and legacy fallback as the shell.
fn read_theme_icon(theme: &mut Option<dunit_image::IconTheme>, name: &str) -> Option<Vec<u8>> {
    match theme.as_mut()?.icon(name, ICON as u32) {
        Ok(image) => Some(image.rgba()),
        Err(dunit_image::Error::Missing) => None,
        Err(error) => {
            log(&alloc::format!(
                "gui_files: image rejected {} {:?}",
                name,
                error
            ));
            None
        }
    }
}

impl Icons {
    /// Load the six mimetype icons from `theme` (already resolved: the
    /// `[files] icon_theme` override, else the shared `[desktop] icon_theme`).
    #[inline(never)]
    fn load(name: &str) -> Icons {
        let mut theme = match dunit_image::IconTheme::open(name) {
            Ok(theme) => Some(theme),
            Err(error) => {
                log(&alloc::format!(
                    "gui_files: icon theme rejected {} {:?}",
                    name,
                    error
                ));
                None
            }
        };
        let icons = Icons {
            folder: read_theme_icon(&mut theme, "mime_folder"),
            text: read_theme_icon(&mut theme, "mime_text"),
            exec: read_theme_icon(&mut theme, "mime_exec"),
            image: read_theme_icon(&mut theme, "mime_image"),
            archive: read_theme_icon(&mut theme, "mime_archive"),
            unknown: read_theme_icon(&mut theme, "mime_unknown"),
        };
        let count = [
            &icons.folder,
            &icons.text,
            &icons.exec,
            &icons.image,
            &icons.archive,
            &icons.unknown,
        ]
        .iter()
        .filter(|v| v.is_some())
        .count();
        log(&alloc::format!(
            "gui_files: icon assets loaded theme={} count={}",
            name,
            count
        ));
        icons
    }

    fn get(&self, k: IconKind) -> Option<&[u8]> {
        match k {
            IconKind::Folder => self.folder.as_deref(),
            IconKind::Text => self.text.as_deref(),
            IconKind::Exec => self.exec.as_deref(),
            IconKind::Image => self.image.as_deref(),
            IconKind::Archive => self.archive.as_deref(),
            IconKind::Unknown => self.unknown.as_deref(),
        }
    }
}

fn push_u32(d: &mut String, mut v: u32) {
    if v == 0 {
        d.push('0');
        return;
    }
    let mut tmp = [0u8; 10];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        d.push(tmp[n] as char);
    }
}

/// Paint the whole window: header breadcrumb, icon grid, status bar, and any
/// active overlay (context menu / text-entry / delete-confirm).
fn render(pixels: &mut [u32], font: &Font, files: &Files, icons: &Icons, mode: &Mode) {
    let w = files.w;
    let h = files.h;
    let cols = files.cols();
    let mut s = Surface::new(pixels, w as usize, h as usize);

    // Straight-alpha base: `clear` writes the configured bg (with its alpha)
    // over every pixel — unlike `fill_rect`, which would blend over and drift
    // the alpha of the previous frame, breaking a translucent background.
    s.clear(files.clear_bg);

    // Header bar + breadcrumb path.
    s.fill_rect(0.0, 0.0, w as f32, HEADER_H as f32, files.pal.header_bg);
    let crumb = fit_label(font, &files.path, 15.0, (w - 2 * PAD) as f32);
    draw_text(&mut s, font, PAD, 20, 15.0, &crumb, files.pal.accent);

    // Grid of cells: an optional ".." parent cell, then the entries.
    let has_parent = files.has_parent();
    let cell_count = files.items.len() + has_parent as usize;
    let start = files.scroll_row * cols as usize;
    for ci in start..cell_count.min(start + files.cap()) {
        let visible = ci - start;
        let col = (visible as i32) % cols;
        let row = (visible as i32) / cols;
        let cx = PAD + col * CELL_W;
        let cy = HEADER_H + row * CELL_H;

        let (label, kind, is_sel) = if has_parent && ci == 0 {
            (String::from(".."), IconKind::Folder, false)
        } else {
            let ii = ci - has_parent as usize;
            let it = &files.items[ii];
            (it.name.clone(), it.kind, files.selected(ii))
        };

        if is_sel {
            s.fill_rect(
                (cx + 4) as f32,
                (cy + 2) as f32,
                (CELL_W - 8) as f32,
                (CELL_H - 6) as f32,
                files.pal.sel_bg,
            );
        }

        // Icon centered horizontally near the top of the cell.
        let ix = cx + (CELL_W - ICON) / 2;
        let iy = cy + 6;
        match icons.get(kind) {
            Some(rgba) => s.blit_image(ix, iy, rgba, ICON as usize, ICON as usize),
            None => s.fill_rect(
                ix as f32,
                iy as f32,
                ICON as f32,
                ICON as f32,
                files.pal.muted,
            ),
        }

        // Label centered under the icon, ellipsized to the cell width.
        let px_lbl = 12.0;
        let text = fit_label(font, &label, px_lbl, (CELL_W - 8) as f32);
        let tw = text_width(font, &text, px_lbl);
        let tx = cx + ((CELL_W as f32 - tw) / 2.0) as i32;
        let baseline = iy + ICON + 16;
        draw_text(&mut s, font, tx, baseline, px_lbl, &text, files.pal.label);
    }

    // Status bar: entry count (+ hidden overflow) and the selected name.
    s.fill_rect(
        0.0,
        (h - STATUS_H) as f32,
        w as f32,
        STATUS_H as f32,
        files.pal.status_bg,
    );
    let mut status = String::new();
    push_u32(&mut status, files.total as u32);
    status.push_str(" items");
    if files.items.len() < files.total {
        status.push_str(" (");
        push_u32(&mut status, files.items.len() as u32);
        status.push_str(" shown)");
    }
    if let Some(ii) = files.current_index() {
        if ii < files.items.len() {
            status.push_str("  \u{2022}  ");
            status.push_str(&files.items[ii].name);
        }
    }
    if !files.status.is_empty() {
        status = files.status.clone();
    }
    if files.selection.selected.len() > 1 {
        status.push_str(&alloc::format!(
            "  [{} selected]",
            files.selection.selected.len()
        ));
    }
    let sb = fit_label(font, &status, 12.0, (w - 2 * PAD) as f32);
    draw_text(&mut s, font, PAD, h - 7, 12.0, &sb, files.pal.muted);

    // Overlays on top of the base view.
    match mode {
        Mode::Browse => {}
        Mode::Preview => {
            if let Some(preview) = &files.preview {
                s.clear(files.clear_bg);
                draw_text(&mut s, font, PAD, 22, 14.0, &preview.path, files.pal.accent);
                for (row, line) in preview
                    .text
                    .lines()
                    .skip(preview.row)
                    .take(((h - 60) / 18).max(0) as usize)
                    .enumerate()
                {
                    let shown = fit_label(font, line, 12.0, (w - 2 * PAD) as f32);
                    draw_text(
                        &mut s,
                        font,
                        PAD,
                        48 + row as i32 * 18,
                        12.0,
                        &shown,
                        files.pal.label,
                    );
                }
                draw_text(
                    &mut s,
                    font,
                    PAD,
                    h - 8,
                    11.0,
                    "Read-only preview  •  Ctrl+C copy text  •  Esc close",
                    files.pal.muted,
                );
            }
        }
        Mode::Text { .. } | Mode::Confirm => {}
    }
}

/// What a client-local pointer landed on in the grid.
enum Hit {
    Parent,
    Item(usize),
}

/// Map a client-local pointer `(lx, ly)` to a grid cell, if any.
fn hit_cell(files: &Files, lx: i32, ly: i32) -> Option<Hit> {
    if lx < PAD || ly < HEADER_H || ly >= files.h - STATUS_H {
        return None;
    }
    let cols = files.cols();
    let col = (lx - PAD) / CELL_W;
    let row = (ly - HEADER_H) / CELL_H;
    if col < 0 || col >= cols || row < 0 || row >= files.rows() {
        return None;
    }
    let ci = (row * cols + col) as usize + files.scroll_row * cols as usize;
    let has_parent = files.has_parent();
    if ci >= files.items.len() + has_parent as usize {
        return None;
    }
    if has_parent && ci == 0 {
        return Some(Hit::Parent);
    }
    Some(Hit::Item(ci - has_parent as usize))
}

/// The interaction mode: plain browsing, a
/// text-entry overlay (for New Folder / Rename), or a delete confirmation.
#[derive(Clone)]
enum Mode {
    Browse,
    Text { rename: bool, text: String },
    Confirm,
    Preview,
}

/// A context-menu command.
#[derive(Clone, Copy)]
enum Action {
    NewFolder,
    Rename,
    Delete,
    Copy,
    Cut,
    Paste,
}

/// The fixed context-menu rows, in display order.
const MENU_ACTIONS: [(Action, &str); 6] = [
    (Action::NewFolder, "New Folder"),
    (Action::Rename, "Rename"),
    (Action::Delete, "Delete"),
    (Action::Copy, "Copy"),
    (Action::Cut, "Cut"),
    (Action::Paste, "Paste"),
];

/// Whether `a` is applicable given the current selection / clipboard state.
fn action_enabled(action: Action, files: &Files) -> bool {
    match action {
        Action::NewFolder | Action::Paste => files.can_write(),
        Action::Rename => files.can_write() && files.single_index().is_some(),
        Action::Delete => files.can_write() && !files.selection.selected.is_empty(),
        Action::Copy => files.can_export(false),
        Action::Cut => files.can_export(true),
    }
}

#[derive(Clone)]
enum Message {
    Sync(Mode, [bool; 6]),
    Action(Action),
    Name(String),
    Confirm,
    Cancel,
    Consumed,
}
struct Controls {
    mode: Mode,
    enabled: [bool; 6],
    pending: Option<Message>,
}
impl Component for Controls {
    type Message = Message;
    fn update(&mut self, message: Message) {
        match message {
            Message::Sync(mode, enabled) => {
                self.mode = mode;
                self.enabled = enabled;
            }
            Message::Name(text) => {
                if let Mode::Text { text: current, .. } = &mut self.mode {
                    *current = text;
                }
            }
            Message::Consumed => self.pending = None,
            message => self.pending = Some(message),
        }
    }
    fn view(&self) -> View<Message> {
        let menu = MENU_ACTIONS
            .iter()
            .enumerate()
            .map(|(i, (a, label))| {
                MenuItem::action(alloc::format!("command{}", i), *label, Message::Action(*a))
                    .disabled(!self.enabled[i])
            })
            .collect();
        let background = View::stack("browser", alloc::vec![]);
        let background = if matches!(self.mode, Mode::Browse) {
            background.context_menu(menu)
        } else {
            background
        };
        let dialog = match &self.mode {
            Mode::Text { rename, text } => Some(View::dialog(
                "filename",
                if *rename {
                    "Rename to:"
                } else {
                    "New folder name:"
                },
                View::input(
                    "name",
                    "Filename",
                    text.clone(),
                    Message::Name,
                    InputOptions {
                        max_chars: 255,
                        ..Default::default()
                    },
                )
                .on_submit(Message::Confirm),
                alloc::vec![
                    DialogAction::new("ok", "OK", Message::Confirm),
                    DialogAction::new("cancel", "Cancel", Message::Cancel)
                ],
                None,
                Some(Message::Cancel),
            )),
            Mode::Confirm => Some(View::message_box(
                "delete",
                "Delete selected files?",
                "This removes the selected entries.",
                alloc::vec![
                    DialogAction::new("ok", "Delete", Message::Confirm),
                    DialogAction::new("cancel", "Cancel", Message::Cancel)
                ],
                Some("ok"),
                Some(Message::Cancel),
            )),
            _ => None,
        };
        View::modal("files", background, dialog)
    }
}
fn paint(window: &mut Window, files: &mut Files, font: &Font, icons: &Icons, mode: &Mode) {
    let (w, h) = window.size();
    files.w = w as i32;
    files.h = h as i32;
    files.clamp_scroll();
    let mut pixels = alloc::vec![0;w as usize*h as usize];
    render(&mut pixels, font, files, icons, mode);
    checked(window.set_canvas(pixels, w, h), "canvas frame");
    checked(
        window.update(Message::Sync(
            mode.clone(),
            core::array::from_fn(|i| action_enabled(MENU_ACTIONS[i].0, files)),
        )),
        "controls",
    );
}
#[inline(never)]
fn open_browser() -> dunit::Result<(Window, Files)> {
    // 0) Load this app's config (apps/gui_files.toml). A missing/garbage file
    // yields the built-in baseline. The resolved values below actually drive the
    // color scheme, icon theme and background transparency (config→behavior).
    let fcfg = dwm_settings::FilesCfg::load("gui_files");
    log(&alloc::format!(
        "gui_files: cfg from_file={} icon_theme={} bg={:#010X} accent={:#010X} bg_alpha={}",
        fcfg.from_file as u32,
        if fcfg.icon_theme.as_str().is_empty() {
            "<desktop>"
        } else {
            fcfg.icon_theme.as_str()
        },
        fcfg.bg,
        fcfg.accent,
        fcfg.bg_alpha,
    ));

    // Resolve the color scheme. Chrome colors are forced opaque; the base clear
    // carries the configured `bg_alpha` (a value < 255 makes the window
    // translucent, so ImportBuffer must advertise ARGB rather than XRGB).
    let pal = Palette {
        header_bg: col(fcfg.header_bg | 0xFF00_0000),
        status_bg: col(fcfg.status_bg | 0xFF00_0000),
        accent: col(fcfg.accent | 0xFF00_0000),
        label: col(fcfg.label | 0xFF00_0000),
        muted: col(fcfg.muted | 0xFF00_0000),
        sel_bg: col(fcfg.sel_bg),
    };
    let clear_bg = col((fcfg.bg_alpha << 24) | (fcfg.bg & 0x00FF_FFFF));
    let fmt = if fcfg.bg_alpha < 255 {
        PixelFormat::Argb8888
    } else {
        PixelFormat::Xrgb8888
    };
    let mut files = Files::new(pal, clear_bg);
    let args = dunit::args::args()?;
    if let Some(path) = args.get(1).map(|s| s.as_str()) {
        match dunit::transfer::File::open(path, true)
            .or_else(|error| {
                if matches!(error.0, dunit::errno::EPERM | dunit::errno::EACCES) {
                    dunit::transfer::File::open(path, false)
                } else {
                    Err(error)
                }
            })
            .and_then(|file| files.open_initial(file))
        {
            Ok(()) => {}
            Err(error) => files.error("Initial path", error),
        }
    }
    let (theme,ui_font)=dunit::ui::load_theme("Stack#browser { width: fill; height: fill; background: #00000000; } Stack#files { width: fill; height: fill; background: #00000000; } Input#name { height: 30; }")?;
    let window = ComponentWindow::open_format(
        W,
        H,
        Controls {
            mode: Mode::Browse,
            enabled: [false; 6],
            pending: None,
        },
        Renderer::new(ui_font, theme.cascade, Color::rgba(0, 0, 0, 0)),
        fmt,
    )?;
    Ok((window, files))
}
#[inline(never)]
fn configured_icon_theme() -> String {
    let cfg = dwm_settings::FilesCfg::load("gui_files");
    let theme = if cfg.icon_theme.as_str().is_empty() {
        dwm_settings::load().desktop.icon_theme
    } else {
        cfg.icon_theme
    };
    String::from(theme.as_str())
}
#[inline(never)]
fn canvas_resources() -> dunit::Result<(Font, Icons)> {
    let theme = configured_icon_theme();
    let font = load_font()?;
    let icons = Icons::load(theme.as_str());
    Ok((font, icons))
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let (font, icons) = canvas_resources()?;
    let (mut window, mut files) = open_browser()?;
    let mut mode = Mode::Browse;
    paint(&mut window, &mut files, &font, &icons, &mode);
    log("gui_files: surface presented OK");
    let mut transfers = file_transfer::Transfers::start();
    checked(
        checked(window.transfers(), "opener client")
            .register_opener(dunit::transfer::Offer {
                formats: alloc::vec![dunit::transfer::Format::Files],
                actions: 1,
                preferred: dunit::transfer::Action::Copy,
            })
            .map_err(|e| dunit::Error(e.0)),
        "register opener",
    );
    let mut queued_event = None;
    let mut pending_repaint = false;
    let mut batch = 0;
    loop {
        checked(
            transfers.sync(&mut window, &mode, &files),
            "drop target registration",
        );
        let event = if let Some(event) = queued_event.take() {
            Some(event)
        } else {
            checked(
                window.next_event(dunit::time::Duration::from_millis(if transfers.copying() {
                    0
                } else {
                    40
                })),
                "window event",
            )
        };
        let mut dirty = false;
        match event {
            Some(WindowEvent::CloseRequested) => break,
            Some(WindowEvent::Resized { .. }) => {
                transfers.cancel_candidate();
                dirty = true;
                log("gui_files: reconfigured OK");
            }
            Some(WindowEvent::Transfer(mut event)) => {
                if event.message.kind == dunit::transfer::Kind::OpenData {
                    let result = event
                        .payload
                        .take()
                        .ok_or(dunit::transfer::Error(dunit::errno::EINVAL))
                        .and_then(|payload| files.open_received(payload.files));
                    let status = if result.is_ok() {
                        dunit::transfer::Status::Ok
                    } else {
                        dunit::transfer::Status::Failed
                    };
                    if let Err(error) = result {
                        files.error("Open document", error);
                    }
                    checked(
                        checked(window.transfers(), "opener client")
                            .complete(event.message.transfer, status)
                            .map_err(|e| dunit::Error(e.0)),
                        "complete open",
                    );
                    mode = if files.preview.is_some() {
                        Mode::Preview
                    } else {
                        Mode::Browse
                    };
                    dirty = true;
                }
                let status = files.status.clone();
                let generation = files.generation;
                let repaint = !matches!(
                    event.message.kind,
                    dunit::transfer::Kind::DragMotion
                        | dunit::transfer::Kind::DragEnter
                        | dunit::transfer::Kind::DragLeave
                );
                checked(
                    transfers.event(&mut window, &mut files, event),
                    "transfer event",
                );
                dirty |= repaint || status != files.status || generation != files.generation;
            }
            Some(WindowEvent::Input(input)) => {
                let kind = input.kind;
                let (x, y) = (input.x, input.y);
                let mods = input.y as u8;
                let ascii = input.value as u8;
                if kind == IN_FOCUS_IN || kind == IN_FOCUS_OUT {
                    transfers.focus(kind == IN_FOCUS_IN);
                } else if kind == IN_LEAVE {
                    transfers.cancel_candidate();
                } else if kind == IN_SCROLL && matches!(mode, Mode::Preview) {
                    if let Some(preview) = &mut files.preview {
                        preview.row = (preview.row as i64 + i64::from(input.x))
                            .clamp(0, preview.text.lines().count().saturating_sub(1) as i64)
                            as usize;
                    }
                    dirty = true;
                } else if kind == IN_SCROLL && matches!(mode, Mode::Browse) {
                    files.scroll(input.x);
                    dirty = true;
                } else if kind == IN_MOVE && matches!(mode, Mode::Browse) {
                    transfers.motion(&mut window, &mut files, x, y);
                } else if kind == IN_UP
                    && input.pointer_button() == 0
                    && matches!(mode, Mode::Browse)
                {
                    if let Some((was_selected, index)) = transfers.release() {
                        if was_selected
                            && files.selection.selected.len() == 1
                            && input.pointer_modifiers() == 0
                        {
                            files.open(index);
                            dirty = true;
                        }
                    }
                } else if kind == IN_DOWN {
                    if matches!(mode, Mode::Browse)
                        && (input.pointer_button() == 1 || window.popup_state().is_none())
                    {
                        if input.pointer_button() == 1 {
                            if let Some(Hit::Item(index)) = hit_cell(&files, x, y) {
                                if !files.selected(index) {
                                    files.choose(index, 0);
                                }
                            }
                            dirty = true;
                        } else if input.pointer_button() == 0 {
                            match hit_cell(&files, x, y) {
                                Some(Hit::Parent) => {
                                    files.go_parent();
                                    dirty = true;
                                }
                                Some(Hit::Item(index)) => {
                                    let was_selected = files.selected(index);
                                    let modifiers = input.pointer_modifiers();
                                    if !was_selected || modifiers != 0 {
                                        files.choose(index, modifiers);
                                    }
                                    transfers.press(
                                        &files,
                                        index,
                                        x,
                                        y,
                                        was_selected && modifiers == 0,
                                    );
                                    dirty = true;
                                }
                                None => {
                                    files.clear_selection();
                                    dirty = true;
                                }
                            }
                        }
                    }
                } else if kind == IN_KEY {
                    let mut transition = None;
                    match &mut mode {
                        Mode::Text { .. } => {}
                        Mode::Preview => {
                            if ascii == KEY_ESC {
                                files.preview = None;
                                transition = Some(Mode::Browse);
                            } else if mods & dunit::window::KEYMOD_CTRL != 0 && input.x == 0x2e {
                                if let Some(preview) = &files.preview {
                                    let payload = checked(
                                        dunit::transfer::Payload::text(&preview.text)
                                            .map_err(|e| dunit::Error(e.0)),
                                        "preview text",
                                    );
                                    checked(
                                        checked(window.transfers(), "clipboard client")
                                            .publish_clipboard(
                                                payload,
                                                1,
                                                dunit::transfer::Action::Copy,
                                            )
                                            .map_err(|e| dunit::Error(e.0)),
                                        "copy preview",
                                    );
                                }
                            }
                        }
                        Mode::Confirm => {}
                        Mode::Browse => {
                            let ctrl = mods & dunit::window::KEYMOD_CTRL != 0;
                            if ctrl
                                && mods
                                    & !(dunit::window::KEYMOD_CTRL | dunit::window::KEYMOD_SHIFT)
                                    == 0
                            {
                                match input.x {
                                    0x2e => transfers.publish(&mut window, &mut files, false),
                                    0x2d => transfers.publish(&mut window, &mut files, true),
                                    0x2f => transfers.paste(&mut window, &mut files),
                                    0x1e => files.select_all(),
                                    _ => {}
                                }
                                dirty = true;
                            } else if ascii == KEY_ESC {
                                if !checked(
                                    transfers.cancel_operation(&mut window, &mut files),
                                    "cancel transfer",
                                ) {
                                    files.clear_selection();
                                }
                                dirty = true;
                            } else if matches!(
                                input.x,
                                0x47 | 0x4f | 0x48 | 0x50 | 0x4b | 0x4d | 0x49 | 0x51
                            ) && !files.items.is_empty()
                            {
                                let old = files.current_index().unwrap_or(0) as i64;
                                let cols = i64::from(files.cols());
                                let next = match input.x {
                                    0x47 => 0,
                                    0x4f => files.items.len() as i64 - 1,
                                    0x48 => old - cols,
                                    0x50 => old + cols,
                                    0x4b => old - 1,
                                    0x4d => old + 1,
                                    0x49 => old - cols * i64::from(files.rows()),
                                    _ => old + cols * i64::from(files.rows()),
                                };
                                files.choose(
                                    next.clamp(0, files.items.len() as i64 - 1) as usize,
                                    mods,
                                );
                                dirty = true;
                            } else if ascii == KEY_ENTER_LF || ascii == KEY_ENTER_CR {
                                if let Some(index) = files.current_index() {
                                    files.open(index);
                                    dirty = true;
                                }
                            }
                        }
                    }
                    if let Some(next) = transition {
                        mode = next;
                        dirty = true;
                    }
                }
            }
            _ => {}
        }
        if let Some(message) = window.component().pending.clone() {
            if let Mode::Text { text, .. } = &mut mode {
                if let Mode::Text { text: proposed, .. } = &window.component().mode {
                    *text = proposed.clone();
                }
            }
            match message {
                Message::Action(action) if action_enabled(action, &files) => match action {
                    Action::NewFolder => {
                        mode = Mode::Text {
                            rename: false,
                            text: String::new(),
                        }
                    }
                    Action::Rename => {
                        mode = Mode::Text {
                            rename: true,
                            text: files
                                .single_index()
                                .map(|i| files.items[i].name.clone())
                                .unwrap_or_default(),
                        }
                    }
                    Action::Delete => mode = Mode::Confirm,
                    Action::Copy => transfers.publish(&mut window, &mut files, false),
                    Action::Cut => transfers.publish(&mut window, &mut files, true),
                    Action::Paste => transfers.paste(&mut window, &mut files),
                },
                Message::Confirm => match &mode {
                    Mode::Text { rename, text } => {
                        if if *rename {
                            files.rename_selected(text)
                        } else {
                            files.make_dir(text)
                        } {
                            mode = Mode::Browse;
                        }
                    }
                    Mode::Confirm => {
                        files.delete_selected();
                        mode = Mode::Browse;
                    }
                    _ => {}
                },
                Message::Cancel => mode = Mode::Browse,
                _ => {}
            }
            checked(window.update(Message::Consumed), "consume action");
            dirty = true;
        } else if let (Mode::Text { text, .. }, Mode::Text { text: proposed, .. }) =
            (&mut mode, &window.component().mode)
        {
            *text = proposed.clone();
        }
        if files.preview.is_some() && matches!(mode, Mode::Browse) {
            mode = Mode::Preview;
        }
        dirty |= checked(transfers.service(&mut window, &mut files), "copy service");
        pending_repaint |= dirty;
        if pending_repaint && batch < 64 {
            if let Some(event) = checked(
                window.next_event(dunit::time::Duration::from_millis(0)),
                "queued window event",
            ) {
                queued_event = Some(event);
                batch += 1;
                continue;
            }
        }
        if pending_repaint {
            paint(&mut window, &mut files, &font, &icons, &mode);
            pending_repaint = false;
        }
        batch = 0;
    }
    window.close()
}
