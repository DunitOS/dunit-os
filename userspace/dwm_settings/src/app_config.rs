//! Typed application and widget settings loaded from their own documents.
use super::*;

/// Directory holding per-app config files (`<id>.toml`).
pub const APPS_DIR: &str = "/system/share/dwm/apps";
/// Directory holding per-widget config files (`<name>.toml`).
pub const WIDGETS_DIR: &str = "/system/share/dwm/widgets";

/// Build "<dir>/<name>.toml".
fn config_path(dir: &str, name: &str) -> String {
    let mut p = String::from(dir);
    p.push('/');
    p.push_str(name);
    p.push_str(".toml");
    p
}

fn read_standalone(path: &str) -> Option<String> {
    match read_file_checked(path) {
        Ok(text) => Some(text),
        Err(libdunit::ENOENT) => None,
        Err(errno) => {
            libdunit::println(&alloc::format!("dwm_settings: warning path={} read errno={}; defaults retained", path, errno));
            None
        }
    }
}

fn standalone_warning(path: &str, error: &ConfigError) {
    libdunit::println(&alloc::format!("dwm_settings: warning path={} line={} field={} reason={}; previous values retained",
        path, error.line, error.field, error.reason));
}

/// Per-terminal config (`apps/gui_terminal.toml`, table `[terminal]`). Mirrors
/// the palette/prompt knobs the terminal used to hardcode. `font` empty means
/// "inherit the desktop `[desktop] font`"; `bg_alpha` 255 keeps the opaque
/// XRGB fast path (values <255 request a translucent ARGB backdrop).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TerminalCfg {
    pub prompt: ConfigStr,
    pub font: ConfigStr,
    /// Active color theme name (`[terminal] theme`), resolved against the shared
    /// `term_theme` preset table. Selects the base palette (`fg/bg/ansi*`); any
    /// explicit color key below still overrides the chosen preset.
    pub theme: ConfigStr,
    pub fg: u32,
    pub bg: u32,
    pub bg_alpha: u32,
    pub ansi: [u32; 8],
    pub ansi_bright: [u32; 8],
    /// True when a config file was actually found (diagnostics only).
    pub from_file: bool,
}

impl TerminalCfg {
    pub const fn baseline() -> Self {
        // Pull the baseline palette from the shared preset table (SSOT) rather
        // than re-listing hex here — `term_theme::DEFAULT` is `green_tea`, the
        // historical hardcoded baseline, so an unset `theme` is a visual no-op.
        let d = term_theme::DEFAULT;
        TerminalCfg {
            prompt: ConfigStr::new("dsh"),
            font: ConfigStr::new(""),
            theme: ConfigStr::new(d.name),
            fg: d.fg,
            bg: d.bg,
            bg_alpha: 255,
            ansi: d.ansi,
            ansi_bright: d.ansi_bright,
            from_file: false,
        }
    }

    /// Load `apps/<app_id>.toml`, overlaying `[terminal]` onto the baseline.
    pub fn load(app_id: &str) -> Self {
        let mut c = Self::baseline();
        let path = config_path(APPS_DIR, app_id);
        if let Some(text) = read_standalone(&path) {
            if let Err(error) = c.apply(&text) { standalone_warning(&path, &error); }
        }
        c
    }

