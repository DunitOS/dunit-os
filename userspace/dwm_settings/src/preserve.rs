//! Preserve file-owned settings without reopening tables emitted by the serializer.
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

struct Section<'a> {
    name: &'a str,
    array: bool,
    raw: &'a str,
    body: &'a str,
}

#[derive(Default)]
struct ValueState {
    depth: usize,
    quote: u8,
    triple: bool,
    escaped: bool,
}

impl ValueState {
    fn open(&self) -> bool {
        self.depth != 0 || self.quote != 0
    }

    fn feed(&mut self, line: &str) {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let byte = bytes[i];
            if self.escaped {
                self.escaped = false;
                i += 1;
                continue;
            }
            if self.quote == b'"' && byte == b'\\' {
                self.escaped = true;
                i += 1;
                continue;
            }
            if self.quote != 0 {
                if byte == self.quote {
                    if !self.triple {
                        self.quote = 0;
                    } else if bytes.get(i..i + 3) == Some(&[byte, byte, byte]) {
                        self.quote = 0;
                        self.triple = false;
                        i += 2;
                    }
                }
            } else if byte == b'#' || (byte == b'/' && bytes.get(i + 1) == Some(&b'/')) {
                break;
            } else if byte == b'"' || byte == b'\'' {
                self.quote = byte;
                self.triple = bytes.get(i..i + 3) == Some(&[byte, byte, byte]);
                if self.triple {
                    i += 2;
                }
            } else if byte == b'[' || byte == b'{' {
                self.depth += 1;
            } else if byte == b']' || byte == b'}' {
                self.depth = self.depth.saturating_sub(1);
            }
            i += 1;
        }
    }
}

// Only structural syntax is inspected. Values and preserved sections stay raw.
fn uncomment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote = 0;
    let mut escaped = false;
    for (i, &byte) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == b'"' && byte == b'\\' {
            escaped = true;
            continue;
        }
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if byte == b'"' || byte == b'\'' {
            quote = byte;
        } else if byte == b'#' || (byte == b'/' && bytes.get(i + 1) == Some(&b'/')) {
            return &line[..i];
        }
    }
    line
}

fn header(line: &str) -> Option<(&str, bool)> {
    let line = uncomment(line).trim();
    let (name, array) =
        if let Some(name) = line.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
            (name, true)
        } else {
            (line.strip_prefix('[')?.strip_suffix(']')?, false)
        };
    let name = name.trim();
    if name.is_empty() || name.contains(['=', ',', '[', ']']) {
        return None;
    }
    Some((name, array))
}

fn key(line: &str) -> Option<&str> {
    let line = uncomment(line);
    let mut quote = 0;
    let mut escaped = false;
    for (i, byte) in line.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == b'"' && byte == b'\\' {
            escaped = true;
            continue;
        }
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if byte == b'"' || byte == b'\'' {
            quote = byte;
        } else if byte == b'=' {
            let key = line[..i].trim();
            return if key.is_empty() { None } else { Some(key) };
        }
    }
    None
}

fn sections(text: &str) -> Vec<Section<'_>> {
    let mut out = Vec::new();
    let (mut start, mut body_start, mut name, mut array) = (0, 0, "", false);
    let mut offset = 0;
    let mut state = ValueState::default();
    for line in text.split_inclusive('\n') {
        if let Some((next_name, next_array)) = if state.open() { None } else { header(line) } {
            out.push(Section {
                name,
                array,
                raw: &text[start..offset],
                body: &text[body_start..offset],
            });
            start = offset;
            body_start = offset + line.len();
            name = next_name;
            array = next_array;
        } else {
            state.feed(line);
        }
        offset += line.len();
    }
    out.push(Section {
        name,
        array,
        raw: &text[start..],
        body: &text[body_start..],
    });
    out
}

fn append(out: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(text);
}

/// Select explicit scalar declarations from the authoritative serializer.
pub(super) fn select(canonical: &str, fields: &[String]) -> String {
    let mut out = String::new();
    for section in sections(canonical) {
        let mut body = String::new();
        for line in section.body.split_inclusive('\n') {
            if let Some(name) = key(line) {
                let field = alloc::format!("{}.{}", section.name, name);
                if fields.contains(&field) {
                    append(&mut body, line);
                }
            }
        }
        if !body.is_empty() {
            append(
                &mut out,
                &section.raw[..section.raw.len() - section.body.len()],
            );
            append(&mut out, &body);
        }
    }
    out
}

