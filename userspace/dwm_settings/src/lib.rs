//! DWM settings loaded from a TOML document (slice 9).
//!
//! The compositor's theme is data, not hardcoded constants: at desktop start
//! `load()` reads `/system/share/dwm/default.toml` from the (embedded) VFS and
//! overlays any recognised keys onto the built-in Green Tea baseline. A missing
//! or unparseable file is not fatal — the baseline is the last-known-good
//! default, so the desktop always comes up. Loaders reject malformed or unknown
//! settings with diagnostics and preserve last-known-good on live reload.
//!
//! The parser is a deliberately tiny TOML subset: `# ` / `//` comments, blank
//! lines, `[section]` headers (recorded but not required), and `key = "value"`
//! / `key = value` pairs. Desktop configuration retains this dialect; installed
//! application manifests use full TOML through dunit_app_manifest.

#![no_std]
extern crate alloc;

mod preserve;
mod catalog;
mod applications;
pub use applications::{AppEntry, AppKind, Applications, MAX_APPS, MAX_WS};
pub(crate) use applications::{parse_apps, parse_apps_seeded};
pub use catalog::{validate_document_with_manifests, validate_installed_document};
mod validation;
mod diagnostics;
mod render;
pub use render::Render;
pub use diagnostics::Diagnostics;
pub mod shell;
pub mod policy;
pub use validation::{validate, ConfigError};
pub mod appearance;
mod overlay;
pub use overlay::{apply_overlay, edit, load_overlay, load_overlay_patch, normalize_overlay, value};
mod scalar_save;
pub use scalar_save::{apply_scalar_overlay, scalar_changes, save_changes, save_overlay};

use alloc::string::String;
use alloc::vec::Vec;

/// Compositor palette (ARGB8888). Field names mirror the `[theme]` TOML keys.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub desktop: u32,
    pub title_focused: u32,
    pub title_unfocused: u32,
    pub border_focused: u32,
    pub border_unfocused: u32,
    pub close: u32,
    pub minimize: u32,
    pub maximize: u32,
    pub panel: u32,
    pub panel_text: u32,
    pub launcher: u32,
    pub taskbtn: u32,
    pub taskbtn_focused: u32,
    pub menu: u32,
    pub menu_hover: u32,
    /// Title-bar text on a focused window (the label the compositor draws over
    /// the title bar). Was a hardcoded 0x00EAF2EC in the compositor.
    pub title_text_focused: u32,
    /// Title-bar / taskbar text on an unfocused window. Was 0x00A8B4AC.
    pub title_text_unfocused: u32,
}

impl Theme {
    /// The built-in Green Tea baseline (identical to the pre-slice-9 constants).
    pub const fn baseline() -> Self {
        Theme {
            desktop: 0xFF1E1E2E,
            title_focused: 0xFF313244,
            title_unfocused: 0xFF232331,
            border_focused: 0xFFA6E3A1,
            border_unfocused: 0xFF45475A,
            close: 0xFFF38BA8,
            minimize: 0xFF45475A,
            maximize: 0xFF45475A,
            panel: 0xFF181825,
            panel_text: 0xFFCDD6F4,
            launcher: 0xFFA6E3A1,
            taskbtn: 0xFF313244,
            taskbtn_focused: 0xFF45475A,
            menu: 0xFF11111B,
            menu_hover: 0xFF45475A,
            title_text_focused: 0xFFEAF2EC,
            title_text_unfocused: 0xFFA8B4AC,
        }
    }

    /// Apply one `key`/`value` pair from the `[theme]` table. Unknown keys and
    /// malformed colors are ignored (the field keeps its baseline value).
    fn apply(&mut self, key: &str, value: &str) {
        let Some(color) = parse_color(value) else {
            return;
        };
        let slot = match key {
            "desktop" => &mut self.desktop,
            "title_focused" => &mut self.title_focused,
            "title_unfocused" => &mut self.title_unfocused,
            "border_focused" => &mut self.border_focused,
            "border_unfocused" => &mut self.border_unfocused,
            "close" => &mut self.close,
            "minimize" => &mut self.minimize,
            "maximize" => &mut self.maximize,
            "panel" => &mut self.panel,
            "panel_text" => &mut self.panel_text,
            "launcher" => &mut self.launcher,
            "taskbtn" => &mut self.taskbtn,
            "taskbtn_focused" => &mut self.taskbtn_focused,
            "menu" => &mut self.menu,
            "menu_hover" => &mut self.menu_hover,
            "title_text_focused" => &mut self.title_text_focused,
            "title_text_unfocused" => &mut self.title_text_unfocused,
            _ => return,
        };
        *slot = color;
    }
}

/// Compositor geometry (pixels). Field names mirror the `[layout]` TOML keys.
/// `max_windows` is a safety cap on concurrently composited windows, not a pixel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub title_h: i32,
    pub border: i32,
    pub panel_h: i32,
    pub launcher_w: i32,
    pub menu_w: i32,
    pub menu_item_h: i32,
    pub dock_w: i32,
    /// Dock edge, compact alignment and cell spacing.
    pub dock_edge: Edge,
    pub dock_centered: bool,
    pub dock_padding: i32,
    pub dock_gap: i32,
    pub dock_margin: i32,
    pub dock_tile_alpha: u32,
    /// Circular controls on the left and a centered title.
    pub traffic_lights: bool,
    pub show_workspaces: bool,
    pub ws_w: i32,
    pub max_windows: usize,
    /// Point size of the TTF window/tray title text. Was a hardcoded 14.0.
    pub title_font_px: i32,
    /// Point size of the TTF taskbar-button text. Was a hardcoded 13.0.
    pub panel_font_px: i32,
}