    /// First pass: find `[terminal] theme = "<name>"` and copy that preset's
    /// palette in as the base. Unknown/empty names fall back to the default.
    fn apply_theme(&mut self, text: &str) {
        let mut section = String::new();
        for raw in text.lines() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section.clear();
                section.push_str(name.trim());
                continue;
            }
            if section != "terminal" {
                continue;
            }
            let Some(eq) = line.find('=') else { continue };
            if line[..eq].trim() != "theme" {
                continue;
            }
            let t = term_theme::resolved_or_default(unquote(line[eq + 1..].trim()));
            self.theme.set(t.name);
            self.fg = t.fg;
            self.bg = t.bg;
            self.ansi = t.ansi;
            self.ansi_bright = t.ansi_bright;
        }
    }

    /// Apply a complete checked document atomically. On error, keep all prior
    /// values (including theme/palette and from_file) intact.
    pub fn apply(&mut self, text: &str) -> Result<(), ConfigError> {
        let mut candidate = *self;
        candidate.apply_theme(text);
        validation::table(text, "terminal", |key, input| {
            match key {
                "prompt" => { candidate.prompt.set(validation::config_string(input)?); }
                "font" => { candidate.font.set(validation::config_string(input)?); }
                "fg" => { candidate.fg = validation::color(input)?; }
                "bg" => { candidate.bg = validation::color(input)?; }
                "bg_alpha" => { candidate.bg_alpha = validation::uint(input, 255)?; }
                "ansi" => { candidate.ansi = validation::colors(input)?; }
                "ansi_bright" => { candidate.ansi_bright = validation::colors(input)?; }
                "theme" => { term_theme::by_name(validation::config_string(input)?).ok_or("unknown terminal theme")?; }
                _ => return Err("unknown setting"),
            }
            Ok(())
        })?;
        candidate.from_file = true;
        *self = candidate;
        Ok(())
    }
}

/// Per-file-manager config (`apps/gui_files.toml`, table `[files]`). `icon_theme`
/// empty means "inherit the desktop `[desktop] icon_theme`". Colors mirror the
/// old hardcoded palette; `bg_alpha` 255 keeps the opaque fast path.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FilesCfg {
    pub icon_theme: ConfigStr,
    pub bg: u32,
    pub header_bg: u32,
    pub status_bg: u32,
    pub accent: u32,
    pub label: u32,
    pub muted: u32,
    pub sel_bg: u32,
    pub bg_alpha: u32,
    pub from_file: bool,
}

impl FilesCfg {
    pub const fn baseline() -> Self { Self::from_theme(Theme::baseline()) }

    const fn from_theme(theme: Theme) -> Self {
        FilesCfg {
            icon_theme: ConfigStr::new(""),
            bg: theme.menu,
            header_bg: theme.panel,
            status_bg: theme.panel,
            accent: theme.launcher,
            label: theme.panel_text,
            muted: theme.title_text_unfocused,
            sel_bg: theme.menu_hover,
            bg_alpha: 255,
            from_file: false,
        }
    }

    pub fn load(app_id: &str) -> Self {
        let mut c = Self::from_theme(load().theme);
        let path = config_path(APPS_DIR, app_id);
        if let Some(text) = read_standalone(&path) {
            if let Err(error) = c.apply(&text) { standalone_warning(&path, &error); }
        }
        c
    }

    /// Apply a complete checked document atomically. On error, keep all prior
    /// values (including theme/palette and from_file) intact.
    pub fn apply(&mut self, text: &str) -> Result<(), ConfigError> {
        let mut candidate = *self;
        validation::table(text, "files", |key, input| {
            match key {
                "bg" => { candidate.bg = validation::color(input)?; }
                "header_bg" => { candidate.header_bg = validation::color(input)?; }
                "status_bg" => { candidate.status_bg = validation::color(input)?; }
                "accent" => { candidate.accent = validation::color(input)?; }
                "label" => { candidate.label = validation::color(input)?; }
                "muted" => { candidate.muted = validation::color(input)?; }
                "sel_bg" => { candidate.sel_bg = validation::color(input)?; }
                "icon_theme" => { candidate.icon_theme.set(validation::config_string(input)?); }
                "bg_alpha" => { candidate.bg_alpha = validation::uint(input, 255)?; }
                _ => return Err("unknown setting"),
            }
            Ok(())
        })?;
        candidate.from_file = true;
        *self = candidate;
        Ok(())
    }
}

/// Per-calculator config (`apps/gui_calc.toml`, table `[calc]`). Mirrors the
/// palette knobs the calculator used to hardcode as `COLOR_*` consts. `font`
/// empty means "inherit the desktop `[desktop] font`"; `bg_alpha` 255 keeps the
/// opaque XRGB fast path (values <255 request a translucent ARGB backdrop, like
/// the terminal/file-manager). The button-role colors are the single source of
/// truth for the calculator face — the app reads them, no `const COLOR_*`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CalcCfg {
    pub font: ConfigStr,
    /// Window background (behind the display + button grid).
    pub bg: u32,
    /// Result/entry display strip fill.
    pub display_bg: u32,
    /// Primary text (display value + button glyphs).
    pub text: u32,
    /// Secondary text (the pending-expression line above the result).
    pub muted: u32,
    /// Digit + dot buttons.
    pub btn: u32,
    /// Function buttons (Clear, backspace, percent, sign).
    pub btn_fn: u32,
    /// Operator buttons (÷ × − +).
    pub btn_op: u32,
    /// Equals button accent fill.
    pub btn_eq: u32,
    pub bg_alpha: u32,
    /// True when a config file was actually found (diagnostics only).
    pub from_file: bool,
}

