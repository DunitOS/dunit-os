#![no_std]
#![no_main]

//! Dolphin-style file-manager client for the M4 userspace DWM (Stack B).
//!
//! Reborn from the plain text-list `gui_files` into an icon-grid browser: a
//! breadcrumb path header, a grid of per-type Breeze-Chameleon icons with TTF
//! labels, click-to-select / click-again-to-open navigation, and a status bar.
//! It lists the VFS with `libdunit::readdir` and paints its own shared buffer
//! directly with `dunit_render::Surface` primitives (fills, `blit_image` for
//! icons, `blit_glyph` for text) — the compositor blits that buffer each tick.

use core::panic::PanicInfo;

extern crate alloc;
use gui_protocol_v1::control::{IN_DOWN,IN_UP,IN_MOVE,IN_LEAVE,IN_KEY,IN_SCROLL,IN_FOCUS_IN,IN_FOCUS_OUT};
use dunit::window::{Window,WindowEvent,PixelFormat};
mod file_model;
use file_model::Files;
mod file_transfer;
mod filename_editor;
mod filename_transfer;
use filename_editor::Editor;

use alloc::string::String;
use alloc::vec::Vec;



use dunit_render::Surface;
use dunit_style::value::Color;
use dunit_text::Font;

// Cooked key bytes the compositor forwards in the IN_KEY `button` field.
const KEY_ENTER_LF: u8 = 0x0a;
const KEY_ENTER_CR: u8 = 0x0d;
const KEY_ESC: u8 = 0x1b;

const W:u32=520;
const H:u32=380;

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
    /// Opaque base fill used for dialog input fields. The window's per-frame base
    /// clear carries the configured `bg_alpha` separately (see `Files::clear_bg`).
    bg: Color,
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
fn load_font() -> Result<Font, ()> {
    let cfg = dwm_settings::load();
    if let Some(bytes) = libdunit::read_binary(cfg.desktop.font.as_str(), 4 * 1024 * 1024) {
        if let Ok(f) = Font::parse(bytes) {
            return Ok(f);
        }
    }
    libdunit::read_binary("/assets/fonts/DejaVuSans.ttf", 4 * 1024 * 1024)
        .ok_or(())
        .and_then(|b| Font::parse(b).map_err(|_| ()))
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::println("gui_files: PANIC");
    libdunit::exit(101)
}

fn fatal(why:&str,error:isize)->!{libdunit::println(&alloc::format!("gui_files: FAIL {} ({})",why,error));libdunit::exit(1)}
fn checked<T>(result:dunit::Result<T>,why:&str)->T{result.unwrap_or_else(|e|fatal(why,e.0))}

fn round_i32(v: f32) -> i32 {
    if v <= 0.0 {
        0
    } else {
        (v + 0.5) as i32
    }
}

/// Total advance width of `text` at `px`.
fn text_width(font: &Font, text: &str, px: f32) -> f32 {
    font.layout_line(text, px).1
}

/// Draw one line of `text` with its left edge at `x` and baseline at `baseline`.
fn draw_text(s: &mut Surface, font: &Font, x: i32, baseline: i32, px: f32, text: &str, color: Color) {
    let (glyphs, _adv) = font.layout_line(text, px);
    for g in glyphs {
        if let Some(bmp) = font.rasterize(g.glyph, px) {
            let ox = x + round_i32(g.x) + bmp.left;
            let oy = baseline - bmp.top;
            s.blit_glyph(&bmp, ox, oy, color);
        }
    }
}


/// Shorten `name` with a trailing ellipsis so it fits within `max_w` px.
fn fit_label(font: &Font, name: &str, px: f32, max_w: f32) -> String {
    if text_width(font, name, px) <= max_w {
        return String::from(name);
    }
    let mut out = String::new();
    for c in name.chars() {
        let mut trial = out.clone();
        trial.push(c);
        trial.push('…');
        if text_width(font, &trial, px) > max_w {
            out.push('…');
            return out;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// A file-type bucket, each mapped to one Breeze mimetype icon.
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
        "txt" | "toml" | "md" | "rs" | "cfg" | "conf" | "log" | "json" | "c" | "h" => IconKind::Text,
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
            libdunit::println(&alloc::format!("gui_files: image rejected {} {:?}", name, error));
            None
        }
    }
}