impl Layout {
    /// The built-in baseline (identical to the pre-slice-9.2 compositor consts).
    pub const fn baseline() -> Self {
        Layout {
            title_h: 26,
            border: 2,
            panel_h: 28,
            // App-menu button width at the left of the panel. Holds the "Dunit"
            // wordmark (logo mark + label); the workspace pips start right after
            // it. Widened from the old 40px hamburger slot to fit the wordmark.
            launcher_w: 92,
            menu_w: 130,
            menu_item_h: 26,
            dock_w: 48,
            dock_edge: Edge::Left,
            dock_centered: false,
            dock_padding: 6,
            dock_gap: 6,
            dock_margin: 0,
            dock_tile_alpha: 255,
            traffic_lights: false,
            show_workspaces: true,
            ws_w: 22,
            max_windows: 8,
            title_font_px: 14,
            panel_font_px: 13,
        }
    }

    /// Apply one `key`/`value` pair from the `[layout]` table. Unknown keys and
    /// non-numeric values are ignored (the field keeps its baseline value).
    /// `max_windows` is clamped to a sane range so a bad config cannot ask for
    /// zero (a dead desktop) or an unbounded spawn count.
    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "dock_edge" => { if let Some(e) = Edge::parse(value) { self.dock_edge = e; } return; }
            "dock_centered" => { if let Some(v) = parse_bool(value) { self.dock_centered = v; } return; }
            "show_workspaces" => { if let Some(v) = parse_bool(value) { self.show_workspaces = v; } return; }
            "traffic_lights" => { if let Some(v) = parse_bool(value) { self.traffic_lights = v; } return; }
            _ => {}
        }
        let Some(n) = parse_uint(value) else {
            return;
        };
        match key {
            "title_h" => self.title_h = n as i32,
            "border" => self.border = n as i32,
            "panel_h" => self.panel_h = n as i32,
            "launcher_w" => self.launcher_w = n as i32,
            "menu_w" => self.menu_w = n as i32,
            "menu_item_h" => self.menu_item_h = n as i32,
            "dock_w" => self.dock_w = (n.min(256) as i32).max(24),
            "dock_padding" => self.dock_padding = (n as i32).clamp(0, 10),
            "dock_gap" => self.dock_gap = (n as i32).clamp(0, 64),
            "dock_margin" => self.dock_margin = (n as i32).clamp(0, 128),
            "dock_tile_alpha" => self.dock_tile_alpha = (n as u32).min(255),
            "ws_w" => self.ws_w = n as i32,
            "max_windows" => self.max_windows = (n as usize).clamp(1, 32),
            "title_font_px" => self.title_font_px = (n as i32).clamp(6, 64),
            "panel_font_px" => self.panel_font_px = (n as i32).clamp(6, 64),
            _ => {}
        }
    }
}

/// Compositor visual effects (concept §5: rounded corners, soft shadows, backdrop
/// blur, gradients, animations). Every effect can be tuned or switched off from
/// the `[effects]` TOML table, so a low-end target (or a user who wants the flat
/// look back) can disable them without a rebuild. Field names mirror the keys.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    /// Rounded-corner radius (px) for windows/panel/menus/buttons; 0 = square.
    pub corner_radius: i32,
    /// Drop-shadow spread (px) under windows/menus; 0 = no shadow.
    pub shadow: i32,
    /// Shadow opacity at the silhouette edge (0..255).
    pub shadow_alpha: i32,
    /// Backdrop blur under the panel/menus (acrylic look).
    pub blur: bool,
    /// Blur kernel radius (px, per separable box pass).
    pub blur_radius: i32,
    /// Blur passes (2-3 ≈ Gaussian); more = smoother but costlier.
    pub blur_iters: i32,
    /// Panel tint opacity over the blurred backdrop (0..255).
    pub panel_alpha: i32,
    /// Menu tint opacity over the blurred backdrop (0..255).
    pub menu_alpha: i32,
    /// Vertical gradient fills on the panel/title bars/buttons.
    pub gradient: bool,
    /// Open/close/switch animations.
    pub anim: bool,
    /// Base animation duration (ms) at the compositor's ~60 Hz tick.
    pub anim_ms: i32,
}

impl Effects {
    /// The built-in baseline — the modern Green Tea look turned on (concept §5).
    pub const fn baseline() -> Self {
        Effects {
            corner_radius: 8,
            shadow: 7,
            shadow_alpha: 90,
            blur: true,
            blur_radius: 4,
            blur_iters: 2,
            panel_alpha: 205,
            menu_alpha: 225,
            gradient: true,
            anim: true,
            anim_ms: 140,
        }
    }

    /// Apply one `key`/`value` pair from the `[effects]` table. Bool keys accept
    /// `true`/`false`/`1`/`0`; the rest are non-negative integers. Unknown keys
    /// and malformed values are ignored (the field keeps its baseline value).
    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "blur" => {
                if let Some(b) = parse_bool(value) {
                    self.blur = b;
                }
            }
            "gradient" => {
                if let Some(b) = parse_bool(value) {
                    self.gradient = b;
                }
            }
            "anim" | "animations" => {
                if let Some(b) = parse_bool(value) {
                    self.anim = b;
                }
            }
            _ => {
                let Some(n) = parse_uint(value) else {
                    return;
                };
                let n = n as i32;
                match key {
                    "corner_radius" => self.corner_radius = n.clamp(0, 64),
                    "shadow" => self.shadow = n.clamp(0, 32),
                    "shadow_alpha" => self.shadow_alpha = n.clamp(0, 255),
                    "blur_radius" => self.blur_radius = n.clamp(0, 16),
                    "blur_iters" => self.blur_iters = n.clamp(1, 4),
                    "panel_alpha" => self.panel_alpha = n.clamp(0, 255),
                    "menu_alpha" => self.menu_alpha = n.clamp(0, 255),
                    "anim_ms" => self.anim_ms = n.clamp(0, 2000),
                    _ => {}
                }
            }
        }
    }
}

/// Bounded inline string for filesystem paths in the config, kept `Copy` (and
/// heap-free) so `Settings` stays `Copy` like the rest of the schema. Paths are
/// short; anything past `CAP` bytes is truncated.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ConfigStr {
    buf: [u8; ConfigStr::CAP],
    len: usize,
}

