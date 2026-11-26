//! Multi-language syntax tokenizer with multi-line comment and fence tracking.
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    PlainText,
    Rust,
    C,
    Toml,
    Markdown,
    Shell,
    Lua,
    Json,
}

impl Language {
    pub fn display_name(self) -> &'static str {
        match self {
            Language::PlainText => "Text",
            Language::Rust => "Rust",
            Language::C => "C",
            Language::Toml => "TOML",
            Language::Markdown => "Markdown",
            Language::Shell => "Shell",
            Language::Lua => "Lua",
            Language::Json => "JSON",
        }
    }

    pub fn comment_prefix(self) -> &'static str {
        match self {
            Language::Rust | Language::C => "//",
            Language::Lua => "--",
            Language::Toml | Language::Shell | Language::PlainText | Language::Markdown => "#",
            Language::Json => "//",
        }
    }

    pub fn parse_name(name: &str) -> Option<Self> {
        let low = name.trim().to_ascii_lowercase();
        match low.as_str() {
            "off" | "none" | "text" | "plain" | "plaintext" | "txt" => Some(Language::PlainText),
            "rs" | "rust" => Some(Language::Rust),
            "c" | "h" | "cpp" => Some(Language::C),
            "toml" | "ini" | "conf" => Some(Language::Toml),
            "md" | "markdown" => Some(Language::Markdown),
            "sh" | "dsh" | "shell" | "bash" => Some(Language::Shell),
            "lua" => Some(Language::Lua),
            "json" => Some(Language::Json),
            _ => None,
        }
    }

    pub fn from_path_and_text(path: Option<&str>, text: &str) -> Self {
        if let Some(p) = path {
            if let Some((_, ext)) = p.rsplit_once('.') {
                if let Some(lang) = Self::parse_name(ext) {
                    return lang;
                }
            }
            if p.ends_with("Makefile") || p.ends_with(".mk") {
                return Language::Shell;
            }
        }
        let first_line = text.lines().next().unwrap_or("").trim();
        if first_line.starts_with("#!") {
            if first_line.contains("lua") {
                return Language::Lua;
            }
            return Language::Shell;
        }
        Language::PlainText
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    Type,
    Constant,
    Function,
    String,
    Number,
    Comment,
    Preprocessor,
    Operator,
    Heading,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineState {
    pub block_comment_depth: u8,
    pub in_code_fence: bool,
}

pub fn compute_line_states(text: &str, lang: Language) -> Vec<LineState> {
    let mut states = Vec::new();
    let mut state = LineState::default();
    let mut start = 0usize;
    for (idx, b) in text.bytes().enumerate() {
        if b == b'\n' {
            states.push(state);
            let (_, next) = scan_line_spans(&text[start..idx], lang, state, false);
            state = next;
            start = idx + 1;
        }
    }
    states.push(state);
    states
}

pub fn highlight_line(
    line: &str,
    lang: Language,
    entry_state: LineState,
) -> (Vec<HighlightSpan>, LineState) {
    scan_line_spans(line, lang, entry_state, true)
}

fn push_span(spans: &mut Vec<HighlightSpan>, start: usize, end: usize, kind: TokenKind) {
    if start >= end {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.kind == kind && last.end == start {
            last.end = end;
            return;
        }
    }
    spans.push(HighlightSpan { start, end, kind });
}

fn scan_line_spans(
    line: &str,
    lang: Language,
    mut state: LineState,
    emit: bool,
) -> (Vec<HighlightSpan>, LineState) {
    let mut spans = Vec::new();
    if lang == Language::PlainText {
        if emit && !line.is_empty() {
            spans.push(HighlightSpan {
                start: 0,
                end: line.len(),
                kind: TokenKind::Plain,
            });
        }
        return (spans, state);
    }
    if lang == Language::Markdown {
        return scan_markdown_line(line, state, emit);
    }

    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    let trim_start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(len);
    if state.block_comment_depth == 0 && trim_start < len {
        if (lang == Language::C && bytes[trim_start] == b'#')
            || (lang == Language::Rust
                && trim_start + 1 < len
                && bytes[trim_start] == b'#'
                && (bytes[trim_start + 1] == b'[' || bytes[trim_start + 1] == b'!'))
        {
            if lang == Language::C {
                if emit {
                    push_span(&mut spans, 0, trim_start, TokenKind::Plain);
                    // Highlight string/comment tails after directive keyword
                    let mut p = trim_start + 1;
                    while p < len && (bytes[p].is_ascii_alphabetic() || bytes[p] == b'_') {
                        p += 1;
                    }
                    push_span(&mut spans, trim_start, p, TokenKind::Preprocessor);
                    if p < len {
                        let (tail, next_st) = scan_line_spans(&line[p..], Language::C, state, true);
                        for sp in tail {
                            push_span(&mut spans, p + sp.start, p + sp.end, sp.kind);
                        }
                        state = next_st;
                    }
                }
                return (spans, state);
            }
        }
        if lang == Language::Toml && bytes[trim_start] == b'[' {
            if let Some(close) = bytes[trim_start..].iter().position(|&b| b == b']') {
                if emit {
                    push_span(&mut spans, 0, trim_start, TokenKind::Plain);
                    push_span(
                        &mut spans,
                        trim_start,
                        trim_start + close + 1,
                        TokenKind::Preprocessor,
                    );
                    let rest = trim_start + close + 1;
                    if rest < len {
                        if let Some(hash) = bytes[rest..].iter().position(|&b| b == b'#') {
                            push_span(&mut spans, rest, rest + hash, TokenKind::Plain);
                            push_span(&mut spans, rest + hash, len, TokenKind::Comment);
                        } else {
                            push_span(&mut spans, rest, len, TokenKind::Plain);
                        }
                    }
                }
                return (spans, state);
            }
        }
    }

    let mut toml_seen_eq = false;
    while i < len {
        // Multi-line block comment continuation
        if state.block_comment_depth > 0 {
            let start = i;
            if lang == Language::Lua {
                if i + 1 < len && bytes[i] == b']' && bytes[i + 1] == b']' {
                    i += 2;
                    state.block_comment_depth = 0;
                    if emit {
                        push_span(&mut spans, start, i, TokenKind::Comment);
                    }
                    continue;
                }
                while i < len && !(i + 1 < len && bytes[i] == b']' && bytes[i + 1] == b']') {
                    i += char_len_at(line, i);
                }
                if i + 1 < len && bytes[i] == b']' && bytes[i + 1] == b']' {
                    i += 2;
                    state.block_comment_depth = 0;
                }
                if emit {
                    push_span(&mut spans, start, i, TokenKind::Comment);
                }
                continue;
            }
            while i < len {
                if lang == Language::Rust && i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*'
                {
                    state.block_comment_depth = state.block_comment_depth.saturating_add(1);
                    i += 2;
                } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    state.block_comment_depth = state.block_comment_depth.saturating_sub(1);
                    if state.block_comment_depth == 0 {
                        break;
                    }
                } else {
                    i += char_len_at(line, i);
                }
            }
            if emit {
                push_span(&mut spans, start, i, TokenKind::Comment);
            }
            continue;
        }

        let b = bytes[i];

        // Comments
        if matches!(lang, Language::Rust | Language::C) && i + 1 < len && b == b'/' {
            if bytes[i + 1] == b'/' {
                if emit {
                    push_span(&mut spans, i, len, TokenKind::Comment);
                }
                break;
            }
            if bytes[i + 1] == b'*' {
                state.block_comment_depth = 1;
                let start = i;
                i += 2;
                if emit {
                    push_span(&mut spans, start, i, TokenKind::Comment);
                }
                continue;
            }
        }
        if matches!(lang, Language::Toml | Language::Shell) && b == b'#' {
            if emit {
                push_span(&mut spans, i, len, TokenKind::Comment);
            }
            break;
        }
        if lang == Language::Lua && i + 1 < len && b == b'-' && bytes[i + 1] == b'-' {
            if i + 3 < len && bytes[i + 2] == b'[' && bytes[i + 3] == b'[' {
                state.block_comment_depth = 1;
                if emit {
                    push_span(&mut spans, i, i + 4, TokenKind::Comment);
                }
                i += 4;
                continue;
            }
            if emit {
                push_span(&mut spans, i, len, TokenKind::Comment);
            }
            break;
        }

        // Rust attributes (#[...], #![...])
        if lang == Language::Rust
            && b == b'#'
            && (i + 1 < len && (bytes[i + 1] == b'[' || bytes[i + 1] == b'!'))
        {
            let start = i;
            let mut depth = 0usize;
            while i < len {
                if bytes[i] == b'[' {
                    depth += 1;
                } else if bytes[i] == b']' {
                    depth = depth.saturating_sub(1);
                    i += 1;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                i += char_len_at(line, i);
            }
            if emit {
                push_span(&mut spans, start, i, TokenKind::Preprocessor);
            }
            continue;
        }

        // Strings & char literals
        if b == b'"' || (b == b'\'' && !is_rust_lifetime(lang, bytes, i)) {
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                let c = bytes[i];
                if c == b'\\' && i + 1 < len {
                    i += 1;
                    i += char_len_at(line, i);
                    continue;
                }
                if c == quote {
                    i += 1;
                    break;
                }
                i += char_len_at(line, i);
            }
            if emit {
                push_span(&mut spans, start, i, TokenKind::String);
            }
            continue;
        }

        // Numbers
        if b.is_ascii_digit() {
            let start = i;
            if b == b'0' && i + 1 < len && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
            {
                i += 2;
            } else {
                i += 1;
            }
            while i < len
                && (bytes[i].is_ascii_hexdigit() || matches!(bytes[i], b'_' | b'.' | b'u' | b'i' | b'f'))
            {
                i += 1;
            }
            if emit {
                push_span(&mut spans, start, i, TokenKind::Number);
            }
            continue;
        }

        // Identifiers / Keywords / Types / Functions
        if is_ident_start(b) || (lang == Language::Shell && b == b'$') {
            let start = i;
            if b == b'$' {
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                if emit {
                    push_span(&mut spans, start, i, TokenKind::Constant);
                }
                continue;
            }
            while i < len && is_ident_cont(bytes[i]) {
                i += 1;
            }
            if emit {
                let word = &line[start..i];
                let after = skip_ws(bytes, i);
                let mut kind = classify_word(lang, word, toml_seen_eq);
                if kind == TokenKind::Plain && after < len {
                    if bytes[after] == b'(' || (lang == Language::Rust && bytes[after] == b'!') {
                        kind = TokenKind::Function;
                    } else if lang == Language::Json && bytes[after] == b':' {
                        kind = TokenKind::Keyword;
                    }
                }
                if kind == TokenKind::Plain && is_upper_const(word) {
                    kind = TokenKind::Constant;
                }
                push_span(&mut spans, start, i, kind);
            }
            continue;
        }

        if lang == Language::Toml && b == b'=' {
            toml_seen_eq = true;
        }

        // Operators vs punctuation / whitespace
        if b.is_ascii_whitespace() {
            let start = i;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if emit {
                push_span(&mut spans, start, i, TokenKind::Plain);
            }
            continue;
        }

        let ch_len = char_len_at(line, i);
        if emit {
            let kind = if is_operator_byte(b) {
                TokenKind::Operator
            } else {
                TokenKind::Plain
            };
            push_span(&mut spans, i, i + ch_len, kind);
        }
        i += ch_len;
    }

    (spans, state)
}

