# DWM Window Manager (Engine v2)

**DWM** (Dunit Window Manager) is the official desktop compositor and window manager for Dunit OS. In Engine v2, DWM operates entirely as an unprivileged userspace service featuring damage tracking, declarative multi-panel layout configuration, CSS-like styling via Dunit Style Sheets (DSS), and an extensible Lua animation and event engine.

---

## 1. Architectural Overview

```text
+-------------------------------------------------------------------------+
|                              Client Windows                             |
|          Terminal, Files, Calculator, GrafX2, Doom, Quake, etc.         |
+------------------------------------+------------------------------------+
                                     |
                                     v (Damage regions & configure events)
+-------------------------------------------------------------------------+
|                       DWM v2 Userspace Compositor                       |
|   - Scene Graph: Panels, Windows, Popups, Menus, Desktop Background     |
|   - Damage Tracker: Composes only dirty rectangles (partial frame sync) |
|   - Styling Engine: DSS parser (colors, margins, borders, radiuses)     |
|   - Lua SDK: Event hooks (`window::minimized`), animation curves        |
+-------------------------------------------------------------------------+
                                     |
                                     v (Linear Framebuffer & PS/2 Input)
+-------------------------------------------------------------------------+
|                              Dunit Kernel                               |
+-------------------------------------------------------------------------+
```

### Key Capabilities
* **Damage Tracking**: DWM avoids redrawing the entire screen every frame. Only damaged regions (such as moving cursors, updated text cells, or animated windows) are re-composed, drastically reducing memory bus bandwidth.
* **Declarative Desktop Layout**: Panels, docks, trays, and workspace widgets are defined entirely in TOML configurations without hardcoded layouts.
* **DSS (Dunit Style Sheets)**: A human-readable stylesheet format providing fine-grained control over colors, gradients, borders, shadows, and hover states.
* **Lua 5.4 Integration**: Window animations (Genie effect, scaling, fading) and workspace layout algorithms are scriptable via sandboxed Lua hooks.

---

## 2. Configuration (`default.toml`)

The desktop configuration lives at `/system/share/dwm/default.toml`. It uses `schema_version = 2`:

```toml
schema_version = 2

[panels]
explicit = true # Disables legacy hardcoded top panel and bottom dock

[panels.bar]
edge = "left"          # "top", "bottom", "left", or "right"
size = 46              # Width (for left/right) or height (for top/bottom) in pixels
length = 16384         # Extends full screen height
offset = 0
style = "left-bar"     # Refers to class in the DSS stylesheet
widgets = [
    "brand",           # Application launcher button
    "spaces",          # Workspace switcher (1..5)
    "top-spacer",      # Expanding spacer pushing apps to the vertical center
    "apps_sys",        # System applications block
    "sep1",            # 12px visual separator
    "apps_games",      # Games block
    "sep2",            # 12px visual separator
    "apps_soft",       # Third-party software block
    "bottom-spacer",   # Expanding spacer pushing clock to the bottom
    "clock"            # Digital clock widget
]

[launcher]
terminal = "/app/gui_terminal"
entries = [
    "sec_sys", "gui_terminal", "gui_files", "gui_calc", "gui_stat", "gui_settings",
    "sec_games", "doom", "quake", "mced", "gameboy",
    "sec_soft", "grafx2", "litexl", "player", "tracker"
]

[workspaces]
count = 5
```

---

## 3. Dunit Style Sheets (DSS)

DSS files (`/assets/dwm/styles/*.dss`) define the visual appearance of desktop components:

```css
/* Left Panel Base */
panel.left-bar {
    background: #0f140de0;
    border-right: 1px solid #23311f;
    padding: 6px 0px;
}

/* Launcher / Brand Button */
button.brand {
    margin: 4px auto;
    width: 32px;
    height: 32px;
    background: #273822;
    border-radius: 8px;
}

button.brand:hover {
    background: #3c5434;
}

/* Workspace Indicators */
button.space-item {
    color: #8da484;
    font-size: 11px;
    margin: 2px auto;
}

button.space-item.active {
    color: #ffffff;
    font-weight: bold;
    background: #3b5233;
    border-radius: 4px;
}

/* Dock Application Icons */
button.dock-item {
    margin: 3px auto;
    padding: 4px;
    border-radius: 8px;
}

button.dock-item:hover {
    background: #2b3b27;
}

button.dock-item:active {
    background: #476140;
}
```

---

## 4. Lua Scripting & Animation Engine

DWM embeds a Lua runtime (`docs/DWM_LUA_SDK.md`) allowing developers to customize window dynamics and animations.

Example script (`/assets/dwm/scripts/events/macos.lua`):
```lua
local dwm = require('dwm')
local animation = require('animation')
assert(dwm.api_version == 2)

-- Register a cubic ease-out easing curve
animation.register('gentle', function(t)
    return 1 - (1 - t)^3
end)

-- Window Creation: Fade in opacity from 0 to 1
dwm.on('window::created', function(win)
    dwm.animate(win, {
        property = 'opacity',
        from = 0,
        to = 1,
        duration = 200,
        easing = 'gentle',
    })
end)

-- Window Minimize: Genie effect collapsing into the dock icon
dwm.on('window::minimized', function(win)
    dwm.animate({
        window = win.id,
        target = win.target,
        effect = 'genie',
        duration_ms = 350,
        easing = 'ease_out',
    })
end)
```

### Supported Event Subscriptions

| Event Name | Description |
|---|---|
| `window::created` | Triggered when a new application window is mapped. |
| `window::closed` | Triggered when a window unmaps or terminates. |
| `window::focused` | Triggered when focus changes to a new window. |
| `window::minimized` | Triggered when a window is minimized to the dock. |
| `window::restored` | Triggered when a minimized window is restored to the desktop. |
| `window::maximized` | Triggered when a window toggles fullscreen or maximized state. |
| `workspace::changed` | Triggered when the user switches virtual desktops. |

---

## 5. Compositor IPC Socket

DWM exposes an IPC control endpoint at `/tmp/dwm.sock`. External utilities and CLI tools can communicate with DWM over this socket:
* Querying active windows and geometry.
* Toggling workspaces programmatically.
* Reloading configuration and DSS stylesheets on the fly (`dwm-msg reload`).

