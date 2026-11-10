#![no_std]
#![no_main]
//! Calculator domain model over SDK controls. SDK owns hit testing and frames.
extern crate alloc;
use alloc::{string::String, vec};
use dunit::transfer::{Action, Kind, Payload, Status};
use dunit::ui::{Component, ComponentWindow, Renderer, View};
use dunit::window::{
    PixelFormat, WindowEvent, IN_DOWN, IN_FOCUS_OUT, IN_KEY, IN_LEAVE, IN_MOVE, IN_UP,
};
const KEY_BACKSPACE: u8 = 8;
const KEY_DEL: u8 = 127;
const KEY_ENTER_LF: u8 = 10;
const KEY_ENTER_CR: u8 = 13;
const KEY_ESC: u8 = 27;
#[derive(Clone, Copy, PartialEq)]
enum Key {
    Digit(u8), // 0..=9
    Dot,
    Op(u8),
    Equals,
    Clear,
    Back,
    Sign,
    Percent,
}

/// The 5×4 face. Modern layout: no duplicate keys, a real decimal point, a sign
/// toggle and a percent key. Column 3 is the operator column; the bottom row is
/// `± 0 . =`.
const KEYS: [[Key; 4]; 5] = [
    [Key::Clear, Key::Back, Key::Percent, Key::Op(b'/')],
    [Key::Digit(7), Key::Digit(8), Key::Digit(9), Key::Op(b'*')],
    [Key::Digit(4), Key::Digit(5), Key::Digit(6), Key::Op(b'-')],
    [Key::Digit(1), Key::Digit(2), Key::Digit(3), Key::Op(b'+')],
    [Key::Sign, Key::Digit(0), Key::Dot, Key::Equals],
];

fn key_label(k: Key) -> &'static str {
    match k {
        Key::Digit(0) => "0",
        Key::Digit(1) => "1",
        Key::Digit(2) => "2",
        Key::Digit(3) => "3",
        Key::Digit(4) => "4",
        Key::Digit(5) => "5",
        Key::Digit(6) => "6",
        Key::Digit(7) => "7",
        Key::Digit(8) => "8",
        Key::Digit(9) => "9",
        Key::Digit(_) => "?",
        Key::Dot => ".",
        Key::Op(b'+') => "+",
        Key::Op(b'-') => "\u{2212}", // − minus sign
        Key::Op(b'*') => "\u{00D7}", // × multiplication sign
        Key::Op(b'/') => "\u{00F7}", // ÷ division sign
        Key::Op(_) => "?",
        Key::Equals => "=",
        Key::Clear => "C",
        Key::Back => "\u{232B}", // ⌫ erase to the left
        Key::Sign => "\u{00B1}", // ± plus-minus
        Key::Percent => "%",
    }
}

/// The display symbol for an operator on the pending-expression line.
fn op_sym(o: u8) -> &'static str {
    match o {
        b'+' => "+",
        b'-' => "\u{2212}",
        b'*' => "\u{00D7}",
        b'/' => "\u{00F7}",
        _ => "?",
    }
}

/// Map a cooked ASCII key (from IN_KEY) to a calculator key.
fn key_from_ascii(a: u8) -> Option<Key> {
    match a {
        b'0'..=b'9' => Some(Key::Digit(a - b'0')),
        b'.' | b',' => Some(Key::Dot),
        b'+' => Some(Key::Op(b'+')),
        b'-' => Some(Key::Op(b'-')),
        b'*' | b'x' | b'X' => Some(Key::Op(b'*')),
        b'/' => Some(Key::Op(b'/')),
        b'%' => Some(Key::Percent),
        b'=' | KEY_ENTER_LF | KEY_ENTER_CR => Some(Key::Equals),
        KEY_BACKSPACE | KEY_DEL => Some(Key::Back),
        KEY_ESC | b'c' | b'C' => Some(Key::Clear),
        _ => None,
    }
}

const ENTRY_CAP: usize = 16;

/// True for NaN or ±infinity, using pure arithmetic (no libm / `is_finite`):
/// NaN is the only value not equal to itself; ±inf falls outside the f64 range.
fn bad(v: f64) -> bool {
    v != v || v > 1.0e308 || v < -1.0e308
}

