//! Calculator state, decimal input and display formatting.
extern crate alloc;
use alloc::string::String;

pub(crate) const KEY_BACKSPACE: u8 = 8;
pub(crate) const KEY_DEL: u8 = 127;
pub(crate) const KEY_ENTER_LF: u8 = 10;
pub(crate) const KEY_ENTER_CR: u8 = 13;
pub(crate) const KEY_ESC: u8 = 27;
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Key {
    Digit(u8), // 0..=9
    Dot,
    Op(u8),
    Equals,
    Clear,
    Back,
    Sign,
    Percent,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(calc: &mut Calc, text: &str) {
        for byte in text.bytes() { calc.press(key_from_ascii(byte).unwrap()); }
    }
    #[test]
    fn arithmetic_editing_and_errors_follow_user_key_sequences() {
        let mut calc = Calc::new(); input(&mut calc, "7+8="); assert_eq!(calc.display(), "15");
        input(&mut calc, "c7*6="); assert_eq!(calc.display(), "42");
        input(&mut calc, "c1/0="); assert_eq!(calc.display(), "Error");
        input(&mut calc, "5+"); assert_eq!(calc.display(), "Error");
        input(&mut calc, "c123\x084"); assert_eq!(calc.display(), "124");
        input(&mut calc, "c.5+.25="); assert_eq!(calc.display(), "0.75");
        input(&mut calc, "c50%"); assert_eq!(calc.display(), "0.5");
    }
    #[test]
    fn finite_results_keep_precision_and_use_scientific_notation_when_needed() {
        assert_eq!(fmt_f64(0.0), "0"); assert_eq!(fmt_f64(-0.0), "0");
        assert_eq!(fmt_f64(0.1 + 0.2), "0.3"); assert_eq!(fmt_f64(-1.25), "-1.25");
        assert_eq!(fmt_f64(1.0e40), "1e40"); assert_eq!(fmt_f64(1.0e-12), "1e-12");
        assert_eq!(fmt_f64(f64::INFINITY), "Error"); assert_eq!(fmt_f64(f64::NAN), "Error");
        assert_ne!(fmt_f64(f64::MAX), "Error");
        assert!(parse_entry("garbage").is_nan());
    }
}

/// The 5×4 face. Modern layout: no duplicate keys, a real decimal point, a sign
/// toggle and a percent key. Column 3 is the operator column; the bottom row is
/// `± 0 . =`.
pub(crate) const KEYS: [[Key; 4]; 5] = [
    [Key::Clear, Key::Back, Key::Percent, Key::Op(b'/')],
    [Key::Digit(7), Key::Digit(8), Key::Digit(9), Key::Op(b'*')],
    [Key::Digit(4), Key::Digit(5), Key::Digit(6), Key::Op(b'-')],
    [Key::Digit(1), Key::Digit(2), Key::Digit(3), Key::Op(b'+')],
    [Key::Sign, Key::Digit(0), Key::Dot, Key::Equals],
];

pub(crate) fn key_label(k: Key) -> &'static str {
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
pub(crate) fn key_from_ascii(a: u8) -> Option<Key> {
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

fn parse_entry(s: &str) -> f64 {
    s.parse().unwrap_or(f64::NAN)
}

const FMT_DECIMALS: usize = 8;
fn fmt_f64(value: f64) -> String {
    if !value.is_finite() { return String::from("Error"); }
    let scientific = value != 0.0 && (value.abs() >= 1.0e12 || value.abs() < 1.0e-8);
    let mut text = if scientific {
        alloc::format!("{:.*e}", FMT_DECIMALS, value)
    } else {
        alloc::format!("{:.*}", FMT_DECIMALS, value)
    };
    let end = text.find('e').unwrap_or(text.len());
    let mut trimmed = end;
    while text.as_bytes()[trimmed - 1] == b'0' { trimmed -= 1; }
    if text.as_bytes()[trimmed - 1] == b'.' { trimmed -= 1; }
    text.replace_range(trimmed..end, "");
    if text == "-0" { text = String::from("0"); }
    text
}

/// The calculator engine: a running accumulator, a pending operator, and the
/// number currently being typed (`entry`). `entering` selects what the display
/// shows (the live `entry` vs. the computed `acc`).
pub(crate) struct Calc {
    entry: String,
    entering: bool,
    acc: f64,
    op: Option<u8>,
    error: bool,
    pub(crate) expr: String,
}

impl Calc {
    pub(crate) fn new() -> Calc {
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
    pub(crate) fn display(&self) -> String {
        if self.error {
            String::from("Error")
        } else if self.entering {
            self.entry.clone()
        } else {
            fmt_f64(self.acc)
        }
    }

    pub(crate) fn press(&mut self, k: Key) {
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
        if !r.is_finite() {
            self.error = true;
            return;
        }
        self.acc = r;
        self.entering = false;
    }
}

