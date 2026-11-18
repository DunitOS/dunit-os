//! Terminal setup uses the shared DWM parser and transactional persistence.
use alloc::{format, string::String, vec::Vec};
use dunit::io::{Read, Write};
use dwm_settings::{edit, value, Settings};

#[derive(Debug, PartialEq, Eq)]
pub enum AppError {
    Command(&'static str),
    Io(dunit::Error),
    Input(dunit::Error),
}
type AppResult<T> = Result<T, AppError>;
impl From<&'static str> for AppError {
    fn from(message: &'static str) -> Self {
        Self::Command(message)
    }
}
impl From<dunit::Error> for AppError {
    fn from(error: dunit::Error) -> Self {
        Self::Io(error)
    }
}

/// Keep one buffer across command/menu transitions so pasted input survives.
pub struct Console<R, W> {
    input: R,
    buffer: [u8; 1024],
    position: usize,
    length: usize,
    output: W,
}
impl<R: Read, W: Write> Console<R, W> {
    pub fn new(input: R, output: W) -> Self {
        Self {
            input,
            output,
            buffer: [0; 1024],
            position: 0,
            length: 0,
        }
    }
    fn print(&mut self, text: &str) -> AppResult<()> {
        self.output.write_all(text.as_bytes()).map_err(Into::into)
    }
    fn println(&mut self, text: &str) -> AppResult<()> {
        self.print(&format!("{}\n", text))
    }
    fn report(&mut self, error: AppError) -> AppResult<()> {
        match error {
            AppError::Command(message) => self.println(&format!("setup: error: {}", message)),
            AppError::Io(error) => Err(AppError::Io(error)),
            AppError::Input(error) => Err(AppError::Input(error)),
        }
    }
    /// Retain the original editor's CR/LF, Ctrl-D, UTF-8 backspace and limit.
    /// SDK buffering amortizes native reads without discarding following lines.
    pub fn read_command(&mut self) -> AppResult<Option<String>> {
        let mut bytes = Vec::new();
        loop {
            if self.position == self.length {
                self.length = self.input.read(&mut self.buffer).map_err(AppError::Input)?;
                self.position = 0;
                if self.length > self.buffer.len() {
                    return Err(AppError::Input(dunit::Error(dunit::errno::EIO)));
                }
                if self.length == 0 {
                    return Ok(None);
                }
            }
            let byte = self.buffer[self.position];
            self.position += 1;
            if byte == 4 {
                return Ok(None);
            }
            match byte {
                b'\n' | b'\r' => {
                    return String::from_utf8(bytes)
                        .map(Some)
                        .map_err(|_| AppError::Input(dunit::Error(dunit::errno::EINVAL)))
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
                        return Err(AppError::Input(dunit::Error(dunit::errno::EINVAL)));
                    }
                    bytes.push(byte);
                }
                _ => {}
            }
        }
    }
}

fn help<R: Read, W: Write>(io: &mut Console<R, W>) -> AppResult<()> {
    io.print(
        "Dunit terminal setup\n\
Commands:\n  menu                 open numbered settings menu\n\
  show [section]       show supported settings as TOML\n\
  get section.key      read a setting\n\
  set section.key VALUE  validate and stage a TOML value\n\
  import-user PATH     activate a persistent user profile\n\
  import-desktop PATH  validate and apply a complete desktop config\n\
  import PATH          stage a TOML theme/settings overlay\n\
  check                check staged serialization and on-disk config\n\
  save                 save staged settings\n\
  reload               discard edits and reload saved settings\n\
  quit                 leave without saving staged edits\n\
Examples: set effects.anim false; set effects.anim_ms 180\n\
          set theme.desktop \"#FF1E1E2E\"\n\
Application registry/startup tables are preserved by save.\n\
Saved settings apply on the next desktop session.\n",
    )?;
    Ok(())
}