fn scan_markdown_line(
    line: &str,
    mut state: LineState,
    emit: bool,
) -> (Vec<HighlightSpan>, LineState) {
    let mut spans = Vec::new();
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        state.in_code_fence = !state.in_code_fence;
        if emit && !line.is_empty() {
            push_span(&mut spans, 0, line.len(), TokenKind::Preprocessor);
        }
        return (spans, state);
    }
    if state.in_code_fence {
        if emit && !line.is_empty() {
            push_span(&mut spans, 0, line.len(), TokenKind::String);
        }
        return (spans, state);
    }
    if !emit {
        return (spans, state);
    }
    if trimmed.starts_with('#') {
        push_span(&mut spans, 0, line.len(), TokenKind::Heading);
        return (spans, state);
    }
    if trimmed.starts_with('>') {
        push_span(&mut spans, 0, line.len(), TokenKind::Comment);
        return (spans, state);
    }
    // Highlight inline `code`, links [text](url), and bullet markers
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let lead = len - trimmed.len();
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        push_span(&mut spans, 0, lead, TokenKind::Plain);
        push_span(&mut spans, lead, lead + 1, TokenKind::Operator);
        i = lead + 1;
    }
    while i < len {
        if bytes[i] == b'`' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += char_len_at(line, i);
            }
            if i < len {
                i += 1;
            }
            push_span(&mut spans, start, i, TokenKind::String);
        } else if bytes[i] == b'[' {
            let start = i;
            if let Some(close) = line[i..].find(']') {
                push_span(&mut spans, start, start + close + 1, TokenKind::Function);
                i = start + close + 1;
            } else {
                push_span(&mut spans, i, i + 1, TokenKind::Plain);
                i += 1;
            }
        } else if bytes[i] == b'*' {
            let start = i;
            let delim = if i + 1 < len && bytes[i + 1] == b'*' { 2 } else { 1 };
            if let Some(rel) = line[i + delim..].find(&line[i..i + delim]) {
                let end = (i + delim + rel + delim).min(len);
                push_span(&mut spans, start, end, TokenKind::Keyword);
                i = end;
            } else {
                push_span(&mut spans, i, i + 1, TokenKind::Plain);
                i += 1;
            }
        } else {
            let l = char_len_at(line, i);
            push_span(&mut spans, i, i + l, TokenKind::Plain);
            i += l;
        }
    }
    (spans, state)
}

