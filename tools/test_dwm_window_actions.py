#!/usr/bin/env python3
"""Exercise the packaged desktop through the shared QEMU harness.

The coordinates describe the default 1920x1080 desktop. Screenshots and the
manifest stay in the run directory for visual review.
"""
import argparse
import json
import re
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def actions(folder, scene="windows"):
    steps = []

    def wait(marker, occurrences=1):
        steps.append({"wait_marker": marker, "occurrences": occurrences, "timeout": 90})

    def motion(x=0, y=0, repeat=1):
        steps.append({"relative_motion": {"x": x, "y": y, "repeat": repeat, "interval": 0.08}})

    def go(x, y):
        steps.append({"relative_motion": {"x": -127, "y": -127, "repeat": 24, "interval": 0.01}})
        for axis, value in (("x", x), ("y", y)):
            count, remainder = divmod(value, 127)
            if count:
                steps.append({"relative_motion": {axis: 127, "repeat": count, "interval": 0.025}})
            if remainder:
                steps.append({"relative_motion": {axis: remainder, "interval": 0.025}})

    def button(down):
        steps.append({"execute": "input-send-event", "arguments": {
            "events": [{"type": "btn", "data": {"button": "left", "down": down}}]}})
        steps.append({"delay_ms": 200})

    def click(x, y):
        go(x, y)
        button(True)
        button(False)

    def drag(point, delta):
        go(*point)
        button(True)
        dx, dy = delta
        while dx or dy:
            x, y = max(-120, min(120, dx)), max(-120, min(120, dy))
            motion(x, y)
            dx -= x
            dy -= y
        button(False)

    def key(*keys, hold=40):
        steps.append({"execute": "send-key", "arguments": {
            "keys": [{"type": "qcode", "data": code} for code in keys], "hold-time": hold}})
        steps.append({"delay_ms": 100})

    def shot(name, resized=0):
        if resized:
            wait("gui_calc: reconfigured OK", resized)
        steps.append({"delay_ms": 3000})
        steps.append({"execute": "screendump", "arguments": {"filename": str(folder / f"{name}.ppm")}})

    def quicksettings():
        click(140, 12)
        shot("quicksettings")
        click(225, 43)
        shot("blur-disabled")
        click(140, 12)
        click(225, 43)
        shot("blur-restored")
        click(140, 12)
        shot("quicksettings-reopened")
        key("esc")
        shot("quicksettings-dismissed")

    wait("gui_server: desktop ready")
    if scene == "quicksettings":
        quicksettings()
        return steps
    if scene == "lifecycle":
        shot("clean-desktop")
    for x, app in ((870, "gui_calc"), (990, "gui_files"), (1050, "gui_terminal")):
        click(x, 1034)
        wait(f"[GUI-PRESENT] app={app}")
    wait("gui_calc: surface presented OK")
    shot("launched")
    if scene == "lifecycle":
        click(135, 43)
        shot("minimized")
        click(870, 1034)
        wait("gui_server: restored pid=")
        shot("restored")
        click(109, 43)
        wait("gui_server: reaped client pid=")
        shot("mouse-closed")
        click(870, 1034)
        wait("gui_calc: surface presented OK", 2)
        shot("reopened")
        key("meta_l", "q")
        wait("gui_server: reaped client pid=", 2)
        shot("shortcut-closed")
        click(350, 123)
        steps.append({"type_text": "echo DWM_CLOSE_REOPEN_OK", "key_interval_ms": 150})
        shot("terminal-after-close")
        steps.append({"type_text": "dtop --once", "key_interval_ms": 150})
        wait("CPU unavailable (no usage counter)")
        shot("process-count")
        return steps
    click(102, 110)
    shot("raised")
    drag((220, 43), (210, 214))  # Calculator outer origin becomes (300, 240).
    shot("moved")

    count = 0
    for name, point, delta in (
        ("left", (300, 400), (-30, 0)),
        ("right", (561, 400), (30, 0)),
        ("top", (430, 240), (0, -20)),
        ("bottom", (430, 633), (0, 20)),
        ("top-left", (300, 240), (-30, -20)),
        ("top-right", (561, 240), (30, -20)),
        ("bottom-left", (300, 633), (-30, 20)),
        ("bottom-right", (561, 633), (30, 20)),
    ):
        drag(point, delta)
        count += 1
        shot(name, count)
        moved_edge = (point[0] + delta[0], point[1] + delta[1])
        drag(moved_edge, (-delta[0], -delta[1]))
        count += 1
        wait("gui_calc: reconfigured OK", count)

    drag((561, 633), (-90, -90))
    count += 1
    shot("minimum", count)
    steps.append({"type_text": "12+3", "submit": False, "key_interval_ms": 200})
    click(464, 554)
    shot("minimum-calculator-15")
    drag((493, 581), (68, 52))
    count += 1
    shot("minimum-restored", count)
    steps.append({"type_text": "12+3=", "submit": False, "key_interval_ms": 200})
    shot("calculator-15")
    key("meta_l", "m")
    key("meta_l", "m")
    shot("rapid-max-restore")
    click(371, 257)
    shot("maximized", count + 1)
    drag((900, 43), (100, 120))
    shot("drag-from-maximized")

    click(350, 123)
    steps.append({"type_text": "echo DWM_USER_REVIEW_OK", "key_interval_ms": 150})
    shot("terminal-output")
    click(390, 83)
    key("home")
    key("ret")
    shot("files-navigation")
    quicksettings()
    key("meta_l", "2")
    shot("workspace-2")
    key("meta_l", "1")
    shot("workspace-1")
    steps.append({"execute": "input-send-event", "arguments": {"events": [
        {"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "alt"}}},
        {"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "tab"}}},
    ]}})
    shot("switcher")
    steps.append({"execute": "input-send-event", "arguments": {"events": [
        {"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "tab"}}},
        {"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "alt"}}},
    ]}})
    wait("gui_server: switcher raised pid=")
    shot("switcher-committed")
    return steps


def boot_fixture(folder):
    """Keep the clean-boot widget assertions without changing product defaults."""
    document = (ROOT / "assets/dwm/default.toml").read_text()
    updated, count = re.subn(
        r"(?m)(^\[widgets\]\n(?:[^\[].*\n)*?enabled\s*=\s*)false\b",
        r"\g<1>true", document, count=1,
    )
    if count != 1:
        raise ValueError("boot fixture requires an explicit disabled widget master")
    expected = tomllib.loads(document)
    expected["widgets"]["enabled"] = True
    if tomllib.loads(updated) != expected:
        raise ValueError("boot fixture changed more than widgets.enabled")
    overlay = folder / "overlay"
    config = overlay / "system/share/dwm/default.toml"
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text(updated)
    return overlay


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", type=Path, default=ROOT / "build/dwm-window-actions")
    parser.add_argument("--accel", choices=("tcg", "kvm", "auto"), default="tcg")
    parser.add_argument("--cpus", type=int, default=4)
    parser.add_argument("--scene", choices=("windows", "lifecycle", "quicksettings", "boot"), default="windows")
    parser.add_argument("--uefi", action="store_true")
    parser.add_argument("--reuse-image", action="store_true")
    args = parser.parse_args()
    if args.scene == "boot" and args.reuse_image:
        parser.error("--scene boot requires a build with its widget fixture")
    folder = args.run_dir.resolve()
    folder.mkdir(parents=True, exist_ok=True)
    manifest = folder / "markers.json"
    required = ["gui_server: desktop ready"]
    if args.scene in ("windows", "lifecycle"):
        required += ["gui_calc: surface presented OK", "[GUI-PRESENT] app=gui_files",
                     "[GUI-PRESENT] app=gui_terminal"]
    if args.scene == "lifecycle":
        required += ["DWM_CLOSE_REOPEN_OK", "CPU unavailable (no usage counter)"]
    contract = {
        "description": "Default desktop pointer, keyboard and application actions; inspect the captured frames.",
        "required": required,
        "forbidden": ["[PANIC]", "[USER-FAULT]", "gui_server: FAIL", "gui_calc: FAIL", "gui_files: FAIL", "gui_terminal: FAIL"],
        "run": {"config": "limine_dwm.conf"},
    }
    if args.scene == "boot":
        contract = json.loads((ROOT / "tools/gui_boot_markers.json").read_text())
        contract["run"]["actions"] = [
            {"delay_ms": 3000},
            {"execute": "screendump", "arguments": {"filename": str(folder / "desktop.ppm")}},
        ]
    else:
        contract["run"]["actions"] = actions(folder, args.scene)
    manifest.write_text(json.dumps(contract, indent=2) + "\n")
    command = [sys.executable, str(ROOT / "tools/qemu_test.py"), "--markers-file", str(manifest),
               "--ready-marker", "gui_server: desktop ready", "--accel", args.accel,
               "--cpus", str(args.cpus), "--workdir", str(folder), "--timeout", "800",
               "--boot-timeout", "120", "--post-boot-delay", "3", "--settle", "4"]
    if not args.reuse_image:
        command.append("--build")
    if args.scene == "boot":
        command += ["--initrd-overlay", str(boot_fixture(folder))]
    if args.uefi:
        command.append("--uefi")
    return subprocess.call(command, cwd=ROOT)


if __name__ == "__main__":
    raise SystemExit(main())