impl ConfigStr {
    const CAP: usize = 128;

    /// Build from a `&str` at const time (baselines), truncating to `CAP`.
    pub const fn new(s: &str) -> Self {
        let src = s.as_bytes();
        let mut buf = [0u8; ConfigStr::CAP];
        let mut i = 0;
        while i < src.len() && i < ConfigStr::CAP {
            buf[i] = src[i];
            i += 1;
        }
        ConfigStr { buf, len: i }
    }

    /// Overwrite in place from a runtime `&str` (config apply), truncating to `CAP`.
    fn set(&mut self, s: &str) {
        let src = s.as_bytes();
        let n = if src.len() > ConfigStr::CAP {
            ConfigStr::CAP
        } else {
            src.len()
        };
        // Canonical padding matters: Settings equality and serialization checks
        // must not depend on bytes left by a previously longer configuration.
        self.buf.fill(0);
        self.buf[..n].copy_from_slice(&src[..n]);
        self.len = n;
    }

    /// Borrow as `&str` (bytes originate from `&str`, so always valid UTF-8).
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

/// Asset paths (`[desktop]` table): everything the desktop loads *by path* lives
/// here, so it is fully config-driven. Baseline values are the in-tree assets; a
/// config may point any of them elsewhere in the VFS.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Desktop {
    /// Wallpaper SVG or legacy 24-bit BMP painted as the compositor backdrop.
    pub wallpaper: ConfigStr,
    /// Icon theme name under `/assets/icons/<name>/`, or absolute theme manifest.
    /// Shared loader resolves archive/index.theme/aliases or flat SVG/RGBA files.
    pub icon_theme: ConfigStr,
    /// TrueType font every GUI app loads for text; load failure is diagnosed by
    /// each consumer rather than silently substituting another font.
    pub font: ConfigStr,
    /// SDK token profile, shared by themed component windows.
    pub sdk_theme: ConfigStr,
    /// Brand logo blitted into the app-menu wordmark, a 32x32 straight-alpha
    /// `R,G,B,A` raster (circular mask baked into the alpha). Absent/mis-sized →
    /// the compositor draws its flat accent mark instead.
    pub logo: ConfigStr,
    /// TOML swatches offered by graphical Settings, independent of renderer code.
    pub palette: ConfigStr,
    pub themes: ConfigStr,
}

impl Desktop {
    pub(crate) fn valid_sdk_theme_path(path: &str) -> bool {
        path.starts_with('/') && path.len() <= ConfigStr::CAP && !path.split('/').any(|part| part == "..")
    }
    /// The built-in baseline: the assets shipped in-tree today.
    pub const fn baseline() -> Self {
        Desktop {
            wallpaper: ConfigStr::new("/assets/wallpapers/wallpaper.bmp"),
            icon_theme: ConfigStr::new("breeze"),
            font: ConfigStr::new("/assets/fonts/DejaVuSans.ttf"),
            sdk_theme: ConfigStr::new("/assets/dwm/sdk/dark.dss"),
            logo: ConfigStr::new("/assets/images/logo.rgba"),
            palette: ConfigStr::new("/assets/dwm/palettes/default.toml"),
            themes: ConfigStr::new("/assets/dwm/themes"),
        }
    }

    /// Apply one `key`/`value` pair from the `[desktop]` table (string values;
    /// already unquoted by the caller). Unknown keys are ignored.
    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "wallpaper" => self.wallpaper.set(value),
            "icon_theme" => self.icon_theme.set(value),
            "font" => self.font.set(value),
            "sdk_theme" if Self::valid_sdk_theme_path(value) => self.sdk_theme.set(value),
            "logo" => self.logo.set(value),
            "palette" => self.palette.set(value),
            "themes" => self.themes.set(value),
            _ => {}
        }
    }
}

/// Desktop widget card (concept §5): a translucent plasmoid painted on the
/// wallpaper (behind windows). `[widgets]` holds only the desktop-wide policy —
/// the master on/off and the default corner. WHICH widgets are actually drawn is
/// owned per-widget by `widgets/<name>.toml` (see [`WidgetCfg`]): each widget is
/// present only when its file exists and is enabled, so a widget is removed by
/// deleting its file (single source of truth, no `[widgets] clock/monitor` bool).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Widgets {
    /// Master toggle: `false` hides the whole plasmoid regardless of per-widget files.
    pub enabled: bool,
    /// Default screen corner inherited by a widget whose file omits `corner`:
    /// 0 = top-left, 1 = top-right, 2 = bottom-left, 3 = bottom-right.
    pub corner: i32,
}

impl Widgets {
    /// The built-in baseline: plasmoid on, default corner top-right. The clock and
    /// monitor widgets ship as their own `widgets/*.toml` files, not bools here.
    pub const fn baseline() -> Self {
        Widgets {
            enabled: true,
            corner: 1,
        }
    }

    /// Apply one `key`/`value` pair from the `[widgets]` table. `enabled` accepts
    /// `true`/`false`/`1`/`0`; `corner` is a non-negative integer clamped to 0..3.
    /// Unknown keys and malformed values are ignored (field keeps its baseline).
    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "enabled" => {
                if let Some(b) = parse_bool(value) {
                    self.enabled = b;
                }
            }
            "corner" => {
                if let Some(n) = parse_uint(value) {
                    self.corner = (n as i32).clamp(0, 3);
                }
            }
            _ => {}
        }
    }
}

/// Runtime display mode. `0/0` means "keep the boot (Limine) resolution": the
/// compositor only asks the kernel to switch scanout when both are non-zero, and
/// on a fixed GOP framebuffer the request is ignored (EOPNOTSUPP). Backend-
/// agnostic — the resolution is user-facing policy, the kernel only supplies the
/// mode-set mechanism.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Display {
    pub width: u32,
    pub height: u32,
}

impl Display {
    /// Baseline `0/0` = keep boot resolution (behaves exactly as today).
    pub const fn baseline() -> Self {
        Display {
            width: 0,
            height: 0,
        }
    }

