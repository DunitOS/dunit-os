//! Shell word quoting, without expansion or evaluation of executable code.
#![no_std]
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

/// Quote one literal word for `words` and the interactive line editor. Control
/// characters are rejected: quoting cannot stop LF/ESC from acting before the
/// word parser receives the submitted command line.
pub fn quote_word(value: &str) -> Result<String, &'static str> {
    if value.chars().any(char::is_control) {
        return Err("control character in shell word");
    }
    let mut out = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    Ok(out)
}
pub fn quote_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> Result<String, &'static str> {
    let mut out = String::new();
    for path in paths {
        if !path.starts_with('/') || path.split('/').any(|p| p == ".." || p == ".") {
            return Err("expected an absolute normalized path");
        }
        let word = quote_word(path)?;
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&word);
    }
    Ok(out)
}
pub fn previous_boundary(text: &str, index: usize) -> usize {
    text.char_indices()
        .take_while(|(at, _)| *at < index)
        .last()
        .map_or(0, |(at, _)| at)
}
pub fn next_boundary(text: &str, index: usize) -> usize {
    text.char_indices()
        .find(|(at, _)| *at > index)
        .map_or(text.len(), |(at, _)| at)
}
/// Streaming UTF-8 input; incomplete sequences stay private until complete.
/// Invalid bytes are discarded and a new lead/control byte resynchronizes.
#[derive(Default)]
pub struct Utf8Decoder {
    bytes: [u8; 4],
    len: usize,
    need: usize,
}
impl Utf8Decoder {
    pub fn push(&mut self, byte: u8) -> Option<char> {
        if byte < 128 {
            self.len = 0;
            self.need = 0;
            return Some(byte as char);
        }
        if self.len != 0 && byte & 0xc0 != 0x80 {
            self.len = 0;
            self.need = 0;
        }
        if self.len == 0 {
            self.need = match byte {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return None,
            };
            self.bytes[0] = byte;
            self.len = 1;
            return None;
        }
        self.bytes[self.len] = byte;
        self.len += 1;
        if self.len != self.need {
            return None;
        }
        let ch = core::str::from_utf8(&self.bytes[..self.len])
            .ok()
            .and_then(|s| s.chars().next());
        self.len = 0;
        self.need = 0;
        ch
    }
}

/// UTF-8 cooked input for a child PTY. An oversized line is rejected as a
/// whole, rather than executing a silently truncated command or argument.
pub struct CookedLine {
    text: String,
    decoder: Utf8Decoder,
    limit: usize,
    overflow: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum CookedEvent {
    None,
    Echo(char),
    Erase,
    Submit(Vec<u8>),
    Rejected,
    Interrupt,
}
impl CookedLine {
    pub fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            decoder: Utf8Decoder::default(),
            limit,
            overflow: false,
        }
    }
    pub fn feed(&mut self, byte: u8) -> CookedEvent {
        let Some(ch) = self.decoder.push(byte) else {
            return CookedEvent::None;
        };
        match ch {
            '\u{3}' => {
                self.text.clear();
                self.overflow = false;
                CookedEvent::Interrupt
            }
            '\n' => {
                if self.overflow {
                    self.text.clear();
                    self.overflow = false;
                    return CookedEvent::Rejected;
                }
                let mut bytes = core::mem::take(&mut self.text).into_bytes();
                bytes.push(b'\n');
                CookedEvent::Submit(bytes)
            }
            '\u{8}' | '\u{7f}' => {
                if !self.overflow && self.text.pop().is_some() {
                    CookedEvent::Erase
                } else {
                    CookedEvent::None
                }
            }
            ch if !ch.is_control() => {
                if self.overflow {
                    return CookedEvent::None;
                }
                if self.text.len().saturating_add(ch.len_utf8()) > self.limit {
                    self.overflow = true;
                    return CookedEvent::None;
                }
                self.text.push(ch);
                CookedEvent::Echo(ch)
            }
            _ => CookedEvent::None,
        }
    }
}

/// Echo's one supported output redirection. Operators inside quoted/escaped
/// words stay literal, which is essential for file paths pasted by the terminal.
pub fn echo_arguments(line: &str) -> Result<(Vec<String>, Option<(String, bool)>), &'static str> {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(current) = quote {
            if ch == current {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        if ch == '>' {
            let append = line[index..].starts_with(">>");
            let skip = if append { 2 } else { 1 };
            let arguments = words(&line[..index])?;
            let mut destination = words(&line[index + skip..])?;
            if destination.len() != 1 {
                return Err("echo redirection requires exactly one path");
            }
            return Ok((arguments, Some((destination.remove(0), append))));
        }
    }
    Ok((words(line)?, None))
}

