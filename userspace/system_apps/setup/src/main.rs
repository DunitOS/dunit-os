#![no_std]
#![no_main]

extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use core::panic::PanicInfo;
use dwm_settings::Settings;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    libdunit::println("setup: fatal error");
    libdunit::exit(1)
}

/// Enumerate the serializer's actual fields, avoiding a second settings schema.
fn value<'a>(document: &'a str, field: &str) -> Option<&'a str> {
    let (wanted_section, wanted_key) = field.split_once('.')?;
    let mut section = "";
    for line in document.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = s;
        } else if section == wanted_section {
            if let Some((key, v)) = line.split_once('=') {
                if key.trim() == wanted_key {
                    return Some(v.trim());
                }
            }
        }
    }
    None
}

fn scalar(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

fn edit(current: &Settings, field: &str, input: &str) -> Result<Settings, &'static str> {
    if field.starts_with("meta.") {
        return Err("schema metadata is read-only");
    }
    if input.is_empty() || input.chars().any(|c| c.is_control()) {
        return Err("enter a non-empty, single-line TOML value");
    }
    let document = dwm_settings::to_toml(current);
    let old = value(&document, field).ok_or("unknown field; use show to list supported fields")?;
    // Strings use the same TOML representation displayed by show/get.
    if old.starts_with('"') && !(input.starts_with('"') && input.ends_with('"')) {
        return Err("string/color values must be enclosed in double quotes");
    }
    let (section, key) = field.split_once('.').ok_or("expected section.key")?;
    let patch = format!("[{}]\n{} = {}\n", section, key, input);
    let mut candidate = *current;
    dwm_settings::parse_into(&patch, &mut candidate);
    let encoded = dwm_settings::to_toml(&candidate);
    let actual = value(&encoded, field).ok_or("field is not serializable")?;
    let requested = scalar(input);
    let resolved = scalar(actual);
    // Reject silently ignored or clamped input. Canonical colors are case-insensitive.
    let matches = if requested.starts_with('#') {
        requested.eq_ignore_ascii_case(resolved)
            || (requested.len() == 7
                && resolved.len() == 9
                && resolved[1..3].eq_ignore_ascii_case("FF")
                && requested[1..].eq_ignore_ascii_case(&resolved[3..]))
    } else {
        requested == resolved
    };
    if !matches {
        return Err("invalid value or out of range; value was not changed");
    }
    if !dwm_settings::roundtrip_ok(&candidate) {
        return Err("settings failed serialization validation");
    }
    Ok(candidate)
}

/// Import is bounded and rejects I/O errors rather than accepting a prefix.
fn read_overlay(path: &str) -> Result<String, &'static str> {
    if !path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err("overlay path must be absolute without parent traversal");
    }
    let fd = libdunit::open(path, libdunit::OPEN_READ);
    if fd < 0 {
        return Err("could not open overlay");
    }
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0u8; 512];
        let count = libdunit::read(fd as usize, &mut chunk);
        if count < 0 {
            libdunit::close(fd as usize);
            return Err("overlay read failed");
        }
        if count == 0 {
            break;
        }
        if bytes.len() + count as usize > 64 * 1024 {
            libdunit::close(fd as usize);
            return Err("overlay exceeds 64 KiB");
        }
        bytes.extend_from_slice(&chunk[..count as usize]);
    }
    if libdunit::close(fd as usize) < 0 {
        return Err("overlay close failed");
    }
    String::from_utf8(bytes).map_err(|_| "overlay is not UTF-8")
}

fn help() {
    libdunit::print(
        "Dunit terminal setup\n\
Commands:\n  menu                 open numbered settings menu\n\
  show [section]       show supported settings as TOML\n\
  get section.key      read a setting\n\
  set section.key VALUE  validate and stage a TOML value\n\
  import PATH          stage a TOML theme/settings overlay\n\
  check                check staged serialization and on-disk config\n\
  save                 save staged settings\n\
  reload               discard edits and reload saved settings\n\
  quit                 leave without saving staged edits\n\
Examples: set effects.anim false; set effects.anim_ms 180\n\
          set theme.desktop \"#FF1E1E2E\"\n\
Application registry/startup tables are preserved by save.\n\
Saved settings apply on the next desktop session.\n",
    );
}