impl CalcCfg {
    pub const fn baseline() -> Self { Self::from_theme(Theme::baseline()) }

    const fn from_theme(theme: Theme) -> Self {
        CalcCfg {
            font: ConfigStr::new(""),
            bg: theme.menu,
            display_bg: theme.panel,
            text: theme.panel_text,
            muted: theme.title_text_unfocused,
            btn: theme.taskbtn,
            btn_fn: theme.taskbtn_focused,
            btn_op: theme.menu_hover,
            btn_eq: theme.launcher,
            bg_alpha: 255,
            from_file: false,
        }
    }

    /// Load `apps/<app_id>.toml`, overlaying `[calc]` onto the baseline.
    pub fn load(app_id: &str) -> Self {
        let mut c = Self::from_theme(load().theme);
        let path = config_path(APPS_DIR, app_id);
        if let Some(text) = read_standalone(&path) {
            if let Err(error) = c.apply(&text) { standalone_warning(&path, &error); }
        }
        c
    }

    /// Apply a complete checked document atomically. On error, keep all prior
    /// values (including theme/palette and from_file) intact.
    pub fn apply(&mut self, text: &str) -> Result<(), ConfigError> {
        let mut candidate = *self;
        validation::table(text, "calc", |key, input| {
            match key {
                "bg" => { candidate.bg = validation::color(input)?; }
                "display_bg" => { candidate.display_bg = validation::color(input)?; }
                "text" => { candidate.text = validation::color(input)?; }
                "muted" => { candidate.muted = validation::color(input)?; }
                "btn" => { candidate.btn = validation::color(input)?; }
                "btn_fn" => { candidate.btn_fn = validation::color(input)?; }
                "btn_op" => { candidate.btn_op = validation::color(input)?; }
                "btn_eq" => { candidate.btn_eq = validation::color(input)?; }
                "font" => { candidate.font.set(validation::config_string(input)?); }
                "bg_alpha" => { candidate.bg_alpha = validation::uint(input, 255)?; }
                _ => return Err("unknown setting"),
            }
            Ok(())
        })?;
        candidate.from_file = true;
        *self = candidate;
        Ok(())
    }
}

/// Per-app config for the demo/tests client (`apps/gui_demo.toml`, table
/// `[demo]`). `gui_demo` is a small showcase app: a column of buttons that post
/// desktop notifications through the compositor (the NOTIFY control message), so
/// its palette is the whole config surface. `font` empty inherits the desktop
/// `[desktop] font`; `bg_alpha` 255 keeps the opaque XRGB fast path (values <255
/// request a translucent ARGB backdrop). Colors are the single source of truth
/// for the demo face — the app reads them, no `const COLOR_*`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DemoCfg {
    pub font: ConfigStr,
    /// Window background.
    pub bg: u32,
    /// Primary text (title + button labels).
    pub text: u32,
    /// Secondary text (the subtitle / hint line).
    pub muted: u32,
    /// Button base fill.
    pub btn: u32,
    /// Accent fill for the primary action button + title rule.
    pub accent: u32,
    pub bg_alpha: u32,
    /// True when a config file was actually found (diagnostics only).
    pub from_file: bool,
}

impl DemoCfg {
    pub const fn baseline() -> Self { Self::from_theme(Theme::baseline()) }

    const fn from_theme(theme: Theme) -> Self {
        DemoCfg {
            font: ConfigStr::new(""),
            bg: theme.menu,
            text: theme.panel_text,
            muted: theme.title_text_unfocused,
            btn: theme.taskbtn,
            accent: theme.launcher,
            bg_alpha: 255,
            from_file: false,
        }
    }