    /// Apply one `key`/`value` pair from the `[display]` table. `width`/`height`
    /// are non-negative integers; unknown keys and malformed values are ignored.
    fn apply(&mut self, key: &str, value: &str) {
        if let Some(n) = parse_uint(value) {
            match key {
                "width" => self.width = n,
                "height" => self.height = n,
                _ => {}
            }
        }
    }
}

/// Screen edge a shell strip (panel / tray) docks to. Fieldless, so it stays
/// `Copy` and `Settings` with it. Serialized as its lowercase name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    /// Top/Bottom strips run horizontally (full width); Left/Right run vertically.
    pub const fn is_horizontal(self) -> bool {
        matches!(self, Edge::Top | Edge::Bottom)
    }

    /// TOML spelling (matches `parse`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Bottom => "bottom",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }

    /// Parse a `pos = "..."` value; unknown spellings return `None` (field keeps
    /// its baseline edge).
    fn parse(value: &str) -> Option<Edge> {
        match value {
            "top" => Some(Edge::Top),
            "bottom" => Some(Edge::Bottom),
            "left" => Some(Edge::Left),
            "right" => Some(Edge::Right),
            _ => None,
        }
    }
}

/// Panel placement (`[panel]` table). The panel is the desktop's shell bar
/// (launcher button, workspace switcher, focused-window title, and — when the
/// tray shares its edge — the tray readout). Its *thickness* stays `[layout]
/// panel_h` (single source of truth); this table only picks which screen edge it
/// docks to, so the panel can move to any side without a rebuild.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Panel {
    pub pos: Edge,
}

impl Panel {
    pub const fn baseline() -> Self {
        Panel { pos: Edge::Top }
    }

    fn apply(&mut self, key: &str, value: &str) {
        if key == "pos" {
            if let Some(e) = Edge::parse(value) {
                self.pos = e;
            }
        }
    }
}

/// System-tray placement + thickness (`[tray]` table). The tray shows live RAM /
/// process / uptime. When it shares the panel's edge it renders INSIDE the panel
/// strip (end-aligned — the classic top-panel look); on any other edge it takes
/// its own reserved strip `size` px thick. Baseline top/28 reproduces the
/// pre-Phase-5 in-panel tray exactly.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Tray {
    pub pos: Edge,
    pub size: i32,
}

impl Tray {
    pub const fn baseline() -> Self {
        Tray {
            pos: Edge::Top,
            size: 28,
        }
    }

    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "pos" => {
                if let Some(e) = Edge::parse(value) {
                    self.pos = e;
                }
            }
            "size" => {
                if let Some(n) = parse_uint(value) {
                    self.size = (n as i32).clamp(12, 200);
                }
            }
            _ => {}
        }
    }
}

/// Highest config schema this build understands. A `[meta] schema_version`
/// greater than this means the file was written by a NEWER desktop and may use
/// keys/semantics we cannot honour, so `load_config` rejects it (→ last-known-
/// good) rather than half-applying an unknown layout. An older version is
/// resolved by the current shell model. Absent `[meta]` is treated as the
/// current version (old hand-written files stay valid).
pub const SCHEMA_VERSION: u32 = 2;

/// True iff this build can load a document declaring schema version `v`. The
/// single predicate `load_config` and the compositor's startup guard both use,
/// so "which versions are accepted" has one definition. Version 0 is not a real
/// schema (a malformed/empty `schema_version`), so it is rejected.
pub const fn version_supported(v: u32) -> bool {
    v >= 1 && v <= SCHEMA_VERSION
}

/// Config document metadata (`[meta]` table). Currently just the schema version,
/// which drives compatibility handling in `load_config`. Kept as its own typed
/// section so the version travels through the same parse/serialize/round-trip
/// path as every other table (no special-case string handling).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub schema_version: u32,
}

impl Meta {
    pub const fn baseline() -> Self {
        Meta {
            schema_version: SCHEMA_VERSION,
        }
    }

    fn apply(&mut self, key: &str, value: &str) {
        if key == "schema_version" {
            if let Some(n) = parse_uint(value) {
                self.schema_version = n;
            }
        }
    }
}

/// Which keyboard modifier arms the Alt-Tab-style window switcher (`[shortcuts]
/// switch_mod`). The switch KEY is always Tab (a UI invariant); only the held
/// modifier is user policy, so a user who prefers a Super-Tab switcher (GNOME
/// style) flips this without a rebuild. `mask()` returns the `libdunit::KEYMOD_*`
/// bit so the compositor tests the held modifier against config, not a constant.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SwitchMod {
    Alt,
    Super,
}

impl SwitchMod {
    pub const fn as_str(self) -> &'static str {
        match self {
            SwitchMod::Alt => "alt",
            SwitchMod::Super => "super",
        }
    }

    fn parse(value: &str) -> Option<SwitchMod> {
        match value {
            "alt" => Some(SwitchMod::Alt),
            "super" => Some(SwitchMod::Super),
            _ => None,
        }
    }

    /// The `KEYMOD_*` bit (mirrors libdunit: ALT = 1<<2, SUPER = 1<<3) this
    /// modifier maps to, so the switcher tests `ev.mods & switch_mod.mask()`.
    pub const fn mask(self) -> u8 {
        match self {
            SwitchMod::Alt => 1 << 2,
            SwitchMod::Super => 1 << 3,
        }
    }
}

/// Keyboard shortcut policy (`[shortcuts]` table). Which global gestures the
/// compositor honours and how they are triggered — user-facing policy, so it
/// lives in config rather than baked into the key-drain loop.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Shortcuts {
    /// Enable the Alt/Super-Tab window switcher overlay.
    pub switcher: bool,
    /// Modifier that arms the switcher (Tab is the fixed trigger key).
    pub switch_mod: SwitchMod,
    /// Modifier that arms the global desktop command shortcuts below (the
    /// tiling-WM `$mod` key). Only the modifier is policy — the action KEYS
    /// (digits for workspaces, Space/Q/M) are UI invariants, exactly like the
    /// switcher's Tab. Alt or Super.
    pub cmd_mod: SwitchMod,
    /// `cmd_mod`+digit(1..=workspaces) jumps to that virtual workspace.
    pub ws_switch: bool,
    /// `cmd_mod`+Space opens the application launcher menu (keyboard-navigable).
    pub launcher: bool,
    /// `cmd_mod`+Q closes the focused window (same path as the close chip).
    pub win_close: bool,
    /// `cmd_mod`+M toggles maximize on the focused window (same path as the
    /// maximize chip — a server-push CONFIGURE).
    pub win_max: bool,
}

