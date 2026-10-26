//! Application registry and user composition, independent of filesystem discovery.
use crate::{parse_bool, parse_str_array, parse_uint, strip_comment, unquote};
use alloc::{string::String, vec::Vec};

/// Upper bound on registered applications (safety cap, not a pixel count): a
/// malformed config cannot make the compositor allocate an unbounded registry.
pub const MAX_APPS: usize = 32;
/// Upper bound on virtual workspaces the switcher will lay out.
pub const MAX_WS: usize = 9;

/// One registered application — an `[application.<id>]` TOML table. This is the
/// data that replaces the compositor's old hardcoded `LAUNCH_APPS`/`app_title`:
/// adding an app is adding a table here, never editing gui_server. `exec`/`icon`/
/// `label` default to derivations of `id` so a minimal entry is just an `id`.
#[derive(Clone, PartialEq, Eq)]
pub struct AppEntry {
    /// Registry key from `[application.<id>]`; also the default exec/icon stem.
    pub id: String,
    /// Human-readable window/taskbar title.
    pub name: String,
    /// ELF to spawn (resolved by `libdunit::spawn` against /app). Defaults to `id`.
    pub exec: String,
    /// Icon: a stem resolved as SVG (then legacy RGBA), or an absolute VFS
    /// path if it contains a '/'. Defaults to `id`.
    pub icon: String,
    /// 1-4 char dock/menu initials fallback when no icon renders. Derived from
    /// `name` (uppercased) when omitted.
    pub label: String,
}

/// The data-driven desktop application model: the registry plus the ordered
/// dock / launcher / autostart lists (indices into `apps`) and the workspace
/// count. Built from `[application.*]`, `[dock]`, `[launcher]`, `[startup]` and
/// `[workspaces]`. Replaces the compositor's hardcoded `LAUNCH_APPS`/`WS_COUNT`.
#[derive(Clone)]
pub struct Applications {
    pub apps: Vec<AppEntry>,
    /// Dock icons, in order, as indices into `apps`.
    pub dock: Vec<usize>,
    /// Launcher-menu entries, in order, as indices into `apps`.
    pub launcher: Vec<usize>,
    /// Autostart spawn list, in order, as indices into `apps` (may repeat).
    pub startup: Vec<usize>,
    /// Number of virtual workspaces.
    pub workspaces: usize,
    /// Test-only: `[startup] self_test = true` makes the compositor drive its
    /// runtime windows through a headless resize self-exercise (Phase 3) — no
    /// mouse exists in the automated harness, so a title-bar maximize chip can't
    /// be clicked. NEVER set in `default.toml`: a real desktop must not auto-
    /// maximize its windows; only `test.toml` opts in.
    pub self_test: bool,
}

impl AppEntry {
    /// Legacy table defaults; installed metadata supplies its own executable and icon.
    fn new(id: &str, name: &str, label: &str) -> AppEntry {
        AppEntry {
            id: String::from(id),
            name: String::from(name),
            exec: String::from(id),
            icon: String::from(id),
            label: String::from(label),
        }
    }
}

impl Applications {
    /// An empty, inert model for failed configuration reads. Application
    /// identities and default lists are loaded from the packaged TOML document.
    pub fn baseline() -> Applications {
        Applications {
            apps: Vec::new(),
            dock: Vec::new(),
            launcher: Vec::new(),
            startup: Vec::new(),
            workspaces: 1,
            self_test: false,
        }
    }
}