/// Parse the entry buffer (only `[-]digits[.digits]`, which we author) into an
/// f64 by exact integer accumulation — no `core` dec2flt dependency.
fn parse_entry(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut i = 0usize;
    let neg = !b.is_empty() && b[0] == b'-';
    if neg {
        i = 1;
    }
    let mut int_part = 0.0f64;
    while i < b.len() && b[i].is_ascii_digit() {
        int_part = int_part * 10.0 + (b[i] - b'0') as f64;
        i += 1;
    }
    let mut frac = 0.0f64;
    let mut scale = 1.0f64;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            frac = frac * 10.0 + (b[i] - b'0') as f64;
            scale *= 10.0;
            i += 1;
        }
    }
    let v = int_part + frac / scale;
    if neg {
        -v
    } else {
        v
    }
}

fn pow10(n: u32) -> u128 {
    let mut r = 1u128;
    let mut i = 0;
    while i < n {
        r *= 10;
        i += 1;
    }
    r
}

fn append_u128(s: &mut String, mut n: u128) {
    if n == 0 {
        s.push('0');
        return;
    }
    let mut tmp = [0u8; 40];
    let mut t = 0usize;
    while n > 0 {
        tmp[t] = b'0' + (n % 10) as u8;
        n /= 10;
        t += 1;
    }
    while t > 0 {
        t -= 1;
        s.push(tmp[t] as char);
    }
}

/// Fractional digits kept before trailing zeros are trimmed.
const FMT_DECIMALS: u32 = 8;

/// Format an f64 for the display: integers show without a decimal point,
/// fractions are rounded to `FMT_DECIMALS` places with trailing zeros trimmed,
/// non-finite → "Error". Integer casts + manual rounding, no libm.
fn fmt_f64(v: f64) -> String {
    let mut s = String::new();
    if bad(v) {
        return String::from("Error");
    }
    let neg = v < 0.0;
    let x = if neg { -v } else { v };
    // Beyond ~1e18 there is no fractional precision left; render the integer.
    if x >= 1.0e18 {
        if x >= 3.0e38 {
            return String::from("Error"); // beyond u128
        }
        if neg {
            s.push('-');
        }
        append_u128(&mut s, x as u128);
        return s;
    }
    let scale = pow10(FMT_DECIMALS);
    let rounded = (x * scale as f64 + 0.5) as u128; // round half up
    let ip = rounded / scale;
    let fp = rounded % scale;
    if neg && rounded != 0 {
        s.push('-');
    }
    append_u128(&mut s, ip);
    if fp != 0 {
        let mut frac = [0u8; FMT_DECIMALS as usize];
        let mut r = fp;
        let mut i = FMT_DECIMALS as usize;
        while i > 0 {
            i -= 1;
            frac[i] = b'0' + (r % 10) as u8;
            r /= 10;
        }
        let mut end = FMT_DECIMALS as usize;
        while end > 0 && frac[end - 1] == b'0' {
            end -= 1;
        }
        if end > 0 {
            s.push('.');
            for &b in &frac[..end] {
                s.push(b as char);
            }
        }
    }
    s
}

/// The calculator engine: a running accumulator, a pending operator, and the
/// number currently being typed (`entry`). `entering` selects what the display
/// shows (the live `entry` vs. the computed `acc`).
struct Calc {
    entry: String,
    entering: bool,
    acc: f64,
    op: Option<u8>,
    error: bool,
    expr: String,
}

impl Calc {
    fn new() -> Calc {
        Calc {
            entry: String::from("0"),
            entering: true,
            acc: 0.0,
            op: None,
            error: false,
            expr: String::new(),
        }
    }

    /// The value the next operation should consume.
    fn current(&self) -> f64 {
        if self.entering {
            parse_entry(&self.entry)
        } else {
            self.acc
        }
    }

    /// The string to paint in the display.
    fn display(&self) -> String {
        if self.error {
            String::from("Error")
        } else if self.entering {
            self.entry.clone()
        } else {
            fmt_f64(self.acc)
        }
    }