impl Shortcuts {
    pub const fn baseline() -> Self {
        Shortcuts {
            switcher: true,
            switch_mod: SwitchMod::Alt,
            cmd_mod: SwitchMod::Super,
            ws_switch: true,
            launcher: true,
            win_close: true,
            win_max: true,
        }
    }

    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "switcher" => {
                if let Some(b) = parse_bool(value) {
                    self.switcher = b;
                }
            }
            "switch_mod" => {
                if let Some(m) = SwitchMod::parse(value) {
                    self.switch_mod = m;
                }
            }
            "cmd_mod" => {
                if let Some(m) = SwitchMod::parse(value) {
                    self.cmd_mod = m;
                }
            }
            "ws_switch" => {
                if let Some(b) = parse_bool(value) {
                    self.ws_switch = b;
                }
            }
            "launcher" => {
                if let Some(b) = parse_bool(value) {
                    self.launcher = b;
                }
            }
            "win_close" => {
                if let Some(b) = parse_bool(value) {
                    self.win_close = b;
                }
            }
            "win_max" => {
                if let Some(b) = parse_bool(value) {
                    self.win_max = b;
                }
            }
            _ => {}
        }
    }
}

/// Quick-settings applet policy (`[quicksettings]` table). The panel applet opens
/// a flyout that toggles live effect/widget flags. The flags themselves stay in
/// `[effects]`/`[widgets]` (single source of truth); this table only gates whether
/// the applet is offered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct QuickSettings {
    pub enabled: bool,
}

impl QuickSettings {
    pub const fn baseline() -> Self {
        QuickSettings { enabled: true }
    }

    fn apply(&mut self, key: &str, value: &str) {
        if key == "enabled" {
            if let Some(b) = parse_bool(value) {
                self.enabled = b;
            }
        }
    }
}

/// Transfer broker policy shared by clipboard and drag-and-drop.
#[derive(Clone,Copy,PartialEq,Eq)]
pub struct Transfers { pub timeout_ms:u32,pub drag_threshold_px:u32,pub trace:bool }
impl Transfers {
    pub const fn baseline()->Self{Self{timeout_ms:5000,drag_threshold_px:6,trace:false}}
    fn apply(&mut self,key:&str,value:&str){
        if key=="trace"{if let Some(value)=parse_bool(value){self.trace=value;}return;}
        if let Some(n)=parse_uint(value){match key{"timeout_ms"=>self.timeout_ms=n.clamp(500,60_000),"drag_threshold_px"=>self.drag_threshold_px=n.clamp(1,64),_=>{}}}
    }
}

/// Desktop notification policy (`[notifications]` table). Transient toasts posted
/// by the compositor (launch feedback) or by clients over the notify IPC channel.
/// `corner` uses the same 0=TL/1=TR/2=BL/3=BR convention as `[widgets] corner`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Notifications {
    pub enabled: bool,
    /// Auto-dismiss timeout in milliseconds (clamped to a sane range on parse).
    pub timeout_ms: u32,
    /// Screen corner toasts stack in (0=top-left, 1=top-right, 2=bottom-left,
    /// 3=bottom-right).
    pub corner: i32,
}

impl Notifications {
    pub const fn baseline() -> Self {
        Notifications {
            enabled: true,
            timeout_ms: 3000,
            corner: 1,
        }
    }

    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "enabled" => {
                if let Some(b) = parse_bool(value) {
                    self.enabled = b;
                }
            }
            "timeout_ms" => {
                if let Some(n) = parse_uint(value) {
                    self.timeout_ms = n.clamp(500, 30_000);
                }
            }
            "corner" => {
                if let Some(n) = parse_uint(value) {
                    self.corner = (n as i32).clamp(0, 3);
                }
            }
            _ => {}
        }
    }
}

/// Window/session restore policy (`[session]` table). When `restore` is on, the
/// compositor persists each window's geometry (keyed by app id) to the session
/// store `SESSION_PATH` and re-applies it when the app opens again. The geometry
/// data lives in its own file (`[window.<id>]` tables); this table is just the
/// master toggle. RAM-only until DunitFS v2, then it survives reboots unchanged.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Session {
    pub restore: bool,
}

impl Session {
    pub const fn baseline() -> Self {
        Session { restore: true }
    }

    fn apply(&mut self, key: &str, value: &str) {
        if key == "restore" {
            if let Some(b) = parse_bool(value) {
                self.restore = b;
            }
        }
    }
}

/// Resolved DWM settings. Extends with `[layout]` etc. in later sub-slices.
/// Optional Lua window-policy runner. The VM runs in a separate process;
/// the compositor enforces a wall-clock deadline and validates its entire result.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Scripts {
    pub enabled: bool,
    pub runner: ConfigStr,
    pub layout: ConfigStr,
    pub events: ConfigStr,
    pub timeout_ms: u32,
}