impl Icons {
    /// Load the six mimetype icons from `theme` (already resolved: the
    /// `[files] icon_theme` override, else the shared `[desktop] icon_theme`).
    fn load(name: &str) -> Icons {
        let mut theme = match dunit_image::IconTheme::open(name) {
            Ok(theme) => Some(theme),
            Err(error) => {libdunit::println(&alloc::format!("gui_files: icon theme rejected {} {:?}",name,error));None}
        };
        let icons = Icons {
            folder: read_theme_icon(&mut theme, "mime_folder"),
            text: read_theme_icon(&mut theme, "mime_text"),
            exec: read_theme_icon(&mut theme, "mime_exec"),
            image: read_theme_icon(&mut theme, "mime_image"),
            archive: read_theme_icon(&mut theme, "mime_archive"),
            unknown: read_theme_icon(&mut theme, "mime_unknown"),
        };
        let count = [&icons.folder, &icons.text, &icons.exec, &icons.image, &icons.archive, &icons.unknown].iter().filter(|v|v.is_some()).count();
        libdunit::println(&alloc::format!("gui_files: icon assets loaded theme={} count={}",name,count));
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
fn render(pixels:&mut[u32], font: &Font, files: &Files, icons: &Icons, mode: &Mode) {
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
    let start=files.scroll_row*cols as usize;
    for ci in start..cell_count.min(start+files.cap()) {
        let visible=ci-start;
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
            s.fill_rect((cx + 4) as f32, (cy + 2) as f32, (CELL_W - 8) as f32, (CELL_H - 6) as f32, files.pal.sel_bg);
        }

        // Icon centered horizontally near the top of the cell.
        let ix = cx + (CELL_W - ICON) / 2;
        let iy = cy + 6;
        match icons.get(kind) {
            Some(rgba) => s.blit_image(ix, iy, rgba, ICON as usize, ICON as usize),
            None => s.fill_rect(ix as f32, iy as f32, ICON as f32, ICON as f32, files.pal.muted),
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
    s.fill_rect(0.0, (h - STATUS_H) as f32, w as f32, STATUS_H as f32, files.pal.status_bg);
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
    if !files.status.is_empty(){status=files.status.clone();}
    if files.selection.selected.len()>1{status.push_str(&alloc::format!("  [{} selected]",files.selection.selected.len()));}
    let sb = fit_label(font, &status, 12.0, (w - 2 * PAD) as f32);
    draw_text(&mut s, font, PAD, h - 7, 12.0, &sb, files.pal.muted);

    // Overlays on top of the base view.
    match mode {
        Mode::Browse => {}
        Mode::Preview=>{if let Some(preview)=&files.preview{
            s.clear(files.clear_bg);draw_text(&mut s,font,PAD,22,14.0,&preview.path,files.pal.accent);
            for (row,line) in preview.text.lines().skip(preview.row).take(((h-60)/18).max(0)as usize).enumerate(){
                let shown=fit_label(font,line,12.0,(w-2*PAD)as f32);draw_text(&mut s,font,PAD,48+row as i32*18,12.0,&shown,files.pal.label);
            }
            draw_text(&mut s,font,PAD,h-8,11.0,"Read-only preview  •  Ctrl+C copy text  •  Esc close",files.pal.muted);
        }},
        Mode::Menu { mx, my } => draw_menu(&mut s, font, files, *mx, *my),
        Mode::Text { rename, editor } => draw_text_box(&mut s, font, &files.pal, w, h, *rename, editor),
        Mode::Confirm => draw_confirm(&mut s, font, files),
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
    let ci = (row * cols + col) as usize+files.scroll_row*cols as usize;
    let has_parent = files.has_parent();
    if ci >= files.items.len() + has_parent as usize {
        return None;
    }
    if has_parent && ci == 0 {
        return Some(Hit::Parent);
    }
    Some(Hit::Item(ci - has_parent as usize))
}


/// The interaction mode: plain browsing, a context menu at `(mx, my)`, a
/// text-entry overlay (for New Folder / Rename), or a delete confirmation.
enum Mode {
    Browse,
    Menu { mx: i32, my: i32 },
    Text { rename: bool, editor: Editor },
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

// --- Overlay geometry ---
const MENU_W: i32 = 140;
const MENU_ROW_H: i32 = 24;
const MENU_H: i32 = MENU_ROW_H * MENU_ACTIONS.len() as i32;
const DLG_W: i32 = 320;
const DLG_H: i32 = 96;

/// Whether `a` is applicable given the current selection / clipboard state.
fn action_enabled(action:Action,files:&Files)->bool{
    match action{
        Action::NewFolder|Action::Paste=>files.can_write(),
        Action::Rename=>files.can_write()&&files.single_index().is_some(),
        Action::Delete=>files.can_write()&&!files.selection.selected.is_empty(),
        Action::Copy=>files.can_export(false),Action::Cut=>files.can_export(true),
    }
}

/// Clamp a menu opened at `(mx, my)` so it stays fully inside a `w`×`h` window.
fn menu_origin(w: i32, h: i32, mx: i32, my: i32) -> (i32, i32) {
    let ox = mx.min(w - MENU_W - 2).max(2);
    let oy = my.min(h - MENU_H - 2).max(2);
    (ox, oy)
}

/// The menu row a client-local pointer `(lx, ly)` landed on, if inside the menu.
fn menu_hit(w: i32, h: i32, mx: i32, my: i32, lx: i32, ly: i32) -> Option<usize> {
    let (ox, oy) = menu_origin(w, h, mx, my);
    if lx < ox || lx >= ox + MENU_W || ly < oy || ly >= oy + MENU_H {
        return None;
    }
    let row = ((ly - oy) / MENU_ROW_H) as usize;
    if row < MENU_ACTIONS.len() {
        Some(row)
    } else {
        None
    }
}

/// Draw the context menu at its clamped origin.
fn draw_menu(s: &mut Surface, font: &Font, files: &Files, mx: i32, my: i32) {
    let (ox, oy) = menu_origin(files.w, files.h, mx, my);
    s.fill_rect(ox as f32, oy as f32, MENU_W as f32, MENU_H as f32, files.pal.header_bg);
    s.stroke_rect(ox as f32, oy as f32, MENU_W as f32, MENU_H as f32, 1.0, files.pal.accent);
    for (i, (action, label)) in MENU_ACTIONS.iter().enumerate() {
        let ry = oy + i as i32 * MENU_ROW_H;
        let color = if action_enabled(*action, files) { files.pal.label } else { files.pal.muted };
        draw_text(s, font, ox + 10, ry + MENU_ROW_H - 8, 13.0, label, color);
    }
}

/// Draw a centered dialog box (bg + accent border) in a `w`×`h` window and
/// return its origin.
fn draw_dialog(s: &mut Surface, pal: &Palette, w: i32, h: i32) -> (i32, i32) {
    let ox = (w - DLG_W) / 2;
    let oy = (h - DLG_H) / 2;
    s.fill_rect(0.0, 0.0, w as f32, h as f32, Color::rgba(0, 0, 0, 0x70)); // scrim
    s.fill_rect(ox as f32, oy as f32, DLG_W as f32, DLG_H as f32, pal.header_bg);
    s.stroke_rect(ox as f32, oy as f32, DLG_W as f32, DLG_H as f32, 1.0, pal.accent);
    (ox, oy)
}

/// Draw the text-entry overlay (New Folder / Rename) with the current buffer.
fn draw_text_box(s: &mut Surface, font: &Font, pal: &Palette, w: i32, h: i32, rename: bool, editor: &Editor) {
    let (ox, oy) = draw_dialog(s, pal, w, h);
    let title = if rename { "Rename to:" } else { "New folder name:" };
    draw_text(s, font, ox + 14, oy + 26, 14.0, title, pal.accent);
    // Input field.
    let fx = ox + 14;
    let fy = oy + 38;
    let fw = DLG_W - 28;
    s.fill_rect(fx as f32, fy as f32, fw as f32, 22.0, pal.bg);
    s.stroke_rect(fx as f32, fy as f32, fw as f32, 22.0, 1.0, pal.muted);
    let shown = fit_label(font, editor.text(), 13.0, (fw - 12) as f32);
    let selection=editor.selection();
    if !selection.is_empty(){
        let left=text_width(font,&editor.text()[..selection.start],13.0).min((fw-12)as f32);
        let right=text_width(font,&editor.text()[..selection.end],13.0).min((fw-12)as f32);
        s.fill_rect((fx+6)as f32+left,(fy+3)as f32,(right-left).max(0.0),16.0,pal.sel_bg);
    }
    draw_text(s, font, fx + 6, fy + 16, 13.0, &shown, pal.label);
    // Caret after the text.
    let cw = text_width(font, &editor.text()[..editor.cursor()], 13.0).min((fw - 12) as f32);
    let cx = fx + 6 + round_i32(cw);
    s.fill_rect(cx as f32, (fy + 4) as f32, 1.0, 14.0, pal.label);
    draw_text(s, font, ox + 14, oy + DLG_H - 10, 11.0, "[Enter] ok   [Esc] cancel", pal.muted);
}

/// Draw the delete-confirmation overlay for the selected entry.
fn draw_confirm(s: &mut Surface, font: &Font, files: &Files) {
    let (ox, oy) = draw_dialog(s, &files.pal, files.w, files.h);
    draw_text(s, font, ox + 14, oy + 26, 14.0, "Delete this file?", files.pal.accent);
    let name = files.current_index()
        .and_then(|i| files.items.get(i))
        .map(|it| it.name.as_str())
        .unwrap_or("");
    let shown = fit_label(font, name, 13.0, (DLG_W - 28) as f32);
    draw_text(s, font, ox + 14, oy + 50, 13.0, &shown, files.pal.label);
    draw_text(s, font, ox + 14, oy + DLG_H - 10, 11.0, "[Enter] delete   [Esc] cancel", files.pal.muted);
}


fn paint(window:&mut Window,files:&mut Files,font:&Font,icons:&Icons,mode:&Mode){
    checked(window.draw(|frame|{
        let (w,h)=frame.size();files.w=w as i32;files.h=h as i32;files.clamp_scroll();
        render(frame.pixels_mut(),font,files,icons,mode);
    }),"present frame");
}

#[no_mangle]
pub extern "C" fn _start(argc:usize,argv:libdunit::RawArgv,envp:libdunit::RawEnvp) -> ! {
    libdunit::init_runtime(argc,argv,envp);
    // 0) Load this app's config (apps/gui_files.toml). A missing/garbage file
    // yields the built-in baseline. The resolved values below actually drive the
    // color scheme, icon theme and background transparency (config→behavior).
    let fcfg = dwm_settings::FilesCfg::load("gui_files");
    libdunit::println(&alloc::format!(
        "gui_files: cfg from_file={} icon_theme={} bg={:#010X} accent={:#010X} bg_alpha={}",
        fcfg.from_file as u32,
        if fcfg.icon_theme.as_str().is_empty() { "<desktop>" } else { fcfg.icon_theme.as_str() },
        fcfg.bg,
        fcfg.accent,
        fcfg.bg_alpha,
    ));

    // Resolve the color scheme. Chrome colors are forced opaque; the base clear
    // carries the configured `bg_alpha` (a value < 255 makes the window
    // translucent, so ImportBuffer must advertise ARGB rather than XRGB).
    let pal = Palette {
        bg: col(fcfg.bg | 0xFF00_0000),
        header_bg: col(fcfg.header_bg | 0xFF00_0000),
        status_bg: col(fcfg.status_bg | 0xFF00_0000),
        accent: col(fcfg.accent | 0xFF00_0000),
        label: col(fcfg.label | 0xFF00_0000),
        muted: col(fcfg.muted | 0xFF00_0000),
        sel_bg: col(fcfg.sel_bg),
    };
    let clear_bg = col((fcfg.bg_alpha << 24) | (fcfg.bg & 0x00FF_FFFF));
    let fmt = if fcfg.bg_alpha < 255 { PixelFormat::Argb8888 } else { PixelFormat::Xrgb8888 };
    // Icon theme: the per-app `[files] icon_theme` override, else the shared
    // `[desktop] icon_theme`; the shared loader resolves the configured source.
    let icon_theme = if fcfg.icon_theme.as_str().is_empty() {
        dwm_settings::load().desktop.icon_theme
    } else {
        fcfg.icon_theme
    };

    let font=load_font().unwrap_or_else(|_|fatal("font parse",libdunit::EINVAL));
    let icons=Icons::load(icon_theme.as_str());
    let mut files=Files::new(pal,clear_bg);
    if let Some(path)=libdunit::arg(1){
        match dunit::transfer::File::open(path,true).or_else(|error|{
            if matches!(error.0,libdunit::EPERM|libdunit::EACCES){dunit::transfer::File::open(path,false)}else{Err(error)}
        }).and_then(|file|files.open_initial(file)){
            Ok(())=>{},Err(error)=>files.error("Initial path",error),
        }
    }
    let mut window=checked(Window::open_format(W,H,fmt),"window handshake");
    let mut mode=Mode::Browse;
    let mut text_revision=0u64;
    let mut clipboard=dunit::text_clipboard::Session::<filename_editor::Snapshot>::default();
    paint(&mut window,&mut files,&font,&icons,&mode);
    libdunit::println("gui_files: surface presented OK");
    let mut transfers=file_transfer::Transfers::start();
    checked(checked(window.transfers(),"opener client").register_opener(dunit::transfer::Offer{formats:alloc::vec![dunit::transfer::Format::Files],actions:1,preferred:dunit::transfer::Action::Copy}).map_err(|e|dunit::Error(e.0)),"register opener");
    let mut filename_drag=filename_transfer::Transfers::default();
    let mut queued_event=None;let mut pending_repaint=false;let mut batch=0;
    loop {
        checked(transfers.sync(&mut window,&mode,&files),"drop target registration");
        let event=if let Some(event)=queued_event.take(){Some(event)}else{checked(window.next_event(dunit::time::Duration::from_millis(if transfers.copying(){0}else{40})),"window event")};
        let mut dirty=false;
        match event {
            Some(WindowEvent::CloseRequested)=>break,
            Some(WindowEvent::Resized{..})=>{transfers.cancel_candidate();dirty=true;libdunit::println("gui_files: reconfigured OK");},
            Some(WindowEvent::Transfer(mut event))=>{
                if event.message.kind==dunit::transfer::Kind::OpenData{
                    let result=event.payload.take().ok_or(dunit::transfer::Error(libdunit::EINVAL)).and_then(|payload|files.open_received(payload.files));
                    let status=if result.is_ok(){dunit::transfer::Status::Ok}else{dunit::transfer::Status::Failed};
                    if let Err(error)=result{files.error("Open document",error);}
                    checked(checked(window.transfers(),"opener client").complete(event.message.transfer,status).map_err(|e|dunit::Error(e.0)),"complete open");
                    mode=if files.preview.is_some(){Mode::Preview}else{Mode::Browse};dirty=true;
                }
                dirty|=checked(clipboard.event(checked(window.transfers(),"clipboard client"),&event,|snapshot,text|{
                    Ok(match &mut mode{Mode::Text{editor,..} if snapshot.current(editor,text_revision)=>editor.insert(text),_=>false})
                }),"filename clipboard");
                if dirty{text_revision=text_revision.wrapping_add(1);}
                dirty|=checked(filename_drag.event(&mut window,&mut mode,text_revision,&files,&font,&event),"filename drop");
                let status=files.status.clone();let generation=files.generation;
                let repaint=!matches!(event.message.kind,dunit::transfer::Kind::DragMotion|dunit::transfer::Kind::DragEnter|dunit::transfer::Kind::DragLeave);
                checked(transfers.event(&mut window,&mut files,event),"transfer event");
                dirty|=repaint||status!=files.status||generation!=files.generation;
            },
            Some(WindowEvent::Input(input))=>{
                let kind=input.kind;let (x,y)=(input.x,input.y);let mods=input.y as u8;let ascii=input.value as u8;
                if checked(filename_drag.pointer(&mut window,&mut mode,&files,&font,&input,text_revision),"filename pointer"){
                    text_revision=text_revision.wrapping_add(1);paint(&mut window,&mut files,&font,&icons,&mode);continue;
                }
                if kind==IN_FOCUS_IN||kind==IN_FOCUS_OUT{transfers.focus(kind==IN_FOCUS_IN);text_revision=text_revision.wrapping_add(1);}
                else if kind==IN_LEAVE{transfers.cancel_candidate();}
                else if kind==IN_SCROLL&&matches!(mode,Mode::Preview){if let Some(preview)=&mut files.preview{preview.row=(preview.row as i64+i64::from(input.x)).clamp(0,preview.text.lines().count().saturating_sub(1)as i64)as usize;}dirty=true;}
                else if kind==IN_SCROLL&&matches!(mode,Mode::Browse){files.scroll(input.x);dirty=true;}
                else if kind==IN_MOVE&&matches!(mode,Mode::Browse){transfers.motion(&mut window,&mut files,x,y);}
                else if kind==IN_UP&&input.pointer_button()==0&&matches!(mode,Mode::Browse){
                    if let Some((was_selected,index))=transfers.release(){if was_selected&&files.selection.selected.len()==1&&input.pointer_modifiers()==0{files.open(index);dirty=true;}}
                }
                else if kind==IN_DOWN{
                    if let Mode::Menu{mx,my}=&mode{
                        let action=menu_hit(files.w,files.h,*mx,*my,x,y).map(|row|MENU_ACTIONS[row].0);mode=Mode::Browse;
                        if let Some(action)=action.filter(|a|action_enabled(*a,&files)){
                            match action{
                                Action::NewFolder=>mode=Mode::Text{rename:false,editor:Editor::new(String::new())},
                                Action::Rename=>mode=Mode::Text{rename:true,editor:Editor::new(files.single_index().map(|i|files.items[i].name.clone()).unwrap_or_default())},
                                Action::Delete=>mode=Mode::Confirm,
                                Action::Copy=>transfers.publish(&mut window,&mut files,false),
                                Action::Cut=>transfers.publish(&mut window,&mut files,true),
                                Action::Paste=>transfers.paste(&mut window,&mut files),
                            }
                        }text_revision=text_revision.wrapping_add(1);dirty=true;
                    }else if matches!(mode,Mode::Browse){
                        if input.pointer_button()==1{
                            if let Some(Hit::Item(index))=hit_cell(&files,x,y){if !files.selected(index){files.choose(index,0);}}
                            mode=Mode::Menu{mx:x,my:y};dirty=true;
                        }else if input.pointer_button()==0{
                            match hit_cell(&files,x,y){
                                Some(Hit::Parent)=>{files.go_parent();dirty=true;},
                                Some(Hit::Item(index))=>{
                                    let was_selected=files.selected(index);let modifiers=input.pointer_modifiers();
                                    if !was_selected||modifiers!=0{files.choose(index,modifiers);}
                                    transfers.press(&files,index,x,y,was_selected&&modifiers==0);dirty=true;
                                },
                                None=>{files.clear_selection();dirty=true;},
                            }
                        }
                    }
                }
                else if kind==IN_KEY{
                    let mut transition=None;
                    match &mut mode{
                        Mode::Text{rename,editor}=>{
                            if ascii==KEY_ESC{transition=Some(Mode::Browse);}
                            else if ascii==KEY_ENTER_LF||ascii==KEY_ENTER_CR{
                                let succeeded=if *rename{files.rename_selected(editor.text())}else{files.make_dir(editor.text())};
                                if succeeded{transition=Some(Mode::Browse);}dirty=true;
                            }else if mods&libdunit::KEYMOD_CTRL!=0&&mods&!(libdunit::KEYMOD_CTRL|libdunit::KEYMOD_SHIFT)==0{
                                if let Some(command)=dunit::text_clipboard::Command::from_scancode(input.x){
                                    checked(clipboard.command(checked(window.transfers(),"clipboard client"),command,editor.snapshot(text_revision)),"filename clipboard command");
                                }else{dirty=editor.key(&input);}
                            }else{dirty=editor.key(&input);}
                            if dirty{text_revision=text_revision.wrapping_add(1);}
                        },
                        Mode::Preview=>{
                            if ascii==KEY_ESC{files.preview=None;transition=Some(Mode::Browse);}
                            else if mods&libdunit::KEYMOD_CTRL!=0&&input.x==0x2e{if let Some(preview)=&files.preview{
                                let payload=checked(dunit::transfer::Payload::text(&preview.text).map_err(|e|dunit::Error(e.0)),"preview text");
                                checked(checked(window.transfers(),"clipboard client").publish_clipboard(payload,1,dunit::transfer::Action::Copy).map_err(|e|dunit::Error(e.0)),"copy preview");
                            }}
                        },
                        Mode::Confirm=>{if ascii==KEY_ESC{transition=Some(Mode::Browse);}else if ascii==KEY_ENTER_LF||ascii==KEY_ENTER_CR{files.delete_selected();transition=Some(Mode::Browse);}},
                        Mode::Menu{..}=>{if ascii==KEY_ESC{transition=Some(Mode::Browse);}},
                        Mode::Browse=>{
                            let ctrl=mods&libdunit::KEYMOD_CTRL!=0;
                            if ctrl&&mods&!(libdunit::KEYMOD_CTRL|libdunit::KEYMOD_SHIFT)==0{
                                match input.x{0x2e=>transfers.publish(&mut window,&mut files,false),0x2d=>transfers.publish(&mut window,&mut files,true),0x2f=>transfers.paste(&mut window,&mut files),0x1e=>files.select_all(),_=>{}}
                                dirty=true;
                            }else if ascii==KEY_ESC{if !checked(transfers.cancel_operation(&mut window,&mut files),"cancel transfer"){files.clear_selection();}dirty=true;}
                            else if matches!(input.x,0x47|0x4f|0x48|0x50|0x4b|0x4d|0x49|0x51)&&!files.items.is_empty(){
                                let old=files.current_index().unwrap_or(0)as i64;let cols=i64::from(files.cols());
                                let next=match input.x{0x47=>0,0x4f=>files.items.len()as i64-1,0x48=>old-cols,0x50=>old+cols,0x4b=>old-1,0x4d=>old+1,0x49=>old-cols*i64::from(files.rows()),_=>old+cols*i64::from(files.rows())};
                                files.choose(next.clamp(0,files.items.len()as i64-1)as usize,mods);dirty=true;
                            }else if ascii==KEY_ENTER_LF||ascii==KEY_ENTER_CR{if let Some(index)=files.current_index(){files.open(index);dirty=true;}}
                        },
                    }
                    if let Some(next)=transition{mode=next;text_revision=text_revision.wrapping_add(1);dirty=true;}
                }
            },
            _=>{},
        }
        if files.preview.is_some()&&matches!(mode,Mode::Browse){mode=Mode::Preview;}
        dirty|=checked(transfers.service(&mut window,&mut files),"copy service");
        pending_repaint|=dirty;
        if pending_repaint&&batch<64{
            if let Some(event)=checked(window.next_event(dunit::time::Duration::from_millis(0)),"queued window event"){
                queued_event=Some(event);batch+=1;continue;
            }
        }
        if pending_repaint{paint(&mut window,&mut files,&font,&icons,&mode);pending_repaint=false;}batch=0;
    }
    checked(window.close(),"window close");libdunit::exit(0)
}