pub fn command<R: Read, W: Write>(
    io: &mut Console<R, W>,
    line: &str,
    settings: &mut Settings,
    dirty: &mut bool,
    pending: &mut String,
) -> AppResult<bool> {
    let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    match cmd {
        "" => {}
        "help" => help(io)?,
        "menu" => {
            return menu(io, settings, dirty, pending);
        }
        "show" => {
            let text = dwm_settings::to_toml(settings);
            if rest.is_empty() {
                io.print(&text)?;
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
                        io.println(line)?;
                    }
                }
                if !found {
                    return Err("unknown section; use show".into());
                }
            }
        }
        "get" => {
            let text = dwm_settings::to_toml(settings);
            let v = value(&text, rest).ok_or("unknown field; use show")?;
            io.println(&format!("{} = {}", rest, v))?;
        }
        "set" => {
            let (field, input) = rest
                .split_once(char::is_whitespace)
                .ok_or("usage: set section.key TOML_VALUE")?;
            let candidate = edit(settings, field, input.trim())?;
            let (section, key) = field.split_once('.').ok_or("expected section.key")?;
            let patch = format!("[{}]\n{} = {}\n", section, key, input.trim());
            let staged = dwm_settings::apply_scalar_overlay(pending, &patch)?;
            *settings = candidate;
            *pending = staged;
            *dirty = true;
            io.println("setup: staged; use save to write")?;
        }
        "import-desktop" | "import-user" => {
            let source = dwm_settings::load_document_from(rest)
                .map_err(|_| "could not load full desktop configuration")?;
            let config = dwm_settings::validate_installed_document(&source)
                .map_err(|_| "invalid full desktop configuration")?;
            if cmd == "import-user" {
                dwm_settings::save_user_document(&source)
                    .map_err(|_| "user profile could not be saved")?;
            } else {
                dwm_settings::save_document(&source)
                    .map_err(|_| "desktop document could not be saved")?;
            }
            *settings = config.settings;
            *dirty = false;
            pending.clear();
            io.println("setup: full desktop applied")?;
        }
        "import" => {
            let (candidate, patch) = dwm_settings::load_overlay_patch(settings, rest)?;
            let staged = dwm_settings::apply_scalar_overlay(pending, &patch)?;
            *settings = candidate;
            *pending = staged;
            *dirty = true;
            io.println("setup: overlay staged; use show to review and save to write")?;
        }
        "check" => {
            if !dwm_settings::roundtrip_ok(settings) {
                return Err("staged settings fail round-trip check".into());
            }
            if !dwm_settings::load_config().valid {
                return Err("saved desktop config is invalid".into());
            }
            io.println("setup: check OK")?;
        }
        "save" => {
            if !dwm_settings::load_config().valid {
                return Err("saved config is invalid; refusing to overwrite it".into());
            }
            if !dwm_settings::roundtrip_ok(settings) {
                return Err("staged settings fail validation".into());
            }
            if !pending.is_empty() && !dwm_settings::save_overlay(pending) {
                return Err("could not save settings".into());
            }
            let saved = dwm_settings::load_config();
            if !saved.valid
                || (!pending.is_empty()
                    && dwm_settings::apply_overlay(&saved.settings, pending).map(|expected| {
                        dwm_settings::to_toml(&expected) == dwm_settings::to_toml(&saved.settings)
                    }) != Ok(true))
            {
                return Err("saved settings failed read-back verification".into());
            }
            *settings = saved.settings;
            pending.clear();
            *dirty = false;
            io.println("setup: saved; applies on next desktop session")?;
        }
        "reload" => {
            let config = dwm_settings::load_config();
            if !config.valid {
                return Err("saved config is invalid; staged settings retained".into());
            }
            *settings = config.settings;
            pending.clear();
            *dirty = false;
            io.println("setup: reloaded")?;
        }
        "quit" | "exit" => {
            return Ok(false);
        }
        _ => return Err("unknown command; use help".into()),
    }
    Ok(true)
}

/// Section and field choices come from the shared serializer, so new supported
/// scalar settings appear here without a parallel menu schema.
pub fn menu<R: Read, W: Write>(
    io: &mut Console<R, W>,
    settings: &mut Settings,
    dirty: &mut bool,
    pending: &mut String,
) -> AppResult<bool> {
    loop {
        let text = dwm_settings::to_toml(settings);
        let sections: Vec<&str> = text
            .lines()
            .filter_map(|line| line.strip_prefix('[').and_then(|s| s.strip_suffix(']')))
            .filter(|s| *s != "meta")
            .collect();
        io.println("\nSettings sections:")?;
        for (i, section) in sections.iter().enumerate() {
            io.println(&format!("  {}. {}", i + 1, section))?;
        }
        io.print("s. Save  c. Commands  q. Quit\nSelect: ")?;
        let Some(choice) = io.read_command()? else {
            return Ok(false);
        };
        match choice.trim() {
            "q" | "quit" => return Ok(false),
            "c" | "commands" => return Ok(true),
            "s" | "save" => {
                if let Err(error) = command(io, "save", settings, dirty, pending) {
                    io.report(error)?;
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
            io.println("setup: choose a section number, s, c or q")?;
            continue;
        };
        let Some(section) = sections.get(index) else {
            io.println("setup: section number out of range")?;
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
            io.println(&format!("\n[{}]", section))?;
            for (i, (field, value)) in fields.iter().enumerate() {
                io.println(&format!("  {}. {} = {}", i + 1, field, value))?;
            }
            io.print("b. Back\nSelect field: ")?;
            let Some(choice) = io.read_command()? else {
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
                io.println("setup: choose a field number or b")?;
                continue;
            };
            io.print(&format!(
                "{} = {}\nNew TOML value (empty cancels): ",
                field, old
            ))?;
            let Some(input) = io.read_command()? else {
                return Ok(false);
            };
            if input.trim().is_empty() {
                continue;
            }
            let update = format!("set {} {}", field, input.trim());
            if let Err(error) = command(io, &update, settings, dirty, pending) {
                io.report(error)?;
            }
        }
    }
}

/// CLI and menu share the same session, including unsaved overlays and input.
pub fn run<R: Read, W: Write>(io: &mut Console<R, W>, arguments: &[String]) -> AppResult<()> {
    let config = dwm_settings::load_config();
    let mut settings = config.settings;
    let mut dirty = false;
    let mut pending = String::new();
    if !config.valid {
        io.println("setup: warning: saved config is invalid; save is disabled")?;
    }
    if !arguments.is_empty() {
        let line = arguments.join(" ");
        let result = command(io, &line, &mut settings, &mut dirty, &mut pending).and_then(|_| {
            if dirty {
                command(io, "save", &mut settings, &mut dirty, &mut pending)?;
            }
            Ok(())
        });
        return match result {
            Err(AppError::Command(message)) => {
                io.report(AppError::Command(message))?;
                Err(AppError::Command(message))
            }
            result => result,
        };
    }
    if menu(io, &mut settings, &mut dirty, &mut pending)? {
        help(io)?;
        loop {
            io.print(if dirty { "setup* > " } else { "setup > " })?;
            let Some(line) = io.read_command()? else {
                break;
            };
            match command(io, line.trim(), &mut settings, &mut dirty, &mut pending) {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => io.report(error)?,
            }
        }
    }
    if dirty {
        io.println("setup: leaving; staged edits were not saved")?;
    }
    Ok(())
}