fn char_len_at(s: &str, idx: usize) -> usize {
    s[idx..].chars().next().map_or(1, char::len_utf8)
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_operator_byte(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-' | b'*' | b'/' | b'=' | b'!' | b'<' | b'>' | b'&' | b'|' | b'^' | b'%' | b':' | b'?'
    )
}

fn is_rust_lifetime(lang: Language, bytes: &[u8], i: usize) -> bool {
    if lang != Language::Rust || i + 1 >= bytes.len() {
        return false;
    }
    // Single quote followed by identifier but not closing quote at +2/+3
    if bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_' {
        let end = i + 2;
        end >= bytes.len() || bytes[end] != b'\''
    } else {
        false
    }
}

fn is_upper_const(word: &str) -> bool {
    word.len() >= 2
        && word.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && word.bytes().any(|b| b.is_ascii_uppercase())
}

fn classify_word(lang: Language, word: &str, toml_seen_eq: bool) -> TokenKind {
    match lang {
        Language::Rust => classify_rust_word(word),
        Language::C => classify_c_word(word),
        Language::Toml => {
            if matches!(word, "true" | "false") {
                TokenKind::Constant
            } else if !toml_seen_eq {
                TokenKind::Keyword
            } else {
                TokenKind::Plain
            }
        }
        Language::Shell => match word {
            "if" | "then" | "else" | "elif" | "fi" | "for" | "in" | "do" | "done" | "while"
            | "case" | "esac" | "function" | "return" | "exit" | "export" | "local" | "exec" => {
                TokenKind::Keyword
            }
            "true" | "false" => TokenKind::Constant,
            "echo" | "cd" | "ls" | "cat" | "grep" | "test" | "mkdir" | "rm" | "cp" | "mv" => {
                TokenKind::Function
            }
            _ => TokenKind::Plain,
        },
        Language::Lua => match word {
            "and" | "break" | "do" | "else" | "elseif" | "end" | "for" | "function" | "goto"
            | "if" | "in" | "local" | "not" | "or" | "repeat" | "return" | "then" | "until"
            | "while" => TokenKind::Keyword,
            "true" | "false" | "nil" => TokenKind::Constant,
            "require" | "print" | "ipairs" | "pairs" | "tostring" | "tonumber" | "table"
            | "string" | "math" => TokenKind::Function,
            _ => TokenKind::Plain,
        },
        Language::Json => match word {
            "true" | "false" | "null" => TokenKind::Constant,
            _ => TokenKind::Plain,
        },
        Language::PlainText | Language::Markdown => TokenKind::Plain,
    }
}

