#![no_std]
#![no_main]
//! SDK Image Viewer with BMP/PNG/JPEG/SVG decoding, Fit/Zoom/Pan, and Files integration.

extern crate alloc;

use alloc::{format, string::String, vec, vec::Vec};
use dunit::{
    args, errno,
    time::Duration,
    transfer::{Action, Format, Kind, Offer, Status},
    ui::{Component, ComponentWindow, DialogAction, InputOptions, Key, MenuItem, View},
    window::{WindowEvent, KEYMOD_CTRL},
    Error, Result,
};
use dunit_image::{Image, MAX_IMAGE_FILE_BYTES};

const W: u32 = 640;
const H: u32 = 460;
const TOP_RESERVED: usize = 68;
const BOTTOM_RESERVED: usize = 28;
const STYLE: &str = "\
Column#root { gap: 0; padding: 0; background: #00000000; } \
Row#toolbar { gap: 6; padding: 4; } \
Column#viewport { height: fill; background: #00000000; } \
Text#status { padding: 4; font-size: 12; }";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ZoomMode {
    Fit,
    Percent(u32),
}

#[derive(Clone, Debug)]
enum DialogMode {
    None,
    Open(String),
}

#[derive(Clone)]
enum Msg {
    OpenPrompt,
    ZoomFit,
    ZoomActual,
    ZoomIn,
    ZoomOut,
    Pan(i32, i32),
    ResetPan,
    WindowSize(u32, u32),
    CanvasSynced,
    LoadDecoded {
        name: String,
        image: Image,
    },
    DialogPath(String),
    DialogConfirm,
    DialogCancel,
    Quit,
}

struct ImageApp {
    path: String,
    image: Option<Image>,
    zoom: ZoomMode,
    pan_x: i32,
    pan_y: i32,
    win_w: u32,
    win_h: u32,
    status: String,
    dialog: DialogMode,
    canvas_dirty: bool,
    should_quit: bool,
}

impl ImageApp {
    fn new(path: String) -> Self {
        let mut app = Self {
            path: path.clone(),
            image: None,
            zoom: ZoomMode::Fit,
            pan_x: 0,
            pan_y: 0,
            win_w: W,
            win_h: H,
            status: String::new(),
            dialog: DialogMode::None,
            canvas_dirty: true,
            should_quit: false,
        };
        app.load_path(&path);
        app
    }

    fn load_path(&mut self, path: &str) {
        self.path = String::from(path);
        self.pan_x = 0;
        self.pan_y = 0;
        self.canvas_dirty = true;
        match dunit_image::load_auto(path) {
            Ok(img) => {
                self.status = format!("{} | {}x{}", path, img.width, img.height);
                self.image = Some(img);
            }
            Err(e) => {
                self.image = None;
                self.status = format!("Failed to open {}: {:?}", path, e);
            }
        }
    }

    fn zoom_pct(&self, win_w: u32, win_h: u32) -> u32 {
        match self.zoom {
            ZoomMode::Percent(p) => p,
            ZoomMode::Fit => {
                let Some(img) = &self.image else {
                    return 100;
                };
                let vw = (win_w as usize).saturating_sub(16).max(1);
                let vh = (win_h as usize)
                    .saturating_sub(TOP_RESERVED + BOTTOM_RESERVED + 16)
                    .max(1);
                let pw = (vw * 100) / img.width.max(1);
                let ph = (vh * 100) / img.height.max(1);
                (pw.min(ph) as u32).clamp(10, 400)
            }
        }
    }
}