impl Scripts {
    pub const fn baseline() -> Self {
        Self {
            enabled: false,
            runner: ConfigStr::new("/app/dwm_lua"),
            layout: ConfigStr::new("/assets/dwm/scripts/columns.lua"),
            events: ConfigStr::new(""),
            timeout_ms: 500,
        }
    }
    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "enabled" => {
                if let Some(v) = parse_bool(value) {
                    self.enabled = v;
                }
            }
            "runner" => self.runner.set(value),
            "layout" => self.layout.set(value),
            "events" => self.events.set(value),
            "timeout_ms" => {
                if let Ok(v) = value.parse::<u32>() {
                    self.timeout_ms = v.clamp(50, 5000);
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub scripts: Scripts,
    pub transfers: Transfers,
    pub meta: Meta,
    pub theme: Theme,
    pub layout: Layout,
    pub effects: Effects,
    pub desktop: Desktop,
    pub widgets: Widgets,
    pub display: Display,
    pub panel: Panel,
    pub tray: Tray,
    pub shortcuts: Shortcuts,
    pub quicksettings: QuickSettings,
    pub notifications: Notifications,
    pub session: Session,
    /// True when a config file was found and read (parse still best-effort).
    pub from_file: bool,
    /// Count of recognised keys applied over the baseline (0 for pure default).
    pub applied: u32,
}

impl Settings {
    pub const fn defaults() -> Self {
        Settings {
            scripts: Scripts::baseline(),
            transfers: Transfers::baseline(),
            meta: Meta::baseline(),
            theme: Theme::baseline(),
            layout: Layout::baseline(),
            effects: Effects::baseline(),
            desktop: Desktop::baseline(),
            widgets: Widgets::baseline(),
            display: Display::baseline(),
            panel: Panel::baseline(),
            tray: Tray::baseline(),
            shortcuts: Shortcuts::baseline(),
            quicksettings: QuickSettings::baseline(),
            notifications: Notifications::baseline(),
            session: Session::baseline(),
            from_file: false,
            applied: 0,
        }
    }
}

const CONFIG_PATH:&str="/system/share/dwm/default.toml";
/// An optional complete user profile overrides the system desktop document.
/// Theme overlays are merged through the separate overlay API.
pub const USER_CONFIG_PATH:&str="/persist/config/dwm.toml";
fn read_active_config()->Result<String,isize>{
    match read_file_checked(USER_CONFIG_PATH){Ok(text)=>Ok(text),Err(libdunit::ENOENT)=>read_file_checked(CONFIG_PATH),Err(error)=>Err(error)}
}
pub fn active_config_path()->&'static str{match read_file_checked(USER_CONFIG_PATH){Err(libdunit::ENOENT)=>CONFIG_PATH,_=>USER_CONFIG_PATH}}
fn write_active_config(text:&str)->Result<(),isize>{
    let path=match read_file_checked(USER_CONFIG_PATH){Ok(_)=>USER_CONFIG_PATH,Err(libdunit::ENOENT)=>CONFIG_PATH,Err(error)=>return Err(error)};
    libdunit::write_string(path,text)
}
/// The packaged desktop remains recoverable even if a user profile is broken.
pub fn load_system_config()->Config{
    for path in [CONFIG_PATH,"/assets/dwm/default.toml"]{if let Ok(source)=read_file_checked(path){if let Ok(config)=validate_installed_document(&source){if config.valid{return config;}}}}
    Config::defaults()
}
/// Activate a complete user profile while retaining the system document.
pub fn save_user_document(text:&str)->Result<(),ConfigError>{
    let config=validate_installed_document(text)?;
    if !config.valid{return Err(ConfigError{line:0,field:"application".into(),reason:"user profile requires a complete application registry"});}
    let _=libdunit::mkdir("/persist/config");
    libdunit::write_string(USER_CONFIG_PATH,text).map_err(|_|ConfigError{line:0,field:USER_CONFIG_PATH.into(),reason:"could not write user profile"})
}

/// The whole resolved desktop configuration: scalar `settings` (theme/layout/
/// effects/desktop) plus the `apps` model, and a `valid` flag for last-known-good
/// handling. A live reload keeps its previous `Config` when a candidate is
/// `!valid` (garbage/empty file), so a bad edit never wipes a working desktop.
#[derive(Clone)]
pub struct Config {
    /// Explicit legacy appearance converted to shared DSS tokens.
    pub appearance: String,
    pub render: Render,
    pub diagnostics: Diagnostics,
    pub settings: Settings,
    pub apps: Applications,
    pub shell: shell::ShellConfig,
    pub policy: policy::PolicyConfig,
    pub valid: bool,
}

impl Config {
    /// The pure built-in default (no config file): baselines, marked valid.
    pub fn defaults() -> Config {
        Config {
            appearance: String::new(),
            diagnostics: Diagnostics::default(),
            render: Render::default(),
            settings: Settings::defaults(),
            apps: Applications::baseline(),
            shell: shell::ShellConfig::default(),
            policy: policy::PolicyConfig::default(),
            valid: false,
        }
    }
}

/// Save differences against the current stored scalar values. For a staged
/// draft use `save_changes`; explicit assignments/imports use `save_overlay`.
/// Complete replacement remains `save_document`. Unchanged inherited appearance
/// values are never materialized by this compatibility helper.
pub fn save(settings: &Settings) -> bool {
    let Ok(source) = load_document() else { return false; };
    let Ok(current) = validate_installed_document(&source) else { return false; };
    let Ok(patch) = scalar_changes(&current.settings, settings) else { return false; };
    scalar_save::save_to(&source, &patch)
}

/// Read + parse the system DWM config, overlaying it on the baseline. Never
/// fails: an absent/unreadable/garbage file yields the pure baseline.
pub fn load() -> Settings {
    // Scalar consumers must validate against the same installed app catalog as
    // the compositor; otherwise a manifest-only dock rejects the whole theme.
    load_config().settings
}

fn config_warning(error: &ConfigError) {
    libdunit::println(&alloc::format!("dwm_settings: warning file={} line={} field={} reason={}; configuration rejected",
        active_config_path(),error.line, error.field, error.reason));
}

/// Low-level scalar parser used by checked edits. Loaders validate the complete
/// document first; callers accepting untrusted documents must use validate().
pub fn parse_into(text: &str, settings: &mut Settings) {
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
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = line[..eq].trim();
        let value = unquote(line[eq + 1..].trim());
        if section == "theme" {
            settings.theme.apply(key, value);
            settings.applied += 1;
        } else if section == "layout" {
            settings.layout.apply(key, value);
            settings.applied += 1;
        } else if section == "effects" {
            settings.effects.apply(key, value);
            settings.applied += 1;
        } else if section == "desktop" {
            settings.desktop.apply(key, value);
            settings.applied += 1;
        } else if section == "widgets" {
            settings.widgets.apply(key, value);
            settings.applied += 1;
        } else if section == "display" {
            settings.display.apply(key, value);
            settings.applied += 1;
        } else if section == "panel" {
            settings.panel.apply(key, value);
            settings.applied += 1;
        } else if section == "tray" {
            settings.tray.apply(key, value);
            settings.applied += 1;
        } else if section == "meta" {
            settings.meta.apply(key, value);
            settings.applied += 1;
        } else if section == "shortcuts" {
            settings.shortcuts.apply(key, value);
            settings.applied += 1;
        } else if section == "quicksettings" {
            settings.quicksettings.apply(key, value);
            settings.applied += 1;
        } else if section == "notifications" {
            settings.notifications.apply(key, value);
            settings.applied += 1;
        } else if section == "transfers" {
            settings.transfers.apply(key,value);
            settings.applied+=1;
        } else if section == "scripts" {
            settings.scripts.apply(key, value);
            settings.applied += 1;
        } else if section == "session" {
            settings.session.apply(key, value);
            settings.applied += 1;
        }
    }
}

/// Read + parse the whole desktop config — scalar `settings` *and* the
/// application model — with last-known-good semantics. Never fails: an absent
/// file yields the pure baseline (marked `valid`); a present-but-garbage file
/// yields `valid == false`, so a live reload can keep the previous good `Config`
/// instead of applying junk. This is the authoritative desktop-config entry
/// point; `load()` remains for callers that only need the scalar `Settings`.
pub fn load_config() -> Config {
    let mut settings = Settings::defaults();
    let text = match read_active_config() {
        Ok(text) => text,
        Err(libdunit::ENOENT) => match read_file_checked("/assets/dwm/default.toml") {
            Ok(text) => text,
            Err(errno) => {
                libdunit::println(&alloc::format!("dwm_settings: warning canonical desktop configuration unavailable errno={}", errno));
                return Config::defaults();
            }
        },
        Err(errno) => {
            libdunit::println(&alloc::format!("dwm_settings: warning could not read complete configuration errno={}", errno));
            let mut config = Config::defaults();
            config.valid = false;
            return config;
        }
    };
    let mut base = match catalog::load() {
        Ok(catalog) => catalog,
        Err(error) => { config_warning(&error); return Config::defaults(); }
    };
    let explicit_registry = text.lines().any(|line| strip_comment(line).trim().starts_with("[application."));
    if !explicit_registry {
        let default = match read_file_checked("/assets/dwm/default.toml") {
            Ok(default) => default,
            Err(_) => { libdunit::println("dwm_settings: canonical application registry unavailable"); return Config::defaults(); }
        };
        if let Err(error) = validation::validate_with_apps(&default, &base) {
            config_warning(&error);
            return Config::defaults();
        }
        base = parse_apps_seeded(&default, &base).0;
    }
    if let Err(error) = validation::validate_with_apps(&text, &base) {
        config_warning(&error);
        return Config::defaults();
    }
    settings.from_file = true;
    parse_into(&text, &mut settings);
    let (apps, saw_apps) = parse_apps_seeded(&text, &base);

    match resolve_config(&text, settings, apps, saw_apps || !base.apps.is_empty()) {
        Ok(config) => config,
        Err(error) => { config_warning(&error); Config::defaults() }
    }
}

/// Validate and resolve a complete document without filesystem access. A document
/// without an application registry has an empty model; load_config additionally
/// inherits the packaged registry for legacy theme-only configurations.
pub fn validate_document(text: &str) -> Result<Config, ConfigError> {
    validate(text)?;
    let mut settings = Settings::defaults();
    settings.from_file = true;
    parse_into(text, &mut settings);
    let (apps, saw_apps) = parse_apps(text);
    resolve_config(text, settings, apps, saw_apps)
}

fn validate_seeded_document(text: &str, base: &Applications) -> Result<Config, ConfigError> {
    validation::validate_with_apps(text, base)?;
    let mut settings = Settings::defaults();
    settings.from_file = true;
    parse_into(text, &mut settings);
    let (apps, saw_apps) = parse_apps_seeded(text, base);
    resolve_config(text, settings, apps, saw_apps || !base.apps.is_empty())
}

/// Load editable source exactly as stored. Missing system source falls back to
/// the packaged document, matching desktop startup's first-run behavior.
pub fn load_document() -> Result<String, ConfigError> {
    match read_active_config() {
        Ok(text) => Ok(text),
        Err(libdunit::ENOENT) => read_file_checked("/assets/dwm/default.toml")
            .map_err(|_| ConfigError { line: 0, field: String::new(), reason: "could not read complete desktop configuration" }),
        Err(_) => Err(ConfigError { line: 0, field: String::new(), reason: "could not read complete desktop configuration" }),
    }
}

/// Read a bounded complete-desktop import source from an explicit absolute path.
pub fn load_document_from(path: &str) -> Result<String, ConfigError> {
    if !path.starts_with('/') || path.split('/').any(|part| part == "..") {
        return Err(ConfigError { line: 0, field: String::new(), reason: "document path must be absolute without traversal" });
    }
    let source = read_file_checked(path).map_err(|_| ConfigError {
        line: 0, field: String::new(), reason: "could not read complete desktop document",
    })?;
    if !validate_installed_document(&source)?.valid {
        return Err(ConfigError { line: 0, field: String::new(), reason: "full desktop import requires an application registry" });
    }
    Ok(source)
}

/// Save checked source to the existing desktop configuration node. Keeping the
/// caller's complete source preserves comments and tables edited by Settings.
/// No runtime state is mutated here; the existing reload validates its candidate.
pub fn save_document(text: &str) -> Result<(), ConfigError> {
    if !validate_installed_document(text)?.valid {
        return Err(ConfigError { line: 0, field: String::from("application"), reason: "full desktop document requires an application registry" });
    }
    write_active_config(text).map_err(|_| ConfigError {
        line: 0, field: String::new(), reason: "could not write desktop configuration",
    })
}

fn resolve_config(text: &str, mut settings: Settings, apps: Applications, saw_apps: bool) -> Result<Config, ConfigError> {
    let declared = settings.meta.schema_version;
    let version_ok = version_supported(declared);
    if version_ok && declared < SCHEMA_VERSION {
        settings.meta.schema_version = SCHEMA_VERSION;
    }
    let shell = shell::resolve(text, &settings, &apps).map_err(|error|validation::locate(text,error))?;
    let policy = policy::resolve(text, &apps).map_err(|error|validation::locate(text,error))?;
    let valid = version_ok
        && apps.workspaces >= 1
        && apps.workspaces <= MAX_WS
        && !apps.apps.is_empty()
        && apps.apps.len() <= MAX_APPS
        && (settings.applied > 0 || saw_apps || shell.explicit);
    let diagnostics = Diagnostics::resolve(text)?;
    let render=Render::resolve(text)?;
    Ok(Config { appearance: appearance::overrides(text, &settings), settings, apps, shell, policy, diagnostics, render, valid })
}

/// Read configurable GUI swatches using the same TOML/color dialect as Settings.
/// An invalid list is rejected as a whole, never silently shortened.
pub fn load_palette(path: &str) -> Option<Vec<u32>> {
    let text = read_file(path)?;
    let mut section = "";
    let mut result = None;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim();
        } else if let Some((key, value)) = line.split_once('=') {
            if section != "palette" || key.trim() != "colors" { continue; }
            if result.is_some() { return None; }
            let value = value.trim();
            if !value.starts_with('[') || !value.ends_with(']') { return None; }
            let entries = parse_str_array(value);
            if entries.is_empty() || entries.len() > 256 { return None; }
            let mut colors = Vec::new();
            for entry in entries {
                let color = parse_color(&entry)?;
                if !colors.contains(&color) { colors.push(color); }
            }
            result = Some(colors);
        }
    }
    result
}

