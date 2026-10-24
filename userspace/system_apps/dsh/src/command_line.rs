//! Shell word quoting, without expansion or evaluation of executable code.
use alloc::string::String;
use alloc::vec::Vec;

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