    /// Load `apps/<app_id>.toml`, overlaying `[demo]` onto the baseline.
    pub fn load(app_id: &str) -> Self {
        let mut c = Self::from_theme(load().theme);
        let path = config_path(APPS_DIR, app_id);
        if let Some(text) = read_standalone(&path) {
            if let Err(error) = c.apply(&text) { standalone_warning(&path, &error); }
        }
        c
    }

    /// Apply a complete checked document atomically. On error, keep all prior
    /// values (including theme/palette and from_file) intact.
    pub fn apply(&mut self, text: &str) -> Result<(), ConfigError> {
        let mut candidate = *self;
        validation::table(text, "demo", |key, input| {
            match key {
                "bg" => { candidate.bg = validation::color(input)?; }
                "text" => { candidate.text = validation::color(input)?; }
                "muted" => { candidate.muted = validation::color(input)?; }
                "btn" => { candidate.btn = validation::color(input)?; }
                "accent" => { candidate.accent = validation::color(input)?; }
                "font" => { candidate.font.set(validation::config_string(input)?); }
                "bg_alpha" => { candidate.bg_alpha = validation::uint(input, 255)?; }
                _ => return Err("unknown setting"),
            }
            Ok(())
        })?;
        candidate.from_file = true;
        *self = candidate;
        Ok(())
    }
}

/// Per-widget config (`widgets/<name>.toml`, table `[widget]`). The compositor
/// draws desktop widgets (clock, monitor, …); each owns a small file. `enabled`
/// is the removal switch — a widget whose file is missing OR whose `enabled` is
/// false is not drawn. `accent` 0 / `corner` -1 mean "inherit the desktop theme
/// accent / `[widgets] corner`".
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClockStyle { Digital, Analog }

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WidgetCfg {
    pub enabled: bool,
    pub style: ClockStyle,
    pub size: u32,
    pub font_px: u32,
    pub interval_ms: u32,
    pub accent: u32,
    pub corner: i32,
    /// True when a config file was actually found.
    pub from_file: bool,
}

impl WidgetCfg {
    pub const fn baseline() -> Self {
        WidgetCfg {
            enabled: true,
            style: ClockStyle::Digital,
            size: 160,
            font_px: 18,
            interval_ms: 1000,
            accent: 0,
            corner: -1,
            from_file: false,
        }
    }

    /// Load `widgets/<name>.toml`. A missing file yields `from_file=false` with
    /// the baseline — the caller decides whether "no file" means "not present".
    pub fn load(name: &str) -> Self {
        let mut c = Self::baseline();
        let path = config_path(WIDGETS_DIR, name);
        if let Some(text) = read_standalone(&path) {
            if let Err(error) = c.apply(&text) { standalone_warning(&path, &error); }
        }
        c
    }

    /// Apply a complete checked document atomically. On error, keep all prior
    /// values (including theme/palette and from_file) intact.
    pub fn apply(&mut self, text: &str) -> Result<(), ConfigError> {
        let mut candidate = *self;
        validation::table(text, "widget", |key, input| {
            match key {
                "enabled" => { candidate.enabled = parse_bool(input).ok_or("expected a boolean")?; }
                "accent" => { candidate.accent = validation::color(input)?; }
                "corner" => { candidate.corner = validation::uint(input, 3)? as i32; }
                "style" => { candidate.style = match validation::config_string(input)? { "analog" => ClockStyle::Analog, "digital" => ClockStyle::Digital, _ => return Err("expected analog or digital") }; }
                "size" => { candidate.size = validation::uint(input, 1024)?; if candidate.size < 24 { return Err("widget size must be at least 24"); } }
                "font_px" => { candidate.font_px = validation::uint(input, 128)?; if candidate.font_px < 6 { return Err("widget font must be at least 6 px"); } }
                "interval_ms" => { candidate.interval_ms = validation::uint(input, 3600000)?; if candidate.interval_ms < 16 { return Err("widget interval must be at least 16 ms"); } }
                _ => return Err("unknown setting"),
            }
            Ok(())
        })?;
        candidate.from_file = true;
        *self = candidate;
        Ok(())
    }
}