/// Parse a single-line TOML string array (`["a", "b"]`) into its elements.
/// Tolerant: absent brackets are ignored, blank/empty elements are dropped.
fn parse_str_array(value: &str) -> Vec<String> {
    let t = value.trim();
    let t = t.strip_prefix('[').unwrap_or(t);
    let t = t.strip_suffix(']').unwrap_or(t);
    let mut out = Vec::new();
    for part in t.split(',') {
        let s = unquote(part.trim());
        if !s.is_empty() {
            out.push(String::from(s));
        }
    }
    out
}

/// Drop a trailing `#`/`//` comment, but only outside a quoted string so a `#`

/// inside a color literal survives.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_str = !in_str,
            b'#' if !in_str => return &line[..i],
            b'/' if !in_str && i + 1 < bytes.len() && bytes[i + 1] == b'/' => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(value)
}

/// Parse `#RRGGBB` / `#AARRGGBB` (and bare `RRGGBB`/`AARRGGBB`) into ARGB8888.
/// A 6-digit value is forced fully opaque.
fn parse_color(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let hex = hex
        .strip_prefix("0x")
        .or_else(|| hex.strip_prefix("0X"))
        .unwrap_or(hex);
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let mut acc: u32 = 0;
    for c in hex.chars() {
        acc = (acc << 4) | c.to_digit(16)?;
    }
    if hex.len() == 6 {
        acc |= 0xFF00_0000;
    }
    Some(acc)
}