impl Component for ImageApp {
    type Message = Msg;

    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::OpenPrompt => self.dialog = DialogMode::Open(self.path.clone()),
            Msg::ZoomFit => {
                self.zoom = ZoomMode::Fit;
                self.pan_x = 0;
                self.pan_y = 0;
                self.canvas_dirty = true;
            }
            Msg::ZoomActual => {
                self.zoom = ZoomMode::Percent(100);
                self.pan_x = 0;
                self.pan_y = 0;
                self.canvas_dirty = true;
            }
            Msg::ZoomIn => {
                let cur = self.zoom_pct(self.win_w, self.win_h);
                self.zoom = ZoomMode::Percent((cur + 25).min(400));
                self.canvas_dirty = true;
            }
            Msg::ZoomOut => {
                let cur = self.zoom_pct(self.win_w, self.win_h);
                self.zoom = ZoomMode::Percent(cur.saturating_sub(25).max(10));
                self.canvas_dirty = true;
            }
            Msg::Pan(dx, dy) => {
                self.pan_x = (self.pan_x + dx).clamp(-2048, 2048);
                self.pan_y = (self.pan_y + dy).clamp(-2048, 2048);
                self.canvas_dirty = true;
            }
            Msg::ResetPan => {
                self.pan_x = 0;
                self.pan_y = 0;
                self.canvas_dirty = true;
            }
            Msg::WindowSize(w, h) => {
                self.win_w = w;
                self.win_h = h;
                self.canvas_dirty = true;
            }
            Msg::CanvasSynced => {
                self.canvas_dirty = false;
            }
            Msg::LoadDecoded { name, image } => {
                self.status = format!("{} | {}x{}", name, image.width, image.height);
                self.path = name;
                self.image = Some(image);
                self.pan_x = 0;
                self.pan_y = 0;
                self.canvas_dirty = true;
            }
            Msg::DialogPath(p) => {
                if let DialogMode::Open(cur) = &mut self.dialog {
                    *cur = p;
                }
            }
            Msg::DialogConfirm => {
                if let DialogMode::Open(p) = core::mem::replace(&mut self.dialog, DialogMode::None) {
                    self.load_path(&p);
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
                        MenuItem::action("mi_open", "Open Image...", Msg::OpenPrompt).mnemonic('o'),
                        MenuItem::separator("mi_sep"),
                        MenuItem::action("mi_quit", "Exit", Msg::Quit).mnemonic('x'),
                    ],
                )
                .menu_mnemonic('f'),
                View::menu(
                    "m_view",
                    "View",
                    vec![
                        MenuItem::action("mi_fit", "Fit to Window", Msg::ZoomFit).mnemonic('f'),
                        MenuItem::action("mi_one", "Actual Size (100%)", Msg::ZoomActual).mnemonic('a'),
                        MenuItem::action("mi_zin", "Zoom In", Msg::ZoomIn).mnemonic('i'),
                        MenuItem::action("mi_zout", "Zoom Out", Msg::ZoomOut).mnemonic('u'),
                        MenuItem::action("mi_reset", "Reset Pan", Msg::ResetPan).mnemonic('r'),
                    ],
                )
                .menu_mnemonic('v'),
            ],
        );

        let toolbar = View::toolbar(
            "toolbar",
            "Image Controls",
            vec![
                View::button("btn_open", "Open", Msg::OpenPrompt).mnemonic('o'),
                View::button("btn_fit", "Fit", Msg::ZoomFit)
                    .class(if matches!(self.zoom, ZoomMode::Fit) { "primary" } else { "secondary" })
                    .mnemonic('w'),
                View::button("btn_100", "1:1", Msg::ZoomActual)
                    .class(if matches!(self.zoom, ZoomMode::Percent(100)) { "primary" } else { "secondary" })
                    .mnemonic('a'),
                View::button("btn_in", "+", Msg::ZoomIn),
                View::button("btn_out", "-", Msg::ZoomOut),
            ],
        );

        let viewport = View::column("viewport", vec![]);
        let zoom_label = match self.zoom {
            ZoomMode::Fit => String::from("Fit"),
            ZoomMode::Percent(p) => format!("{}%", p),
        };
        let status = View::status_bar(
            "status",
            "Image Status",
            format!("{} | zoom {}", self.status, zoom_label),
        );

        let root = View::column("root", vec![menu, toolbar, viewport, status]);

        let dialog = match &self.dialog {
            DialogMode::None => None,
            DialogMode::Open(p) => Some(View::dialog(
                "dlg_open",
                "Open Image File",
                View::input("dlg_open_in", "Image path", p.clone(), Msg::DialogPath, InputOptions::default())
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
            .on_key(Key::plain('='), Msg::ZoomIn)
            .on_key(Key::plain('+'), Msg::ZoomIn)
            .on_key(Key::plain('-'), Msg::ZoomOut)
            .on_key(Key::plain('0'), Msg::ZoomFit)
            .on_key(Key::plain('1'), Msg::ZoomActual)
            .on_key(Key::plain('h'), Msg::Pan(-32, 0))
            .on_key(Key::plain('l'), Msg::Pan(32, 0))
            .on_key(Key::plain('k'), Msg::Pan(0, -32))
            .on_key(Key::plain('j'), Msg::Pan(0, 32))
    }
}

fn render_image_canvas(app: &ImageApp, width: u32, height: u32, bg: u32) -> Vec<u32> {
    let w = width as usize;
    let h = height as usize;
    let mut buf = vec![bg; w * h];
    let Some(img) = &app.image else {
        return buf;
    };
    let pct = app.zoom_pct(width, height) as usize;
    let draw_w = ((img.width * pct) / 100).max(1);
    let draw_h = ((img.height * pct) / 100).max(1);

    let vp_top = TOP_RESERVED;
    let vp_bottom = h.saturating_sub(BOTTOM_RESERVED);
    let vp_h = vp_bottom.saturating_sub(vp_top).max(1);

    let ox = (w as i32 - draw_w as i32) / 2 + app.pan_x;
    let oy = vp_top as i32 + (vp_h as i32 - draw_h as i32) / 2 + app.pan_y;

    for dy in 0..draw_h {
        let py = oy + dy as i32;
        if py < vp_top as i32 || py >= vp_bottom as i32 {
            continue;
        }
        let sy = (dy * img.height) / draw_h;
        for dx in 0..draw_w {
            let px = ox + dx as i32;
            if px < 0 || px >= w as i32 {
                continue;
            }
            let sx = (dx * img.width) / draw_w;
            let src = img.pixels[sy * img.width + sx];
            let a = (src >> 24) & 0xFF;
            let dst = &mut buf[(py as usize) * w + (px as usize)];
            if a == 255 {
                *dst = src;
            } else if a > 0 {
                let inv = 255 - a;
                let sr = (src >> 16) & 0xFF;
                let sg = (src >> 8) & 0xFF;
                let sb = src & 0xFF;
                let dr = (*dst >> 16) & 0xFF;
                let dg = (*dst >> 8) & 0xFF;
                let db = *dst & 0xFF;
                let r = (sr * a + dr * inv) / 255;
                let g = (sg * a + dg * inv) / 255;
                let b = (sb * a + db * inv) / 255;
                *dst = 0xFF00_0000 | (r << 16) | (g << 8) | b;
            }
        }
    }
    buf
}

#[dunit::main]
fn main() -> Result<()> {
    let argv = args::args()?;
    let mut path = String::from("/assets/images/logo.png");
    let mut check_only = false;
    let mut self_test = false;
    for arg in argv.iter().skip(1) {
        if arg == "--dimensions" || arg == "--no-wait" {
            check_only = true;
        } else if arg == "--self-test" {
            self_test = true;
        } else {
            path = arg.clone();
        }
    }

    if check_only {
        let img = dunit_image::load_auto(&path).map_err(|_| Error(errno::EINVAL))?;
        dunit::println!("[GUI-IMAGE] {} {}x{}", path, img.width, img.height)?;
        return Ok(());
    }

    let app = ImageApp::new(path.clone());
    let bg = dunit::ui::load_theme("")
        .ok()
        .and_then(|(t, _)| t.tokens.color("window-bg"))
        .map_or(0xFF1E_1E2E, |c| {
            0xFF00_0000 | ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
        });
    let mut ui = ComponentWindow::open_themed_with_style(W, H, app, STYLE)?;
    let offer = Offer {
        formats: alloc::vec![Format::Files],
        actions: Action::Copy as u32 | Action::Link as u32,
        preferred: Action::Copy,
    };
    if let Ok(t) = ui.transfers() {
        let _ = t.register_opener(offer.clone());
    }
    let _ = ui.set_transfer_target(Some(offer));
    if let Some(img) = &ui.component().image {
        let _ = dunit::println!("[GUI-IMAGE] loaded {} {}x{}", path, img.width, img.height);
    }

    if self_test {
        let cur_size = ui.size();
        ui.update(Msg::WindowSize(cur_size.0, cur_size.1))?;
        let pixels = render_image_canvas(ui.component(), cur_size.0, cur_size.1, bg);
        ui.set_canvas(pixels, cur_size.0, cur_size.1)?;
        ui.update(Msg::ZoomActual)?;
        let actual_pixels = render_image_canvas(ui.component(), cur_size.0, cur_size.1, bg);
        ui.set_canvas(actual_pixels, cur_size.0, cur_size.1)?;
        ui.update(Msg::ZoomFit)?;
        dunit::println!("[GUI-IMAGE] self-test OK")?;
        return ui.close();
    }

    let mut last_size = ui.size();
    loop {
        if ui.component().should_quit {
            return ui.close();
        }
        let cur_size = ui.size();
        if cur_size != last_size {
            ui.update(Msg::WindowSize(cur_size.0, cur_size.1))?;
        }
        if ui.component().canvas_dirty || cur_size != last_size {
            last_size = cur_size;
            let pixels = render_image_canvas(ui.component(), cur_size.0, cur_size.1, bg);
            ui.set_canvas(pixels, cur_size.0, cur_size.1)?;
            ui.update(Msg::CanvasSynced)?;
        }
        match ui.next_event(Duration::from_millis(200)) {
            Ok(Some(WindowEvent::CloseRequested)) => return ui.close(),
            Ok(Some(WindowEvent::Transfer(mut ev))) => {
                let msg = ev.message;
                if matches!(msg.kind, Kind::OpenData | Kind::Data) {
                    let mut status = Status::Unsupported;
                    if let Some(payload) = ev.payload.take() {
                        if let Some(file) = payload.files.first() {
                            if let Ok(bytes) = file.read_all(MAX_IMAGE_FILE_BYTES) {
                                if let Ok(img) = dunit_image::decode(&bytes) {
                                    dunit::println!(
                                        "[GUI-IMAGE] transfer loaded {} {}x{}",
                                        file.path,
                                        img.width,
                                        img.height
                                    )?;
                                    let _ = ui.update(Msg::LoadDecoded {
                                        name: file.path.clone(),
                                        image: img,
                                    });
                                    status = Status::Ok;
                                }
                            }
                        }
                    }
                    if let Ok(t) = ui.transfers() {
                        let _ = t.complete(msg.transfer, status);
                    }
                } else if msg.kind == Kind::DragEnter {
                    if let Ok(t) = ui.transfers() {
                        let _ = t.drag_accept(msg.transfer, 0, Action::Copy);
                    }
                }
            }
            Ok(_) => {}
            Err(Error(errno::EPIPE)) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}