fn command(line: &str, settings: &mut Settings, dirty: &mut bool) -> Result<bool, &'static str> {
    let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    match cmd {
        "" => {}
        "help" => help(),
        "menu" => {
            return menu(settings, dirty).map_err(|_| "menu input failed");
        }
        "show" => {
            let text = dwm_settings::to_toml(settings);
            if rest.is_empty() {
                libdunit::print(&text);
            } else {
                let mut selected = false;
                let mut found = false;
                for line in text.lines() {
                    if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
                    {
                        selected = section == rest;
                        found |= selected;
                    }
                    if selected {
                        libdunit::println(line);
                    }
                }
                if !found {
                    return Err("unknown section; use show");
                }
            }
        }
        "get" => {
            let text = dwm_settings::to_toml(settings);
            let v = value(&text, rest).ok_or("unknown field; use show")?;
            libdunit::println(&format!("{} = {}", rest, v));
        }
        "set" => {
            let (field, input) = rest
                .split_once(char::is_whitespace)
                .ok_or("usage: set section.key TOML_VALUE")?;
            *settings = edit(settings, field, input.trim())?;
            *dirty = true;
            libdunit::println("setup: staged; use save to write");
        }
        "import" => {
            let text = read_overlay(rest)?;
            let mut candidate = *settings;
            let mut section = "";
            let mut count = 0usize;
            for raw in text.lines() {
                // Match the shared TOML subset: # and // comments outside double quotes.
                let mut quoted = false;
                let bytes = raw.as_bytes();
                let mut end = bytes.len();
                for (i, byte) in bytes.iter().enumerate() {
                    if *byte == b'"' {
                        quoted = !quoted;
                    }
                    if !quoted
                        && (*byte == b'#' || (*byte == b'/' && bytes.get(i + 1) == Some(&b'/')))
                    {
                        end = i;
                        break;
                    }
                }
                if quoted {
                    return Err("overlay contains an unterminated string");
                }
                let line = raw[..end].trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                    section = name.trim();
                    continue;
                }
                let (key, input) = line
                    .split_once('=')
                    .ok_or("overlay contains a malformed line")?;
                let field = format!("{}.{}", section, key.trim());
                if field == "meta.schema_version" {
                    let version = input
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| "invalid overlay schema")?;
                    if !dwm_settings::version_supported(version) {
                        return Err("unsupported overlay schema");
                    }
                    continue; // overlays cannot replace the running schema metadata
                }
                candidate = edit(&candidate, &field, input.trim())?;
                count += 1;
            }
            if count == 0 {
                return Err("overlay contains no supported settings");
            }
            *settings = candidate;
            *dirty = true;
            libdunit::println("setup: overlay staged; use show to review and save to write");
        }
        "check" => {
            if !dwm_settings::roundtrip_ok(settings) {
                return Err("staged settings fail round-trip check");
            }
            if !dwm_settings::load_config().valid {
                return Err("saved desktop config is invalid");
            }
            libdunit::println("setup: check OK");
        }
        "save" => {
            if !dwm_settings::load_config().valid {
                return Err("saved config is invalid; refusing to overwrite it");
            }
            if !dwm_settings::roundtrip_ok(settings) {
                return Err("staged settings fail validation");
            }
            if !dwm_settings::save(settings) {
                return Err("could not save settings");
            }
            let saved = dwm_settings::load_config();
            if !saved.valid
                || dwm_settings::to_toml(&saved.settings) != dwm_settings::to_toml(settings)
            {
                return Err("saved settings failed read-back verification");
            }
            *dirty = false;
            libdunit::println("setup: saved; applies on next desktop session");
        }
        "reload" => {
            let config = dwm_settings::load_config();
            if !config.valid {
                return Err("saved config is invalid; staged settings retained");
            }
            *settings = config.settings;
            *dirty = false;
            libdunit::println("setup: reloaded");
        }
        "quit" | "exit" => {
            return Ok(false);
        }
        _ => return Err("unknown command; use help"),
    }
    Ok(true)
}

