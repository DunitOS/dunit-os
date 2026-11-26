//! Bounded block/inline Markdown and plaintext parser for read-only typography views.
use alloc::{format, string::String, vec::Vec};

pub const MAX_BLOCKS: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocBlock {
    Heading { level: u8, text: String },
    Paragraph(String),
    Bullet { indent: u8, text: String },
    Numbered { index: usize, text: String },
    Quote(String),
    CodeBlock { lang: String, text: String },
    Rule,
}

pub fn parse_markdown(input: &str) -> Vec<DocBlock> {
    let mut blocks = Vec::new();
    let mut paragraph = Vec::new();
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_lines = Vec::new();

    let flush_para = |para: &mut Vec<String>, out: &mut Vec<DocBlock>| {
        if !para.is_empty() && out.len() < MAX_BLOCKS {
            out.push(DocBlock::Paragraph(format_inline(&para.join(" "))));
            para.clear();
        }
    };

    for raw_line in input.lines() {
        let line = raw_line.trim_end_matches('\r');
        if blocks.len() >= MAX_BLOCKS {
            break;
        }
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_code {
                blocks.push(DocBlock::CodeBlock {
                    lang: core::mem::take(&mut code_lang),
                    text: code_lines.join("\n"),
                });
                code_lines.clear();
                in_code = false;
            } else {
                flush_para(&mut paragraph, &mut blocks);
                code_lang = String::from(rest.trim());
                in_code = true;
            }
            continue;
        }
        if in_code {
            code_lines.push(String::from(line));
            continue;
        }
        if trimmed.is_empty() {
            flush_para(&mut paragraph, &mut blocks);
            continue;
        }
        if is_rule(trimmed) {
            flush_para(&mut paragraph, &mut blocks);
            blocks.push(DocBlock::Rule);
            continue;
        }
        if let Some((level, body)) = parse_heading(trimmed) {
            flush_para(&mut paragraph, &mut blocks);
            blocks.push(DocBlock::Heading {
                level,
                text: format_inline(body),
            });
            continue;
        }
        if let Some(quote) = trimmed.strip_prefix("> ") {
            flush_para(&mut paragraph, &mut blocks);
            blocks.push(DocBlock::Quote(format_inline(quote)));
            continue;
        }
        if let Some((indent, item)) = parse_bullet(line) {
            flush_para(&mut paragraph, &mut blocks);
            blocks.push(DocBlock::Bullet {
                indent,
                text: format_inline(item),
            });
            continue;
        }
        if let Some((num, item)) = parse_numbered(trimmed) {
            flush_para(&mut paragraph, &mut blocks);
            blocks.push(DocBlock::Numbered {
                index: num,
                text: format_inline(item),
            });
            continue;
        }
        paragraph.push(String::from(trimmed));
    }

    if in_code && blocks.len() < MAX_BLOCKS {
        blocks.push(DocBlock::CodeBlock {
            lang: code_lang,
            text: code_lines.join("\n"),
        });
    }
    flush_para(&mut paragraph, &mut blocks);
    blocks
}

fn is_rule(line: &str) -> bool {
    line.len() >= 3 && (line.chars().all(|c| c == '-') || line.chars().all(|c| c == '*'))
}

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some((hashes.min(3) as u8, rest.trim()))
}

fn parse_bullet(line: &str) -> Option<(u8, &str)> {
    let spaces = line.bytes().take_while(|&b| b == b' ').count();
    let rest = &line[spaces..];
    let item = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))?;
    Some(((spaces / 2).min(3) as u8, item.trim()))
}

fn parse_numbered(line: &str) -> Option<(usize, &str)> {
    let dot = line.find(". ")?;
    if dot == 0 || dot > 6 {
        return None;
    }
    let num = line[..dot].parse::<usize>().ok()?;
    Some((num, line[dot + 2..].trim()))
}

/// Normalize simple Markdown inline syntax (`**bold**`, `*em*`, `` `code` ``, `[label](url)`)
/// into readable typography without dropping URL destinations.
pub fn format_inline(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let bytes = input.as_bytes();
    while i < bytes.len() {
        if input[i..].starts_with("**") || input[i..].starts_with("__") {
            let marker = &input[i..i + 2];
            if let Some(end) = input[i + 2..].find(marker) {
                out.push_str(&input[i + 2..i + 2 + end]);
                i += 2 + end + 2;
                continue;
            }
        }
        if bytes[i] == b'`' {
            if let Some(end) = input[i + 1..].find('`') {
                out.push_str(&input[i + 1..i + 1 + end]);
                i += 1 + end + 1;
                continue;
            }
        }
        if bytes[i] == b'[' {
            if let Some(close_br) = input[i + 1..].find("](") {
                let label = &input[i + 1..i + 1 + close_br];
                let url_start = i + 1 + close_br + 2;
                if let Some(close_par) = input[url_start..].find(')') {
                    let url = &input[url_start..url_start + close_par];
                    out.push_str(&format!("{} ({})", label, url));
                    i = url_start + close_par + 1;
                    continue;
                }
            }
        }
        let ch = input[i..].chars().next().unwrap_or('\0');
        if ch == '*' || ch == '_' {
            let close = input[i + 1..].find(ch);
            if let Some(end) = close {
                if end > 0 && !input[i + 1..].starts_with(' ') {
                    out.push_str(&input[i + 1..i + 1 + end]);
                    i += 1 + end + 1;
                    continue;
                }
            }
        }
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}
