# GUI Server & UI Kit

This article details the architecture of **`gui_server`** and the native component-based graphical user interface toolkit of **Dunit OS**.

---

## 1. System Role & Architecture

`gui_server` is the system service responsible for window management, surface transport, event dispatching, and high-level widget rendering:

```text
+-------------------------------------------------------------------------+
|                              Applications                               |
|        Files · System Monitor · Settings · Calculator · Demo            |
+------------------------------------+------------------------------------+
                                     |
                                     v (Synchronous IPC / Shared Memory)
+-------------------------------------------------------------------------+
|                               gui_server                                |
|  - Window Registry: Tracking window bounds, titles, active/minimized    |
|  - Event Dispatcher: Routing keyboard, mouse, and focus events          |
|  - Component Engine: Declarative widget tree (Button, Text, Grid, etc.) |
|  - Font Engine: TrueType rasterization & glyph caching                  |
|  - Theme Engine: Palette colors, paddings, shadows, and borders         |
+-------------------------------------------------------------------------+
                                     |
                                     v
+-------------------------------------------------------------------------+
|                       DWM v2 Compositor / Kernel                        |
+-------------------------------------------------------------------------+
```

---

## 2. Component Hierarchy & Layout Containers

Applications build user interfaces by defining a declarative tree of UI nodes. Each node specifies its parent index, visual attributes, and interaction payload.

### Container Kinds

| Container | Description | Typical Use |
|---|---|---|
| `Column` | Arranges child nodes vertically from top to bottom. | Form layouts, sidebar navigation, dialogs. |
| `Row` | Arranges child nodes horizontally from left to right. | Toolbars, button rows, status bars. |
| `Stack` | Layers child nodes on top of each other along the Z-axis. | Badges, overlay notifications, card media. |
| `Grid` | Arranges children into rows and columns with explicit cells. | Calculator keypads, dashboard metrics. |

---

## 3. Comprehensive Widget Catalog

### Controls & Indicators

* **Text (`DUNIT_UI_NODE_TEXT`)**: Renders static labels. Supports soft text-wrapping, text alignment (`Left`, `Center`, `Right`), and custom text colors.
* **Button (`DUNIT_UI_NODE_BUTTON`)**: Interactive clickable button. Emits an application-defined 64-bit message ID upon activation. Supports hover, pressed, and disabled states.
* **Input (`DUNIT_UI_NODE_INPUT`)**: Single-line text input field. Supports keyboard navigation, selection, backspace/delete, cursor blinking, and clipboard integration.
* **Textarea (`DUNIT_UI_NODE_TEXTAREA`)**: Multi-line text editor with vertical scrolling and word wrapping.
* **Checkbox (`DUNIT_UI_NODE_CHECKBOX`)**: Binary toggle box with associated text label.
* **Switch (`DUNIT_UI_NODE_SWITCH`)**: Modern sliding toggle switch for settings and preferences.
* **Radio (`DUNIT_UI_NODE_RADIO`)**: Mutually exclusive selection controls grouped by parent container.
* **Slider (`DUNIT_UI_NODE_SLIDER`)**: Horizontal and vertical continuous value selectors.
* **Spinbox (`DUNIT_UI_NODE_SPINBOX`)**: Numeric input with up/down stepper buttons.
* **Progress (`DUNIT_UI_NODE_PROGRESS`)**: Visual progress indicators supporting both determinate (0–100%) and indeterminate busy states.
* **Table (`DUNIT_UI_NODE_TABLE`)**: Multi-column tabular data grid with column headers and cell selection.
* **VirtualList & VirtualTree**: High-performance virtualized views that only instantiate visible rows, capable of displaying large datasets (e.g. 50,000+ file entries) with zero lag.

---

## 4. Green Tea Theming & Design System

The system UI employs the **Green Tea** design language, characterized by low eye-strain dark tones and organic sage-green accents:

| Token | Hex Value | Usage |
|---|---|---|
| `surface-base` | `#111618` | Application window background |
| `surface-elevated` | `#192023` | Cards, panels, input field backgrounds |
| `surface-border` | `#293438` | Subtle window and container dividers |
| `accent-primary` | `#6b9365` | Active buttons, focus borders, slider thumbs |
| `accent-hover` | `#7fa878` | Hovered interactive elements |
| `text-primary` | `#e4ece2` | Headings, primary body text |
| `text-secondary` | `#94a392` | Captions, placeholders, disabled labels |

---

## 5. Input Pipeline & Focus Routing

1. **PS/2 Mouse & Keyboard**: Low-level packets are translated by kernel drivers and delivered via IPC to `gui_server`.
2. **Hit Testing**: `gui_server` performs geometric hit-testing against the top-most active window and its child component hierarchy.
3. **Keyboard Focus & Tab Order**: Pressing `Tab` / `Shift+Tab` advances focus linearly through focusable controls (`Button`, `Input`, `Checkbox`, etc.).
4. **Drag & Drop (DnD)**: `gui_server` tracks mouse drag thresholds and manages drag payloads between different application windows using MIME type negotiation.
5. **Native File Dialog**: Applications can invoke the system file picker (`dunit_ui_file_dialog`) to allow users to select or save files with full VFS access permissions.