fn classify_rust_word(word: &str) -> TokenKind {
    match word {
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "else"
        | "enum" | "extern" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop"
        | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "static"
        | "struct" | "super" | "trait" | "type" | "unsafe" | "use" | "where" | "while"
        | "dyn" => TokenKind::Keyword,
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64"
        | "i128" | "isize" | "f32" | "f64" | "bool" | "char" | "str" | "String" | "Vec"
        | "Option" | "Result" | "Box" | "Self" | "self" => TokenKind::Type,
        "true" | "false" | "Some" | "None" | "Ok" | "Err" => TokenKind::Constant,
        _ => TokenKind::Plain,
    }
}

fn classify_c_word(word: &str) -> TokenKind {
    match word {
        "auto" | "break" | "case" | "const" | "continue" | "default" | "do" | "else"
        | "enum" | "extern" | "for" | "goto" | "if" | "inline" | "register" | "restrict"
        | "return" | "sizeof" | "static" | "struct" | "switch" | "typedef" | "union"
        | "volatile" | "while" => TokenKind::Keyword,
        "void" | "char" | "short" | "int" | "long" | "float" | "double" | "signed"
        | "unsigned" | "bool" | "_Bool" | "size_t" | "ssize_t" | "int8_t" | "int16_t"
        | "int32_t" | "int64_t" | "uint8_t" | "uint16_t" | "uint32_t" | "uint64_t"
        | "uintptr_t" | "FILE" => TokenKind::Type,
        "NULL" | "true" | "false" => TokenKind::Constant,
        _ => TokenKind::Plain,
    }
}
