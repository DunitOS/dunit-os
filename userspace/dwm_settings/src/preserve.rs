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
    fn open(&self) -> bool { self.depth != 0 || self.quote != 0 }

    fn feed(&mut self, line: &str) {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let byte = bytes[i];
            if self.escaped { self.escaped = false; i += 1; continue; }
            if self.quote == b'"' && byte == b'\\' {
                self.escaped = true;
                i += 1;
                continue;
            }
            if self.quote != 0 {
                if byte == self.quote {
                    if !self.triple { self.quote = 0; }
                    else if bytes.get(i..i + 3) == Some(&[byte, byte, byte]) {
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
                if self.triple { i += 2; }
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
        if escaped { escaped = false; continue; }
        if quote == b'"' && byte == b'\\' { escaped = true; continue; }
        if quote != 0 {
            if byte == quote { quote = 0; }
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
    let (name, array) = if let Some(name) = line.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        (name, true)
    } else {
        (line.strip_prefix('[')?.strip_suffix(']')?, false)
    };
    let name = name.trim();
    if name.is_empty() || name.contains(['=', ',', '[', ']']) { return None; }
    Some((name, array))
}

fn key(line: &str) -> Option<&str> {
    let line = uncomment(line);
    let mut quote = 0;
    let mut escaped = false;
    for (i, byte) in line.bytes().enumerate() {
        if escaped { escaped = false; continue; }
        if quote == b'"' && byte == b'\\' { escaped = true; continue; }
        if quote != 0 {
            if byte == quote { quote = 0; }
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
            out.push(Section { name, array, raw: &text[start..offset], body: &text[body_start..offset] });
            start = offset;
            body_start = offset + line.len();
            name = next_name;
            array = next_array;
        } else {
            state.feed(line);
        }
        offset += line.len();
    }
    out.push(Section { name, array, raw: &text[start..], body: &text[body_start..] });
    out
}

fn extras(body: &str, owned: &[&str]) -> String {
    let mut out = String::new();
    let mut keep = false;
    let mut state = ValueState::default();
    for line in body.split_inclusive('\n') {
        if let Some(name) = if state.open() { None } else { key(line) } {
            keep = !owned.contains(&name);
        }
        if keep { out.push_str(line); }
        state.feed(line);
    }
    out
}

fn append(out: &mut String, text: &str) {
    if text.is_empty() { return; }
    if !out.is_empty() && !out.ends_with('\n') { out.push('\n'); }
    out.push_str(text);
}

/// Merge unowned assignments into their existing canonical table, then append
/// wholly unowned tables verbatim. Serializer-owned keys always win.
pub(super) fn merge(canonical: &str, existing: &str) -> String {
    let canonical_sections = sections(canonical);
    let existing_sections = sections(existing);
    let mut out = String::new();
    for section in &canonical_sections {
        append(&mut out, section.raw);
        let owned: Vec<&str> = section.body.lines().filter_map(key).collect();
        for old in &existing_sections {
            if old.name == section.name && old.array == section.array {
                append(&mut out, &extras(old.body, &owned));
            }
        }
    }
    for old in &existing_sections {
        if !canonical_sections.iter().any(|s| s.name == old.name && s.array == old.array) {
            append(&mut out, old.raw);
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
        let merged = merge(canonical, existing);
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
        let merged = merge("[theme]\ncolor = 2\n", &existing);
        assert!(merged.ends_with(extra));
        assert_eq!(merged.matches("[[application]]").count(), 2);
    }

    #[test]
    fn ownership_is_derived_and_quoted_comments_are_not_syntax() {
        let canonical = "[newtable]\nnew_key = 9\n";
        let old = "root = 'keep'\n[newtable]\nnew_key = 1\nextra = \"escaped \\\" # still value\" # comment\nlist = [\n 'first',\n 'second'\n]\n";
        let merged = merge(canonical, old);
        assert!(merged.starts_with("root = 'keep'\n[newtable]\nnew_key = 9\n"));
        assert!(merged.contains("extra = \"escaped \\\" # still value\" # comment"));
        assert!(merged.ends_with("list = [\n 'first',\n 'second'\n]\n"));
        assert!(!merged.contains("new_key = 1"));
    }
}