/// Build the `Applications` model from a config document. Returns the model and
/// whether any `[application.<id>]` table was present. With none, the built-in
/// baseline registry is returned (an old theme-only config still yields the
/// standard desktop) and the caller judges validity from the scalar `applied`.
///
/// Two passes over the tiny-TOML lines: the first registers every
/// `[application.<id>]` (first-seen order) and applies its `name`/`exec`/`icon`/
/// `label` keys plus the raw `[dock]`/`[launcher]`/`[startup]` `entries` arrays
/// and `[workspaces].count`; the second resolves those id-lists into indices
/// (unknown ids dropped). Absent `[dock]`/`[launcher]` default to the whole
/// registry in order; absent `[startup]` autostarts nothing.
pub(crate) fn parse_apps(text: &str) -> (Applications, bool) {
    parse_apps_seeded(text, &Applications::baseline())
}
pub(crate) fn parse_apps_seeded(text: &str, base: &Applications) -> (Applications, bool) {
    let mut apps = base.apps.clone();
    let mut saw_apps = false;
    let mut dock_ids: Vec<String> = Vec::new();
    let mut launcher_ids: Vec<String> = Vec::new();
    let mut startup_ids: Vec<String> = Vec::new();
    let (mut have_dock, mut have_launcher, mut have_startup) = (false, false, false);
    let mut workspaces = base.workspaces;
    let mut self_test = base.self_test;

    let mut section = String::new(); // full header, e.g. "application.gui_files"
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section.clear();
            section.push_str(name.trim());
            // Register the id eagerly so even an empty `[application.x]` counts.
            if let Some(id) = section.strip_prefix("application.") {
                saw_apps = true;
                let id = id.trim();
                if !id.is_empty() && apps.len() < MAX_APPS && !apps.iter().any(|a| a.id == id) {
                    apps.push(AppEntry::new(id, id, ""));
                }
            }
            continue;
        }
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = line[..eq].trim();
        let rhs = line[eq + 1..].trim();

        if let Some(id) = section.strip_prefix("application.") {
            let id = id.trim();
            let value = unquote(rhs);
            if let Some(app) = apps.iter_mut().find(|a| a.id == id) {
                match key {
                    "name" => app.name = String::from(value),
                    "exec" => app.exec = String::from(value),
                    "icon" => app.icon = String::from(value),
                    "label" => app.label = String::from(value),
                    _ => {}
                }
            }
        } else if section == "dock" && key == "entries" {
            dock_ids = parse_str_array(rhs);
            have_dock = true;
        } else if section == "launcher" && key == "entries" {
            launcher_ids = parse_str_array(rhs);
            have_launcher = true;
        } else if section == "startup" && key == "entries" {
            startup_ids = parse_str_array(rhs);
            have_startup = true;
        } else if section == "startup" && key == "self_test" {
            self_test = parse_bool(rhs).unwrap_or(false);
        } else if section == "workspaces" && (key == "count" || key == "n") {
            if let Some(n) = parse_uint(rhs) {
                workspaces = (n as usize).clamp(1, MAX_WS);
            }
        }
    }

    if apps.is_empty() {
        // Caller resolves an omitted registry from the canonical data document.
        return (Applications::baseline(), false);
    }

    // Fill any label left empty from the (possibly overridden) name.
    for app in apps.iter_mut() {
        if app.label.is_empty() {
            app.label = default_label(&app.name);
        }
    }

    let resolve = |ids: &[String], apps: &[AppEntry]| -> Vec<usize> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(i) = apps.iter().position(|a| &a.id == id) {
                out.push(i);
            }
        }
        out
    };
    let all: Vec<usize> = (0..apps.len()).collect();
    let inherited = |indices: &Vec<usize>| {
        if base.apps.is_empty() || saw_apps {
            all.clone()
        } else {
            indices.clone()
        }
    };
    let dock = if have_dock {
        resolve(&dock_ids, &apps)
    } else {
        inherited(&base.dock)
    };
    let launcher = if have_launcher {
        resolve(&launcher_ids, &apps)
    } else {
        inherited(&base.launcher)
    };
    let startup = if have_startup {
        resolve(&startup_ids, &apps)
    } else {
        base.startup.clone()
    };

    (
        Applications {
            apps,
            dock,
            launcher,
            startup,
            workspaces,
            self_test,
        },
        saw_apps,
    )
}

/// Derive a 1-4 char uppercase dock/menu label from a display name (fallback for
/// an `[application.*]` table with no explicit `label`). Takes leading
/// alphanumerics of the first word; `?` when the name has none.
fn default_label(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            for u in c.to_uppercase() {
                if out.len() < 4 {
                    out.push(u);
                }
            }
        } else if !out.is_empty() {
            break;
        }
        if out.len() >= 4 {
            break;
        }
    }
    if out.is_empty() {
        out.push('?');
    }
    out
}