pub fn words(line: &str) -> Result<Vec<String>, &'static str> {
    let mut result = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for ch in line.chars() {
        if ch == '\0' {
            return Err("NUL in command");
        }
        if escaped {
            word.push(ch);
            started = true;
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            started = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            } else {
                word.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            started = true;
        } else if ch.is_ascii_whitespace() {
            if started {
                result.push(core::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(ch);
            started = true;
        }
    }
    if escaped {
        return Err("trailing escape");
    }
    if quote.is_some() {
        return Err("unterminated quote");
    }
    if started {
        result.push(word);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropped_paths_are_literal_utf8_words_without_line_injection() {
        let paths = ["/tmp/a b", "/tmp/привет'мир", "/tmp/$HOME;$(x)", "/tmp/\\x"];
        assert_eq!(words(&quote_paths(paths).unwrap()).unwrap(), paths);
        assert_eq!(words(&quote_word("").unwrap()).unwrap(), [""]);
        for path in [
            "/tmp/a\nb",
            "/tmp/a\rb",
            "/tmp/\x1b[x",
            "/tmp/\0",
            "relative",
            "/tmp/../x",
        ] {
            assert!(quote_paths([path]).is_err());
        }
    }
    #[test]
    fn stream_and_caret_never_split_utf8() {
        let value = "aé🙂я";
        let mut decoder = Utf8Decoder::default();
        let actual: String = value.bytes().filter_map(|b| decoder.push(b)).collect();
        assert_eq!(actual, value);
        let mut at = value.len();
        while at > 0 {
            let before = previous_boundary(value, at);
            assert!(value.is_char_boundary(before));
            assert_eq!(next_boundary(value, before), at);
            at = before;
        }
        assert_eq!(decoder.push(0xf0), None);
        assert_eq!(decoder.push(b'\n'), Some('\n'));
        assert_eq!(decoder.push(0x80), None);
    }
    #[test]
    fn cooked_utf8_erase_and_whole_line_overflow() {
        let mut line = CookedLine::new(4);
        for byte in "яé".bytes() {
            line.feed(byte);
        }
        assert_eq!(line.feed(127), CookedEvent::Erase);
        assert_eq!(
            line.feed(b'\n'),
            CookedEvent::Submit("я\n".as_bytes().to_vec())
        );
        for byte in "abcde".bytes() {
            line.feed(byte);
        }
        assert_eq!(line.feed(127), CookedEvent::None);
        assert_eq!(line.feed(b'\n'), CookedEvent::Rejected);
        assert_eq!(line.feed(b'\n'), CookedEvent::Submit(alloc::vec![b'\n']));
        line.feed(0xf0);
        assert_eq!(line.feed(3), CookedEvent::Interrupt);
        assert_eq!(line.feed(b'\n'), CookedEvent::Submit(alloc::vec![b'\n']));
    }
    #[test]
    fn quoted_arguments_and_empty() {
        assert_eq!(
            words(r#"exec lua '/tmp/a b.lua' "" a\ b"#).unwrap(),
            ["exec", "lua", "/tmp/a b.lua", "", "a b"]
        );
    }
    #[test]
    fn syntax_errors() {
        let trailing = alloc::format!("lua {}", char::from(92));
        for line in ["lua 'x", trailing.as_str(), "lua a\0b"] {
            assert!(words(line).is_err());
        }
    }
    #[test]
    fn embedded_quotes_unicode() {
        assert_eq!(
            words(r#"a'b'"c" 'привет мир' '\'"#).unwrap(),
            ["abc", "привет мир", "\\"]
        );
        assert!(words(" \t").unwrap().is_empty());
    }
}

#[cfg(test)]
mod redirection_tests {
    use super::*;
    #[test]
    fn quoted_file_names_are_literal_and_only_unquoted_redirects_write() {
        let path = "/tmp/часы's > notes";
        let quoted = quote_word(path).unwrap();
        assert_eq!(
            echo_arguments(&quoted),
            Ok((alloc::vec![String::from(path)], None))
        );
        let command = alloc::format!("{} >> '/tmp/out file'", quoted);
        assert_eq!(
            echo_arguments(&command),
            Ok((
                alloc::vec![String::from(path)],
                Some((String::from("/tmp/out file"), true))
            ))
        );
        assert!(echo_arguments("a > b > c").is_err());
        assert!(echo_arguments("'unterminated").is_err());
    }
}