/// Section and field choices come from the shared serializer, so new supported
/// scalar settings appear here without a parallel menu schema.
fn menu(settings: &mut Settings, dirty: &mut bool) -> Result<bool, isize> {
    loop {
        let text = dwm_settings::to_toml(settings);
        let sections: Vec<&str> = text
            .lines()
            .filter_map(|line| line.strip_prefix('[').and_then(|s| s.strip_suffix(']')))
            .filter(|s| *s != "meta")
            .collect();
        libdunit::println("\nSettings sections:");
        for (i, section) in sections.iter().enumerate() {
            libdunit::println(&format!("  {}. {}", i + 1, section));
        }
        libdunit::print("s. Save  c. Commands  q. Quit\nSelect: ");
        let Some(choice) = read_command()? else {
            return Ok(false);
        };
        match choice.trim() {
            "q" | "quit" => return Ok(false),
            "c" | "commands" => return Ok(true),
            "s" | "save" => {
                if let Err(error) = command("save", settings, dirty) {
                    libdunit::println(&format!("setup: error: {}", error));
                }
                continue;
            }
            _ => {}
        }
        let Some(index) = choice
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| i.checked_sub(1))
        else {
            libdunit::println("setup: choose a section number, s, c or q");
            continue;
        };
        let Some(section) = sections.get(index) else {
            libdunit::println("setup: section number out of range");
            continue;
        };
        loop {
            let document = dwm_settings::to_toml(settings);
            let mut selected = false;
            let mut fields: Vec<(String, &str)> = Vec::new();
            for line in document.lines() {
                if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                    selected = name == *section;
                } else if selected {
                    if let Some((key, value)) = line.split_once('=') {
                        fields.push((format!("{}.{}", section, key.trim()), value.trim()));
                    }
                }
            }
            libdunit::println(&format!("\n[{}]", section));
            for (i, (field, value)) in fields.iter().enumerate() {
                libdunit::println(&format!("  {}. {} = {}", i + 1, field, value));
            }
            libdunit::print("b. Back\nSelect field: ");
            let Some(choice) = read_command()? else {
                return Ok(false);
            };
            if choice.trim() == "b" {
                break;
            }
            let field = choice
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| fields.get(i));
            let Some((field, old)) = field else {
                libdunit::println("setup: choose a field number or b");
                continue;
            };
            libdunit::print(&format!(
                "{} = {}\nNew TOML value (empty cancels): ",
                field, old
            ));
            let Some(input) = read_command()? else {
                return Ok(false);
            };
            if input.trim().is_empty() {
                continue;
            }
            match edit(settings, field, input.trim()) {
                Ok(candidate) => {
                    *settings = candidate;
                    *dirty = true;
                    libdunit::println("setup: staged");
                }
                Err(error) => libdunit::println(&format!("setup: error: {}", error)),
            }
        }
    }
}

/// Read one byte at a time so multiple pasted lines are never lost. Works on
/// the console and on a PTY; input echo belongs to the console/shell line discipline.
/// Ctrl-D exits and backspace edits the current line.
fn read_command() -> Result<Option<String>, isize> {
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        let n = libdunit::read(0, &mut byte);
        if n == libdunit::EAGAIN {
            libdunit::yield_now();
            continue;
        }
        if n < 0 {
            return Err(n);
        }
        if n == 0 || byte[0] == 4 {
            return Ok(None);
        }
        match byte[0] {
            b'\n' | b'\r' => {
                return String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| libdunit::EINVAL);
            }
            8 | 127 => {
                if let Some(last) = bytes.pop() {
                    if last & 0xC0 == 0x80 {
                        while let Some(previous) = bytes.pop() {
                            if previous & 0xC0 != 0x80 {
                                break;
                            }
                        }
                    }
                }
            }
            32..=126 | 128..=255 => {
                if bytes.len() >= 512 {
                    return Err(libdunit::EINVAL);
                }
                bytes.push(byte[0]);
            }
            _ => {}
        }
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: usize, argv: libdunit::RawArgv, envp: libdunit::RawEnvp) -> ! {
    libdunit::init_runtime(argc, argv, envp);
    let config = dwm_settings::load_config();
    let mut settings = config.settings;
    let mut dirty = false;
    if !config.valid {
        libdunit::println("setup: warning: saved config is invalid; save is disabled");
    }
    if argc > 1 {
        let mut line = String::new();
        for i in 1..argc {
            if i > 1 {
                line.push(' ');
            }
            line.push_str(libdunit::arg(i).unwrap_or(""));
        }
        match command(&line, &mut settings, &mut dirty) {
            Ok(_) => {
                if dirty {
                    if let Err(error) = command("save", &mut settings, &mut dirty) {
                        libdunit::println(&format!("setup: error: {}", error));
                        libdunit::exit(1);
                    }
                }
                libdunit::exit(0)
            }
            Err(error) => {
                libdunit::println(&format!("setup: error: {}", error));
                libdunit::exit(1);
            }
        }
    }
    match menu(&mut settings, &mut dirty) {
        Ok(true) => help(),
        Ok(false) => {
            if dirty {
                libdunit::println("setup: leaving; staged edits were not saved");
            }
            libdunit::exit(0);
        }
        Err(errno) => {
            libdunit::println(&format!("setup: input error {}", errno));
            libdunit::exit(1);
        }
    }
    loop {
        libdunit::print(if dirty { "setup* > " } else { "setup > " });
        match read_command() {
            Ok(Some(line)) => match command(line.trim(), &mut settings, &mut dirty) {
                Ok(true) => {}
                Ok(false) => {
                    if dirty {
                        libdunit::println("setup: leaving; staged edits were not saved");
                    }
                    libdunit::exit(0);
                }
                Err(error) => libdunit::println(&format!("setup: error: {}", error)),
            },
            Ok(None) => {
                if dirty {
                    libdunit::println("setup: leaving; staged edits were not saved");
                }
                libdunit::exit(0);
            }
            Err(errno) => {
                libdunit::println(&format!("setup: input error {}", errno));
                libdunit::exit(1);
            }
        }
    }
}