/// Parse a non-negative decimal integer (e.g. a `[layout]` pixel count). Returns
/// `None` for an empty string or any non-digit character, so a malformed value
/// leaves the field at its baseline. Saturates rather than overflowing.
fn parse_uint(value: &str) -> Option<u32> {
    if value.is_empty() {
        return None;
    }
    let mut acc: u32 = 0;
    for c in value.chars() {
        let d = c.to_digit(10)?;
        acc = acc.saturating_mul(10).saturating_add(d);
    }
    Some(acc)
}

/// Parse a boolean toggle for the `[effects]` table: `true`/`false` (any case)
/// or `1`/`0`. Anything else returns `None` (field keeps its baseline value).
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" | "True" | "TRUE" | "1" | "on" | "yes" => Some(true),
        "false" | "False" | "FALSE" | "0" | "off" | "no" => Some(false),
        _ => None,
    }
}

/// Slurp a whole VFS file into a String (best-effort). `None` on open error or
/// non-UTF-8 content, read error, or oversized input. Never accepts a partial read.
fn read_file(path: &str) -> Option<String> {
    read_file_checked(path).ok()
}

fn read_file_checked(path: &str) -> Result<String, isize> {
    let fd = libdunit::open(path, libdunit::OPEN_READ);
    if fd < 0 {
        return Err(fd);
    }
    let fd = fd as usize;
    let mut data: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let n = libdunit::read(fd, &mut chunk);
        if n < 0 {
            libdunit::close(fd);
            return Err(n);
        }
        if n == 0 {
            break;
        }
        if data.len() + n as usize > 64 * 1024 {
            libdunit::close(fd);
            return Err(libdunit::EFBIG);
        }
        data.extend_from_slice(&chunk[..n as usize]);
    }
    let closed = libdunit::close(fd);
    if closed < 0 {
        return Err(closed);
    }
    String::from_utf8(data).map_err(|_| libdunit::EINVAL)
}

mod scalar_document;
pub use scalar_document::{to_toml, roundtrip_ok};
mod app_config;
pub use app_config::{APPS_DIR, WIDGETS_DIR, TerminalCfg, FilesCfg, CalcCfg, DemoCfg, WidgetCfg, ClockStyle};
mod session_store;
pub use session_store::{WinGeom, load_session, save_session, SESSION_PATH, MAX_SESSION};