/// Canonical changed fields only; unchanged implicit values stay implicit.
pub(super) fn changes(before: &str, after: &str) -> String {
    let old = sections(before);
    let mut fields = Vec::new();
    for section in sections(after) {
        for line in section.body.lines() {
            if let Some(name) = key(line) {
                let prior = old
                    .iter()
                    .find(|s| s.name == section.name)
                    .and_then(|s| s.body.lines().find(|line| key(line) == Some(name)));
                if prior.map(str::trim) != Some(line.trim()) {
                    fields.push(alloc::format!("{}.{}", section.name, name));
                }
            }
        }
    }
    select(after, &fields)
}

/// Replace only owned declarations, retaining table order, leading comments,
/// unrelated assignments and independent/array tables byte-for-byte.
pub(super) fn patch(existing: &str, updates: &str) -> String {
    if updates.is_empty() {
        return existing.into();
    }
    let edits = sections(updates);
    let old = sections(existing);
    let mut out = String::new();
    for section in &old {
        let Some(edit) = edits
            .iter()
            .find(|e| e.name == section.name && e.array == section.array)
        else {
            append(&mut out, section.raw);
            continue;
        };
        append(
            &mut out,
            &section.raw[..section.raw.len() - section.body.len()],
        );
        let replacements: Vec<_> = edit
            .body
            .split_inclusive('\n')
            .filter_map(|line| key(line).map(|name| (name, line)))
            .collect();
        let mut replaced = Vec::new();
        let mut skip = false;
        let mut state = ValueState::default();
        for line in section.body.split_inclusive('\n') {
            if !state.open() {
                skip = false;
                if let Some(name) = key(line) {
                    if let Some((_, value)) = replacements.iter().find(|(field, _)| *field == name)
                    {
                        append(&mut out, value);
                        replaced.push(name);
                        skip = true;
                    }
                }
            }
            if !skip {
                append(&mut out, line);
            }
            state.feed(line);
        }
        for (field, value) in replacements {
            if !replaced.contains(&field) {
                append(&mut out, value);
            }
        }
    }
    for edit in &edits {
        if !old
            .iter()
            .any(|s| s.name == edit.name && s.array == edit.array)
        {
            append(&mut out, edit.raw);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_stay_in_single_canonical_table() {
        let canonical = "[theme]\ncolor = 2\n[layout]\nwidth = 10\n";
        let existing = "[theme] # user theme\ncolor = 1\ncustom = '#fff' # keep\n[layout]\nwidth = 4\nfuture = true\n";
        let merged = patch(existing, canonical);
        assert_eq!(merged.matches("[theme]").count(), 1);
        assert_eq!(merged.matches("[layout]").count(), 1);
        assert!(merged.contains("color = 2\ncustom = '#fff' # keep\n[layout]"));
        assert!(merged.ends_with("width = 10\nfuture = true\n"));
        assert!(!merged.contains("color = 1"));
        assert!(!merged.contains("width = 4"));
    }

    #[test]
    fn nested_and_registry_array_tables_stay_verbatim() {
        let extra = "[theme.future]\npath = \"a#b//c\"\n\n[[application]]\nname = 'one'\nargs = [\n 'a',\n 'b'\n]\n[[application]]\nname = 'two'";
        let existing = alloc::format!("[theme]\ncolor = 1\n{extra}");
        let merged = patch(&existing, "[theme]\ncolor = 2\n");
        assert!(merged.ends_with(extra));
        assert_eq!(merged.matches("[[application]]").count(), 2);
    }

    #[test]
    fn ownership_is_derived_and_quoted_comments_are_not_syntax() {
        let canonical = "[newtable]\nnew_key = 9\n";
        let old = "root = 'keep'\n[newtable]\nnew_key = 1\nextra = \"escaped \\\" # still value\" # comment\nlist = [\n 'first',\n 'second'\n]\n";
        let merged = patch(old, canonical);
        assert!(merged.starts_with("root = 'keep'\n[newtable]\nnew_key = 9\n"));
        assert!(merged.contains("extra = \"escaped \\\" # still value\" # comment"));
        assert!(merged.ends_with("list = [\n 'first',\n 'second'\n]\n"));
        assert!(!merged.contains("new_key = 1"));
    }
}