    fn press(&mut self, k: Key) {
        // In the error state only Clear is live.
        if self.error && !matches!(k, Key::Clear) {
            return;
        }
        match k {
            Key::Digit(d) => self.digit(d),
            Key::Dot => self.dot(),
            Key::Op(o) => self.set_op(o),
            Key::Equals => self.equals(),
            Key::Clear => *self = Calc::new(),
            Key::Back => self.back(),
            Key::Sign => self.sign(),
            Key::Percent => self.percent(),
        }
    }

    fn start_entry(&mut self) {
        self.entry.clear();
        self.entering = true;
    }

    fn digit(&mut self, d: u8) {
        if !self.entering {
            self.start_entry();
        }
        if self.entry == "0" {
            self.entry.clear();
        } else if self.entry == "-0" {
            self.entry.truncate(1); // keep the sign
        }
        if self.entry.len() < ENTRY_CAP {
            self.entry.push((b'0' + d) as char);
        }
        if self.entry.is_empty() || self.entry == "-" {
            self.entry.push('0');
        }
    }

    fn dot(&mut self) {
        if !self.entering {
            self.start_entry();
        }
        if self.entry.is_empty() || self.entry == "-" {
            self.entry.push('0');
        }
        if !self.entry.contains('.') && self.entry.len() < ENTRY_CAP {
            self.entry.push('.');
        }
    }

    fn back(&mut self) {
        if !self.entering {
            return;
        }
        self.entry.pop();
        if self.entry.is_empty() || self.entry == "-" {
            self.entry = String::from("0");
        }
    }

    fn sign(&mut self) {
        if self.entering {
            if self.entry.starts_with('-') {
                self.entry.remove(0);
            } else if self.entry != "0" {
                self.entry.insert(0, '-');
            }
        } else {
            self.acc = -self.acc;
        }
    }

    fn percent(&mut self) {
        self.acc = self.current() / 100.0;
        self.entering = false;
    }

    fn set_op(&mut self, o: u8) {
        let cur = self.current();
        if self.op.is_some() && self.entering {
            self.fold(cur); // chain left-to-right
        } else {
            self.acc = cur;
        }
        if self.error {
            return;
        }
        self.op = Some(o);
        self.entering = false;
        self.expr = {
            let mut s = fmt_f64(self.acc);
            s.push(' ');
            s.push_str(op_sym(o));
            s
        };
    }

    fn equals(&mut self) {
        if self.op.is_some() {
            let rhs = self.current();
            self.fold(rhs);
            self.op = None;
            self.expr.clear();
            self.entering = false;
        }
    }

    /// `acc = acc <op> rhs`, guarding divide-by-zero and non-finite → error.
    fn fold(&mut self, rhs: f64) {
        let Some(o) = self.op else {
            return;
        };
        let r = match o {
            b'+' => self.acc + rhs,
            b'-' => self.acc - rhs,
            b'*' => self.acc * rhs,
            b'/' => {
                if rhs == 0.0 {
                    self.error = true;
                    return;
                }
                self.acc / rhs
            }
            _ => return,
        };
        if bad(r) {
            self.error = true;
            return;
        }
        self.acc = r;
        self.entering = false;
    }
}

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
    let overlay = alloc::format!("Column#calculator {{ padding: 10; gap: 8; background: #00000000; }} Column#display {{ background: #{:06X}; padding: 8; height: 76; }} Text {{ color: #{:06X}; }} Text#expression {{ color: #{:06X}; font-size: 12; }} Text#value {{ font-size: 24; }} Grid#keys {{ background: #00000000; grid-columns: 4; gap: 6; height: fill; }} Button {{ background: #{:06X}; color: #{:06X}; height: fill; min-height: 36; }} Button.function {{ background: #{:06X}; }} Button.operator {{ background: #{:06X}; }} Button.equals {{ background: #{:06X}; }}", cfg.display_bg & 0xffffff, cfg.text & 0xffffff, cfg.muted & 0xffffff, cfg.btn & 0xffffff, cfg.text & 0xffffff, cfg.btn_fn & 0xffffff, cfg.btn_op & 0xffffff, cfg.btn_eq & 0xffffff);
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
