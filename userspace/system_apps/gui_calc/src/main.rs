#![no_std]
#![no_main]
//! Calculator domain model over SDK controls. SDK owns hit testing and frames.
extern crate alloc;
use alloc::vec;
use dunit::transfer::{Action, Kind, Payload, Status};
use dunit::ui::{Component, ComponentWindow, Renderer, View};
use dunit::window::{
    PixelFormat, WindowEvent, IN_DOWN, IN_FOCUS_OUT, IN_KEY, IN_LEAVE, IN_MOVE, IN_UP,
};
mod calculator;
use calculator::*;

fn calculator_key(event: dunit::ui::UiEvent) -> dunit::ui::EventResponse<Key> {
    if let dunit::ui::UiEventKind::NativeKey {
        scancode,
        character,
        modifiers,
    } = event.kind
    {
        if modifiers & dunit::window::KEYMOD_CTRL == 0 {
            let ascii = match (scancode, character) {
                (0x01, 0) => KEY_ESC,
                (0x0e, 0) => KEY_BACKSPACE,
                (0x53, 0) => KEY_DEL,
                _ => character as u8,
            };
            if let Some(key) = key_from_ascii(ascii) {
                return dunit::ui::EventResponse::message(key).prevent();
            }
        }
    }
    Default::default()
}
impl Component for Calc {
    type Message = Key;
    fn update(&mut self, key: Key) {
        self.press(key);
    }
    fn view(&self) -> View<Key> {
        let keys = KEYS
            .iter()
            .flatten()
            .enumerate()
            .map(|(i, key)| {
                View::button(alloc::format!("key{}", i), key_label(*key), *key).class(match key {
                    Key::Digit(_) | Key::Dot => "digit",
                    Key::Op(_) => "operator",
                    Key::Equals => "equals",
                    _ => "function",
                })
            })
            .collect();
        View::column(
            "calculator",
            vec![
                View::column(
                    "display",
                    vec![
                        View::text("expression", self.expr.clone()),
                        View::text("value", self.display()),
                    ],
                ),
                View::grid("keys", keys),
            ],
        )
        .on_capture(calculator_key)
    }
}
fn native(error: dunit::transfer::Error) -> dunit::Error {
    dunit::Error(error.0)
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let cfg = dwm_settings::CalcCfg::load("gui_calc");
    let format = if cfg.bg_alpha < 255 {
        PixelFormat::Argb8888
    } else {
        PixelFormat::Xrgb8888
    };
    dunit::println!(
        "gui_calc: cfg from_file={} bg={:#010X} btn_op={:#010X} bg_alpha={} fmt={}",
        cfg.from_file as u32,
        cfg.bg,
        cfg.btn_op,
        cfg.bg_alpha,
        if cfg.bg_alpha < 255 {
            "argb8888"
        } else {
            "xrgb8888"
        }
    )?;
    let overlay = alloc::format!("Column#calculator {{ padding: 10; gap: 8; background: #00000000; }} Column#display {{ background: #{:06X}; padding: 8; height: 76; }} Text {{ color: #{:06X}; }} Text#expression {{ color: #{:06X}; font-size: 12; }} Text#value {{ font-size: 24; }} Grid#keys {{ padding: 0; background: #00000000; grid-columns: 4; gap: 6; height: fill; }} Button {{ background: #{:06X}; color: #{:06X}; height: fill; min-height: 36; }} Button.function {{ background: #{:06X}; }} Button.operator {{ background: #{:06X}; }} Button.equals {{ background: #{:06X}; }}", cfg.display_bg & 0xffffff, cfg.text & 0xffffff, cfg.muted & 0xffffff, cfg.btn & 0xffffff, cfg.text & 0xffffff, cfg.btn_fn & 0xffffff, cfg.btn_op & 0xffffff, cfg.btn_eq & 0xffffff);
    let (theme, _) = dunit::ui::load_theme(&overlay)?;
    let desktop = dwm_settings::load();
    let font = dunit::canvas::Font::load(&[
        cfg.font.as_str(),
        desktop.desktop.font.as_str(),
        "/assets/fonts/DejaVuSans.ttf",
    ])?;
    let mut ui = ComponentWindow::open_format(
        260,
        360,
        Calc::new(),
        Renderer::new(
            font.into_inner(),
            theme.cascade,
            dunit::canvas::Color::rgba(
                (cfg.bg >> 16) as u8,
                (cfg.bg >> 8) as u8,
                cfg.bg as u8,
                cfg.bg_alpha as u8,
            ),
        ),
        format,
    )?;
    dunit::println!("gui_calc: surface presented OK")?;
    let mut candidate = None;
    let mut source = None;
    loop {
        let input = match ui.next_event(dunit::time::Duration::from_millis(40))? {
            Some(WindowEvent::CloseRequested) => break,
            Some(WindowEvent::Resized { .. }) => {
                candidate = None;
                dunit::println!("gui_calc: reconfigured OK")?;
                continue;
            }
            Some(WindowEvent::Transfer(event)) => {
                let m = event.message;
                if event.source
                    && matches!(m.kind, Kind::Cancelled | Kind::Finished)
                    && source == Some(m.request)
                {
                    source = None;
                }
                if m.kind == Kind::Result
                    && source == Some(m.request)
                    && m.arg0 != Status::Ok as u32
                {
                    source = None;
                    dunit::println!("gui_calc: drag rejected")?;
                }
                continue;
            }
            Some(WindowEvent::Input(input)) => input,
            _ => continue,
        };
        if matches!(input.kind, IN_FOCUS_OUT | IN_LEAVE | IN_UP) {
            candidate = None;
        }
        if input.kind == IN_DOWN && input.pointer_button() == 0 {
            if ui.widget_rect("display").is_some_and(|r| {
                input.x as f32 >= r.x
                    && (input.x as f32) < r.x + r.w
                    && input.y as f32 >= r.y
                    && (input.y as f32) < r.y + r.h
            }) {
                candidate = Some((
                    input.x,
                    input.y,
                    ui.component().display(),
                    dwm_settings::load().transfers.drag_threshold_px,
                ));
            }
        } else if input.kind == IN_MOVE {
            if let Some((x, y, text, threshold)) = candidate.as_ref() {
                let dx = i64::from(input.x) - i64::from(*x);
                let dy = i64::from(input.y) - i64::from(*y);
                if dx * dx + dy * dy >= i64::from(*threshold).pow(2) {
                    let payload = Payload::text(text).map_err(native)?;
                    source = Some(
                        ui.transfers()?
                            .start_drag(payload, 1, Action::Copy)
                            .map_err(native)?,
                    );
                    candidate = None;
                }
            }
        } else if input.kind == IN_KEY {
            let modifiers = input.y as u8;
            if modifiers & dunit::window::KEYMOD_CTRL != 0 {
                if input.x == 0x2e
                    && modifiers & !(dunit::window::KEYMOD_CTRL | dunit::window::KEYMOD_SHIFT) == 0
                {
                    let payload = Payload::text(&ui.component().display()).map_err(native)?;
                    ui.transfers()?
                        .publish_clipboard(payload, 1, Action::Copy)
                        .map_err(native)?;
                }
            }
        }
    }
    ui.close()
}
