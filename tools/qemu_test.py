#!/usr/bin/env python3
"""Build Dunit images and verify guest behavior through QEMU/QMP.

Keyboard input uses PS/2 scancodes; the guest has no serial input console.
Serial captures boot diagnostics and userspace stdout/stderr. Kernel-terminal
builtins draw to the framebuffer, so check screenshots for their output.
Marker manifests define commands, input actions and required/forbidden output.
Every automated run shuts down its VM and retains the serial log.

    python3 tools/qemu_test.py --build --markers-file tools/boot_markers.json
    python3 tools/qemu_test.py --build --cmd "exec args_test one two" --json
"""

from __future__ import annotations

import argparse
import json
import os
import selectors
import shutil
import signal
import socket
import subprocess
import sys
import time
import tempfile
from dataclasses import dataclass, field, asdict
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# Marker the kernel prints right before it enters the interactive terminal
# loop. Once we see it, keystrokes will be accepted.
DEFAULT_READY_MARKER = "[TERM-007] Starting main loop"

# Default LIMINE_CONFIG for --build. A markers manifest may override it.
DEFAULT_CONFIG = "limine_test_terminal.conf"

# ---------------------------------------------------------------------------
# ASCII -> QEMU qcode mapping for send-key. Values are (qcode, needs_shift).
# ---------------------------------------------------------------------------
_BASE = {
    "a": "a", "b": "b", "c": "c", "d": "d", "e": "e", "f": "f", "g": "g",
    "h": "h", "i": "i", "j": "j", "k": "k", "l": "l", "m": "m", "n": "n",
    "o": "o", "p": "p", "q": "q", "r": "r", "s": "s", "t": "t", "u": "u",
    "v": "v", "w": "w", "x": "x", "y": "y", "z": "z",
    "0": "0", "1": "1", "2": "2", "3": "3", "4": "4",
    "5": "5", "6": "6", "7": "7", "8": "8", "9": "9",
    " ": "spc", "-": "minus", "=": "equal", "[": "bracket_left",
    "]": "bracket_right", ";": "semicolon", "'": "apostrophe",
    "`": "grave_accent", ",": "comma", ".": "dot", "/": "slash",
    "\\": "backslash",
}
# Characters that require Shift on a US layout, mapped to the un-shifted qcode.
_SHIFTED = {
    "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7",
    "*": "8", "(": "9", ")": "0", "_": "minus", "+": "equal",
    "{": "bracket_left", "}": "bracket_right", ":": "semicolon",
    '"': "apostrophe", "~": "grave_accent", "<": "comma", ">": "dot",
    "?": "slash", "|": "backslash",
}


def char_to_keys(ch: str) -> list[str] | None:
    """Return the list of qcodes needed to type `ch`, or None if unsupported."""
    if ch == "\n":
        return ["ret"]
    if ch in _BASE:
        return [_BASE[ch]]
    if "A" <= ch <= "Z":
        return ["shift", _BASE[ch.lower()]]
    if ch in _SHIFTED:
        return ["shift", _SHIFTED[ch]]
    return None


@dataclass
class CommandResult:
    command: str
    serial_output: str
    duration_s: float
    timed_out: bool


@dataclass
class RunResult:
    ok: bool
    reason: str
    booted: bool
    iso: str
    boot_serial: str = ""
    commands: list[CommandResult] = field(default_factory=list)
    screenshot: str | None = None
    full_serial_len: int = 0
    full_serial_log: str | None = None
    exit_code: int | None = None
    # Marker budget (M0): required markers that were missing and forbidden
    # markers that showed up. Empty lists mean the marker budget passed.
    markers_checked: bool = False
    missing_markers: list[str] = field(default_factory=list)
    forbidden_hits: list[str] = field(default_factory=list)
    network_test: dict | None = None


class Qmp:
    """QMP framing over a UNIX socket or the emulator's local stdio pipes."""

    def __init__(self, path: Path, *, reader=None, writer=None):
        self.path = path
        self.sock: socket.socket | None = None
        self.reader = reader
        self.writer = writer
        self._buf = b""

    def connect(self, timeout: float = 20.0) -> None:
        deadline = time.monotonic() + timeout
        if self.reader is not None and self.writer is not None:
            os.set_blocking(self.reader.fileno(), False)
            os.set_blocking(self.writer.fileno(), False)
            self._read_json(timeout)
            self.execute("qmp_capabilities", timeout=max(0.0, deadline - time.monotonic()))
            return
        last_err: Exception | None = None
        while time.monotonic() < deadline:
            s = None
            try:
                s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.connect(str(self.path))
                self.sock = s
                break
            except OSError as err:
                if s is not None:
                    s.close()
                last_err = err
                time.sleep(0.1)
        if self.sock is None:
            raise RuntimeError(f"could not connect to QMP socket: {last_err}")
        # Read the greeting then negotiate capabilities.
        self._read_json(max(0.0, deadline - time.monotonic()))
        self.execute("qmp_capabilities", timeout=max(0.0, deadline - time.monotonic()))

    @staticmethod
    def _wait_pipe(stream, event, deadline: float) -> None:
        with selectors.DefaultSelector() as selector:
            selector.register(stream, event)
            if not selector.select(max(0.0, deadline - time.monotonic())):
                raise TimeoutError("QMP pipe timed out")

    def _read_json(self, timeout: float = 20.0) -> dict:
        deadline = time.monotonic() + timeout
        while True:
            nl = self._buf.find(b"\n")
            if nl >= 0:
                line = self._buf[:nl]
                self._buf = self._buf[nl + 1:]
                if line.strip():
                    return json.loads(line.decode("utf-8", "replace"))
                continue
            if self.sock is not None:
                self.sock.settimeout(max(0.0, deadline - time.monotonic()))
                chunk = self.sock.recv(65536)
            else:
                if self.reader is None:
                    raise RuntimeError("QMP is not connected")
                self._wait_pipe(self.reader, selectors.EVENT_READ, deadline)
                chunk = os.read(self.reader.fileno(), 65536)
            if not chunk:
                raise RuntimeError("QMP transport closed")
            self._buf += chunk

    def execute(self, command: str, arguments: dict | None = None, *, timeout: float = 20.0) -> dict:
        deadline = time.monotonic() + timeout
        msg = {"execute": command}
        if arguments:
            msg["arguments"] = arguments
        payload = (json.dumps(msg) + "\n").encode()
        if self.sock is not None:
            self.sock.settimeout(timeout)
            self.sock.sendall(payload)
        else:
            if self.writer is None:
                raise RuntimeError("QMP is not connected")
            remaining = memoryview(payload)
            while remaining:
                self._wait_pipe(self.writer, selectors.EVENT_WRITE, deadline)
                try:
                    written = os.write(self.writer.fileno(), remaining)
                except BlockingIOError:
                    continue
                remaining = remaining[written:]
        # Skip async events until we get a return/error.
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("QMP response timed out")
            reply = self._read_json(remaining)
            if "return" in reply or "error" in reply:
                return reply

    def send_key(self, qcodes: list[str], hold_ms: int = 40) -> None:
        keys = [{"type": "qcode", "data": code} for code in qcodes]
        self.execute("send-key", {"keys": keys, "hold-time": hold_ms})

    def screendump(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        descriptor, filename = tempfile.mkstemp(prefix=".capture-", suffix="-" + path.name,
                                                dir=path.parent)
        os.close(descriptor)
        capture = Path(filename)
        try:
            reply = self.execute("screendump", {"filename": str(capture)})
            if "error" in reply:
                raise RuntimeError(f"screendump failed: {reply['error']}")
            if not capture.is_file() or capture.stat().st_size == 0:
                raise RuntimeError(f"screendump produced no image: {path}")
            capture.replace(path)
        finally:
            capture.unlink(missing_ok=True)

    def quit(self) -> None:
        try:
            self.execute("quit")
        except Exception:
            pass

    def close(self) -> None:
        if self.sock is not None:
            try:
                self.sock.close()
            finally:
                self.sock = None
        for stream in (self.reader, self.writer):
            if stream is not None:
                stream.close()
        self.reader = self.writer = None


class SerialTail:
    """Follows the QEMU serial log file and lets callers read incremental text."""

    def __init__(self, path: Path):
        self.path = path
        self._fh = None
        self._data = ""

    def open(self, timeout: float = 20.0) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.path.exists():
                self._fh = self.path.open("r", encoding="utf-8", errors="replace")
                return
            time.sleep(0.05)
        raise RuntimeError(f"serial log never appeared: {self.path}")

    def pump(self) -> str:
        """Read whatever new text is available, append to buffer, return delta."""
        if self._fh is None:
            return ""
        delta = self._fh.read()
        if delta:
            self._data += delta
        return delta

    @property
    def text(self) -> str:
        return self._data

    def wait_for_marker(self, marker: str, timeout: float) -> bool:
        deadline = time.monotonic() + timeout
        start = len(self._data)
        while time.monotonic() < deadline:
            self.pump()
            if marker in self._data[max(0, start - len(marker)):]:
                return True
            time.sleep(0.05)
        return False

    def collect_until_idle(self, idle_s: float, max_s: float) -> str:
        """Collect output until the serial line is quiet for `idle_s`, or `max_s`."""
        start_len = len(self._data)
        deadline = time.monotonic() + max_s
        last_change = time.monotonic()
        while time.monotonic() < deadline:
            delta = self.pump()
            if delta:
                last_change = time.monotonic()
            elif time.monotonic() - last_change >= idle_s:
                break
            time.sleep(0.05)
        else:
            self.pump()
        return self._data[start_len:]


def load_markers(markers_file: str | None,
                 require: list[str] | None,
                 forbid: list[str] | None) -> tuple[list[str], list[str | dict], dict]:
    """Merge a markers manifest file with CLI --require/--forbid markers.

    Returns (required, forbidden, run_defaults). `run_defaults` carries an
    optional {"config", "cmd"} block from the manifest so a single file both
    describes how to boot and what must be observed.
    """
    required: list[str] = []
    forbidden: list[str | dict] = []
    run_defaults: dict = {}
    if markers_file:
        data = json.loads(Path(markers_file).read_text(encoding="utf-8"))
        for entry in data.get("required", []):
            required.append(entry["marker"] if isinstance(entry, dict) else entry)
        for entry in data.get("forbidden", []):
            if isinstance(entry, dict) and "after" in entry:
                if not isinstance(entry["after"], str) or not entry["after"]:
                    raise ValueError("forbidden marker phase must be a nonempty string")
                forbidden.append({"marker": entry["marker"], "after": entry["after"]})
                if entry["after"] not in required:
                    required.append(entry["after"])
            else:
                forbidden.append(entry["marker"] if isinstance(entry, dict) else entry)
        run_defaults = data.get("run", {}) or {}
    required.extend(require or [])
    forbidden.extend(forbid or [])
    return required, forbidden, run_defaults


def forbidden_marker_hits(text: str, markers: list[str | dict]) -> list[str]:
    """Global failures or failures after an explicitly required phase boundary."""
    hits = []
    for entry in markers:
        marker = entry["marker"] if isinstance(entry, dict) else entry
        start = 0
        if isinstance(entry, dict):
            boundary = entry["after"]
            offset = text.find(boundary)
            if offset < 0:  # load_markers also requires the boundary itself
                continue
            start = offset + len(boundary)
        if marker in text[start:]:
            hits.append(marker)
    return hits


def build_iso(config: str, variant: str | None = None, overlay: Path | None = None, profile: str = "full") -> Path:
    """Build the ISO via the Makefile. Returns the ISO path."""
    if shutil.which("make") is None:
        raise RuntimeError("`make` not found; cannot --build")
    selection = f"variant={variant}" if variant else f"LIMINE_CONFIG={config}"
    print(f"[qemu_test] building ISO ({selection}) ...", file=sys.stderr)
    subprocess.run(
        (["make", f"iso-{variant}"] if variant else ["make", "iso", f"LIMINE_CONFIG={config}"])
        + ([f"INITRD_OVERLAY={overlay.resolve()}"] if overlay is not None else [])
        + ([f"IMAGE_PROFILE={profile}"] if not variant else []),
        cwd=REPO_ROOT,
        check=True,
    )
    iso = REPO_ROOT / "build" / (f"dunit_{variant}.iso" if variant else "microkernel.iso")
    if not iso.is_file():
        raise RuntimeError("build succeeded but ISO not found")
    return iso


def network_test_config(config: str, workdir: Path) -> str:
    """Copy a smoke config, preserving the basename that enables its checks."""
    from pack_initrd import SMOKE_CONFIGS

    source = Path(config)
    if not source.is_absolute():
        source = REPO_ROOT / source
    if source.name not in SMOKE_CONFIGS:
        raise ValueError("--net-test builds require a boot-smoke configuration")
    lines = source.read_text().splitlines(keepends=True)
    matches = [i for i, line in enumerate(lines) if line.lstrip().startswith("cmdline:")]
    if len(matches) != 1:
        raise ValueError("--net-test requires exactly one kernel cmdline")
    i = matches[0]
    prefix, command = lines[i].split("cmdline:", 1)
    tokens = command.split()
    if any(token.startswith("net_irq_test=") and token != "net_irq_test=1" for token in tokens):
        raise ValueError("conflicting network diagnostic boot option")
    if "net_irq_test=1" not in tokens:
        tokens.append("net_irq_test=1")
    lines[i] = prefix + "cmdline: " + " ".join(tokens) + "\n"
    output = workdir / "net-config" / source.name
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("".join(lines))
    return str(output.resolve())


def build_disk(config: str, workdir: Path) -> Path:
    """Build a raw BIOS/UEFI bootable disk image via install_disk.py.

    Unlike build_iso(), this path needs NO xorriso: it uses parted, mkfs.fat,
    mtools, and the bundled Limine binary. That makes a fully self-contained,
    zero-touch AI build+boot+test loop possible even on hosts without xorriso.
    Returns the disk image path.
    """
    if shutil.which("make") is None:
        raise RuntimeError("`make` not found; cannot --build-disk")
    print(f"[qemu_test] building disk image (config={config}) ...", file=sys.stderr)
    # Compile kernel + userspace artifacts the installer consumes. Pass the
    # config through to `make` so the kernel honours the boot-smoke-tests
    # feature gate for the test configs (the Makefile keys KERNEL_FEATURES off
    # the LIMINE_CONFIG basename). Without this the disk kernel would always be
    # built from the default limine.conf (feature OFF) and the in-kernel
    # DunitFS self-tests — which only run on the disk boot, since /persist is
    # mounted only from a real partition — would never be present.
    subprocess.run(
        ["make", "all", "userspace", f"LIMINE_CONFIG={config}"],
        cwd=REPO_ROOT,
        check=True,
    )
    workdir.mkdir(parents=True, exist_ok=True)
    image = workdir / "dunit-test-disk.img"
    subprocess.run(
        [
            sys.executable,
            str(REPO_ROOT / "tools" / "install_disk.py"),
            str(image),
            "--no-build",
            "--config",
            config,
            "--yes-i-know-this-erases-the-disk",
        ],
        cwd=REPO_ROOT,
        check=True,
    )
    if not image.is_file():
        raise RuntimeError("disk build succeeded but image not found")
    return image


def type_command(qmp: Qmp, text: str, key_interval: float = 0.05, submit: bool = True) -> None:
    sequence = []
    for ch in text:
        keys = char_to_keys(ch)
        if keys is None:
            raise ValueError(f"cannot type {ch!r} with the guest US keyboard layout")
        sequence.append(keys)
    for keys in sequence:
        qmp.send_key(keys)
        # send-key replies before its timed key release. Overlapping the next
        # character can leave Shift held after an underscore/quote and corrupt
        # the command. Let the 40 ms press finish before sending another key.
        time.sleep(key_interval)
    if submit:
        qmp.send_key(["ret"])
        time.sleep(0.05)


def run_command_actions(qmp: Qmp, serial: SerialTail, steps: list[dict],
                        output_start: int, deadline: float) -> None:
    """Execute manifest input actions while a command is running in the guest.

    Marker waits only inspect this command's output. Every wait and injection
    shares the command deadline, so an unresponsive guest cannot hang the run.
    """
    measurements = {}
    for step in steps:
        if time.monotonic() >= deadline:
            raise RuntimeError("command actions exceeded command timeout")
        if "measure_start" in step:
            label = step["measure_start"]
            if not isinstance(label, str) or not label or label in measurements:
                raise ValueError("measurement needs a unique nonempty label")
            measurements[label] = time.perf_counter_ns()
        elif "measure_end" in step:
            label = step["measure_end"]
            if label not in measurements:
                raise ValueError("measurement was not started")
            elapsed = time.perf_counter_ns() - measurements.pop(label)
            print("[INPUT-TIMING] " + json.dumps({"phase": label, "wall_ns": elapsed}), flush=True)
        elif "wait_marker" in step:
            marker = step["wait_marker"]
            occurrences=step.get("occurrences",1)
            if not isinstance(occurrences,int) or not 1<=occurrences<=1024:
                raise ValueError("marker occurrences must be between 1 and 1024")
            wait_deadline = min(deadline, time.monotonic() + float(step.get("timeout", 20)))
            while True:
                serial.pump()
                if serial.text[output_start:].count(marker)>=occurrences:
                    break
                if time.monotonic() >= wait_deadline:
                    raise RuntimeError(f"command action marker not seen: {marker!r}")
                time.sleep(0.02)
        elif "type_text" in step:
            # Slow software-rendered editors need realistic input pacing; keep
            # the existing terminal default and bound manifest overrides.
            interval = float(step.get("key_interval_ms", 50)) / 1000
            if not 0.05 <= interval <= 1:
                raise ValueError("key interval must be between 50 and 1000 ms")
            submit = step.get("submit", True)
            if not isinstance(submit, bool):
                raise ValueError("type_text submit must be a boolean")
            type_command(qmp, str(step["type_text"]), interval, submit)
        elif "delay_ms" in step:
            delay = float(step["delay_ms"]) / 1000
            if not 0 <= delay <= 5:
                raise ValueError("input action delay must be between 0 and 5000 ms")
            time.sleep(min(delay, max(0, deadline - time.monotonic())))
        elif "execute" in step:
            if step["execute"] == "screendump":
                path = Path(step["arguments"]["filename"])
                qmp.screendump(path if path.is_absolute() else REPO_ROOT / path)
                continue
            reply = qmp.execute(step["execute"], step.get("arguments"))
            if "error" in reply:
                raise RuntimeError(f"command action QMP failed: {reply['error']}")
        elif "relative_motion" in step:
            motion = step["relative_motion"]
            dx, dy = int(motion.get("x", 0)), int(motion.get("y", 0))
            repeat = int(motion.get("repeat", 1))
            interval = float(motion.get("interval", 0.03))
            if not (-127 <= dx <= 127 and -127 <= dy <= 127):
                raise ValueError("relative motion must fit a PS/2 packet (-127..127)")
            if not (1 <= repeat <= 1024 and 0 <= interval <= 1):
                raise ValueError("relative motion repeat/interval outside supported bounds")
            events = [{"type": "rel", "data": {"axis": axis, "value": value}}
                      for axis, value in (("x", dx), ("y", dy)) if value]
            for _ in range(repeat):
                if time.monotonic() >= deadline:
                    raise RuntimeError("relative motion exceeded command timeout")
                reply = qmp.execute("input-send-event", {"events": events})
                if "error" in reply:
                    raise RuntimeError(f"relative input injection failed: {reply['error']}")
                serial.pump()
                time.sleep(min(interval, max(0, deadline - time.monotonic())))
        else:
            raise ValueError(f"unknown command action: {step!r}")
    if measurements:
        raise ValueError("unfinished input measurement")


def resolve_ovmf() -> tuple[Path, Path]:
    """Locate an OVMF firmware (CODE, VARS) pair for UEFI boot. Raises if none."""
    code_candidates = [
        "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
        "/usr/share/edk2-ovmf/x64/OVMF_CODE.4m.fd",
        "/usr/share/ovmf/x64/OVMF_CODE.4m.fd",
        "/usr/share/OVMF/OVMF_CODE_4M.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
    ]
    vars_candidates = [
        "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
        "/usr/share/edk2-ovmf/x64/OVMF_VARS.4m.fd",
        "/usr/share/ovmf/x64/OVMF_VARS.4m.fd",
        "/usr/share/OVMF/OVMF_VARS_4M.fd",
        "/usr/share/OVMF/OVMF_VARS.fd",
    ]
    code = next((Path(p) for p in code_candidates if Path(p).is_file()), None)
    vars_ = next((Path(p) for p in vars_candidates if Path(p).is_file()), None)
    if code is None or vars_ is None:
        raise RuntimeError("OVMF firmware not found for --uefi (install edk2-ovmf)")
    return code, vars_


def build_qemu_command(image: Path, is_disk: bool, accel: str, mem: str,
                       qmp_sock: Path, serial_log: Path, extra: list[str],
                       display: str = "none", qmp_transport: str = "unix") -> list[str]:
    cmd = [
        "qemu-system-x86_64",
        "-no-reboot",
        "-m", mem,
        "-display", display,
        "-serial", f"file:{serial_log}",
        "-qmp", "stdio" if qmp_transport == "stdio" else f"unix:{qmp_sock},server,nowait",
        "-vga", "std", "-global", "VGA.vgamem_mb=32",
    ]
    if display != "none":
        # Interactive session: give the guest a smooth absolute pointer so the
        # user can click around without pointer-capture warping. Keyboard is the
        # default i8042 the kernel already drives. q35 has no USB by default, so
        # add an xHCI controller for the tablet to attach to.
        cmd += ["-device", "qemu-xhci", "-device", "usb-tablet"]

    if is_disk:
        cmd += ["-drive", f"file={image},format=raw,if=ide", "-boot", "c"]
    else:
        cmd += ["-cdrom", str(image), "-boot", "d"]
    if accel == "kvm":
        cmd += ["-machine", "q35,accel=kvm", "-cpu", "host"]
    elif accel == "tcg":
        cmd += ["-machine", "q35,accel=tcg"]
    else:  # auto
        if os.access("/dev/kvm", os.W_OK):
            cmd += ["-machine", "q35,accel=kvm", "-cpu", "host"]
        else:
            cmd += ["-machine", "q35,accel=tcg"]
    cmd += extra
    return cmd


def run(args: argparse.Namespace) -> RunResult:
    workdir = Path(args.workdir) if args.workdir else REPO_ROOT / "build"
    workdir.mkdir(parents=True, exist_ok=True)

    required, forbidden, run_defaults = load_markers(
        args.markers_file, args.require_marker, args.forbid_marker)
    # A markers manifest may define how to boot (config + commands). CLI flags
    # win; the manifest fills gaps so `--markers-file` alone is enough.
    if run_defaults.get("config") and args.config == DEFAULT_CONFIG and args.build:
        args.config = run_defaults["config"]
    if run_defaults.get("cmd") and not args.cmd:
        args.cmd = list(run_defaults["cmd"])
    if run_defaults.get("scratch_disk_mib") and not args.scratch_disk:
        args.scratch_disk = int(run_defaults["scratch_disk_mib"])
    if run_defaults.get("scratch_bus") and args.scratch_bus == "ahci":
        args.scratch_bus = str(run_defaults["scratch_bus"])
    if run_defaults.get("uefi") and not args.uefi:
        args.uefi = bool(run_defaults["uefi"])
    net_test = bool(getattr(args, "net_test", False) or run_defaults.get("net_test"))
    net_live = bool(getattr(args, "net_live", False) or run_defaults.get("net_live"))
    net_dhcp = bool(getattr(args, "net_dhcp", False) or run_defaults.get("net_dhcp"))
    net_static = bool(getattr(args, "net_static", False) or run_defaults.get("net_static"))
    net_socket = bool(getattr(args, "net_socket", False) or run_defaults.get("net_socket"))
    net_dns = bool(getattr(args, "net_dns", False) or run_defaults.get("net_dns"))
    net_dns_posix = bool(getattr(args, "net_dns_posix", False) or run_defaults.get("net_dns_posix"))
    net_listen = bool(getattr(args, "net_listen", False) or run_defaults.get("net_listen"))
    net_listen_posix = bool(getattr(args, "net_listen_posix", False) or run_defaults.get("net_listen_posix"))
    net_reno = bool(getattr(args, "net_reno", False) or run_defaults.get("net_reno"))
    peers = sum((net_test, net_live, net_dhcp, net_static, net_socket, net_dns, net_dns_posix, net_listen, net_listen_posix, net_reno))
    if peers > 1:
        raise ValueError("choose one network test peer")
    if peers and (args.interactive or args.variant or args.disk or args.build_disk):
        raise ValueError("network peers require an automated developer ISO run")
    if args.ready_marker == DEFAULT_READY_MARKER and run_defaults.get("ready_marker"):
        args.ready_marker = str(run_defaults["ready_marker"])
    if args.initrd_overlay is None and run_defaults.get("initrd_overlay"):
        args.initrd_overlay = REPO_ROOT / str(run_defaults["initrd_overlay"])

    is_disk = False
    if args.build:
        if net_test:
            args.config = network_test_config(args.config, workdir)
        image = build_iso(args.config, args.variant, args.initrd_overlay, args.image_profile)
    elif args.build_disk:
        image = build_disk(args.config, workdir)
        is_disk = True
    elif args.disk:
        image = Path(args.disk)
        is_disk = True
        if not image.is_file():
            return RunResult(ok=False, reason=f"disk image not found: {image}",
                             booted=False, iso=str(image))
    else:
        image = Path(args.iso) if args.iso else REPO_ROOT / "build" / "microkernel.iso"
        if not image.is_file():
            return RunResult(ok=False, reason=f"ISO not found: {image}",
                             booted=False, iso=str(image))
    qmp_sock = workdir / "qemu-qmp.sock"
    serial_log = workdir / "qemu-serial.log"
    for stale in (qmp_sock, serial_log):
        try:
            stale.unlink()
        except FileNotFoundError:
            pass

    display = args.display if getattr(args, "interactive", False) else "none"
    extra_args = list(args.qemu_arg or [])
    cpus = getattr(args, "cpus", None)
    if cpus is not None:
        if not 1 <= cpus <= 256:
            raise ValueError("--cpus must be between 1 and 256")
        if any(a == "-smp" or a.startswith("-smp=") for a in extra_args):
            raise ValueError("use --cpus or raw -smp, not both")
        extra_args += ["-smp", str(cpus)]
    # UEFI: prepend an OVMF pflash pair (CODE read-only, a writable VARS copy) so
    # the same image boots through its EFI path instead of SeaBIOS.
    if args.uefi:
        code, vars_src = resolve_ovmf()
        vars_copy = workdir / "ovmf-vars.fd"
        shutil.copyfile(vars_src, vars_copy)
        extra_args += [
            "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={code}",
            "-drive", f"if=pflash,format=raw,unit=1,file={vars_copy}",
        ]
        print(f"[qemu_test] UEFI firmware: {code}", file=sys.stderr)
    # A blank scratch disk lets the in-kernel installer self-test provision a
    # pristine target. On an ISO boot the CD is ATAPI (the AHCI driver skips it),
    # so an AHCI scratch is the only SATA disk and enumerates as sda; a virtio
    # scratch registers as vd0.
    if args.scratch_disk:
        scratch = workdir / "dunit-scratch.img"
        try:
            scratch.unlink()
        except FileNotFoundError:
            pass
        with open(scratch, "wb") as fh:
            fh.truncate(int(args.scratch_disk) * 1024 * 1024)
        if args.scratch_bus == "virtio":
            extra_args += [
                "-drive", f"file={scratch},format=raw,if=none,id=dunitscratch",
                "-device", "virtio-blk-pci,drive=dunitscratch,disable-modern=on,disable-legacy=off",
            ]
        else:
            extra_args += ["-drive", f"file={scratch},format=raw,if=ide"]
        print(f"[qemu_test] blank scratch disk: {scratch} "
              f"({args.scratch_disk} MiB, bus={args.scratch_bus})", file=sys.stderr)
    qmp_transport = getattr(args, "qmp_transport", "unix")
    peer = None
    if peers:
        if any(arg in ("-nic", "-netdev", "-net") or
               arg.startswith(("-nic=", "-netdev=", "-net=")) or
               arg.startswith("e1000,") for arg in extra_args):
            raise ValueError("network peer owns its reference NIC and backend")
        if net_reno:
            from net_reno_transport import NetRenoPeer
            peer = NetRenoPeer(workdir)
        elif net_listen or net_listen_posix:
            from net_listen_transport import NetListenPeer
            peer = NetListenPeer(workdir)
        elif net_dns or net_dns_posix:
            from net_dns_transport import NetDnsPeer
            peer = NetDnsPeer(workdir, posix=net_dns_posix)
        elif net_socket:
            from net_socket_transport import NetSocketPeer
            peer = NetSocketPeer(workdir)
        elif net_static:
            from net_static_transport import NetStaticPeer
            peer = NetStaticPeer(workdir)
        elif net_dhcp:
            from net_dhcp_transport import NetDhcpPeer, NetDhcpPersistencePeer
            phase = run_defaults.get("dhcp_persistence")
            if phase is None:
                peer = NetDhcpPeer(workdir, reboot=bool(run_defaults.get("dhcp_reboot")))
            else:
                peer = NetDhcpPersistencePeer(workdir, phase)
        elif net_live:
            from net_live_transport import NetLivePeer
            peer = NetLivePeer(workdir)
        else:
            from net_test_transport import NetPeer
            peer = NetPeer(workdir)
        peer.start()
        extra_args += ["-nic", "none", "-netdev",
                       f"socket,id=dunit-test,connect={peer.endpoint}",
                       "-device", "e1000,netdev=dunit-test,mac=52:54:00:12:34:56"]
    qemu_cmd = build_qemu_command(image, is_disk, args.accel, args.mem, qmp_sock,
                                  serial_log, extra_args, display, qmp_transport)
    print("[qemu_test] launch:", " ".join(qemu_cmd), file=sys.stderr)

    metadata = None
    metadata_path = getattr(args, "performance_record", None)
    result = RunResult(ok=False, reason="", booted=False, iso=str(image))
    launch_started = time.monotonic()
    try:
        if metadata_path:
            from performance_metadata import collect

            metadata = collect(REPO_ROOT, image, qemu_cmd, args.image_profile)
        with (workdir / "qemu-stderr.log").open("wb") as qemu_stderr:
            proc = subprocess.Popen(
                qemu_cmd, cwd=REPO_ROOT, stderr=qemu_stderr,
                stdout=subprocess.PIPE if qmp_transport == "stdio" else subprocess.DEVNULL,
                stdin=subprocess.PIPE if qmp_transport == "stdio" else subprocess.DEVNULL,
                bufsize=0,
            )
    except BaseException:
        if peer is not None:
            peer.stop()
        raise
    hard_deadline = time.monotonic() + args.timeout
    qmp = (Qmp(qmp_sock, reader=proc.stdout, writer=proc.stdin)
           if qmp_transport == "stdio" else Qmp(qmp_sock))
    serial = SerialTail(serial_log)

    def remaining() -> float:
        return max(0.0, hard_deadline - time.monotonic())

    def force_quit(reason: str) -> None:
        result.reason = result.reason or reason
        try:
            qmp.quit()
        except Exception:
            pass
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.send_signal(signal.SIGKILL)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass

    try:
        qmp.connect(timeout=min(20.0, args.timeout))
        serial.open(timeout=min(20.0, args.timeout))
        if metadata is not None:
            cpus_reply = qmp.execute("query-cpus-fast")
            if isinstance(cpus_reply.get("return"), list):
                metadata["guest"]["emulator_vcpus"] = len(cpus_reply["return"])

        if args.interactive:
            # Human-in-the-loop mode: a real QEMU window is open on the user's
            # display. Do not inject keystrokes, do not force-quit on a timeout —
            # just keep the serial log flowing and wait until the user closes the
            # window (or Ctrl-C here). The serial log is still persisted for
            # post-session analysis.
            print("[qemu_test] interactive window open — poke the system by hand; "
                  "close the QEMU window (or Ctrl-C) to end.", file=sys.stderr)
            try:
                while proc.poll() is None:
                    serial.pump()
                    time.sleep(0.2)
            except KeyboardInterrupt:
                pass
            force_quit("interactive session ended")
            result.ok = True
            result.booted = True
            result.reason = "interactive session ended"
            return result

        # Wait for the terminal to be ready to accept keystrokes.
        boot_budget = min(args.boot_timeout, remaining())
        booted = serial.wait_for_marker(args.ready_marker, boot_budget)
        result.booted = booted
        result.boot_serial = serial.text
        if metadata is not None and booted:
            metadata["measurements"]["launch_to_ready_wall_ms"] = (
                time.monotonic() - launch_started
            ) * 1000
        if not booted:
            force_quit(
                f"boot marker {args.ready_marker!r} not seen in {boot_budget:.0f}s"
            )
            result.ok = False
            return result

        # The terminal drains its i8042 buffer right before the loop starts, so
        # give it a beat before typing or the first keystrokes get discarded.
        time.sleep(args.post_boot_delay)
        serial.pump()
        for command in args.cmd or []:
            if remaining() <= 1.0:
                result.reason = "hard timeout before running all commands"
                break
            before = len(serial.text)
            type_command(qmp, command)
            cmd_budget = min(args.cmd_timeout, remaining())
            t0 = time.monotonic()
            actions = run_defaults.get("command_actions", {}).get(command, [])
            run_command_actions(qmp, serial, actions, before, t0 + cmd_budget)
            serial.collect_until_idle(args.settle, max(0, cmd_budget - (time.monotonic() - t0)))
            out = serial.text[before:]
            dur = time.monotonic() - t0
            result.commands.append(CommandResult(
                command=command,
                serial_output=out,
                duration_s=round(dur, 2),
                timed_out=(dur >= cmd_budget - 0.05),
            ))

        # Boot-scene waits can refer to this VM's startup output. Command waits
        # above remain scoped to their own launch.
        # The per-command budget applies only to command_actions above.
        if (net_dhcp or net_static or net_socket or net_dns or net_dns_posix or net_listen or net_listen_posix or net_reno) and not peer.wait_complete(remaining()):
            raise RuntimeError("IPv4 peer did not complete the wire exchange")
        run_command_actions(qmp, serial, run_defaults.get("actions", []),
                            0, hard_deadline)

        if args.screenshot:
            shot = Path(args.screenshot)
            if not shot.is_absolute():
                shot = workdir / shot
            qmp.screendump(shot)
            result.screenshot = str(shot)

        # Optionally linger until the hard timeout to keep collecting serial.
        if args.linger:
            end = min(hard_deadline, time.monotonic() + args.linger)
            while time.monotonic() < end:
                serial.pump()
                time.sleep(0.1)

        force_quit("completed")
        result.ok = True
        if not result.reason:
            result.reason = "completed"
    except Exception as err:
        force_quit(f"error: {err}")
        result.ok = False
        result.reason = f"error: {err}"
    finally:
        if proc.poll() is None:
            force_quit("run stopped")
        serial.pump()
        qmp.close()
        if peer is not None:
            result.network_test = peer.stop()
            if not result.network_test.get("ok", False):
                result.ok = False
                result.reason += "; network peer verification failed"
        result.full_serial_len = len(serial.text)
        result.exit_code = proc.returncode
        # Persist the full serial log alongside the run for later inspection.
        try:
            full_serial_log = workdir / "qemu-serial.full.log"
            full_serial_log.write_text(serial.text, encoding="utf-8")
            result.full_serial_log = str(full_serial_log)
        except OSError as error:
            result.ok = False
            result.reason += f"; cannot save full serial log: {error}"

        # Marker budget (M0): required markers must all appear and forbidden
        # markers must not. This is what ties a WORKING claim to real evidence.
        if required or forbidden:
            result.markers_checked = True
            text = serial.text
            result.missing_markers = [m for m in required if m not in text]
            result.forbidden_hits = forbidden_marker_hits(text, forbidden)
            if result.missing_markers or result.forbidden_hits:
                marker_reason = (
                    f"marker budget failed: {len(result.missing_markers)} missing, "
                    f"{len(result.forbidden_hits)} forbidden")
                # Keep an existing failure reason (e.g. boot failure) primary.
                if result.ok:
                    result.reason = marker_reason
                else:
                    result.reason = f"{result.reason}; {marker_reason}"
                result.ok = False

        if metadata is not None:
            if result.network_test is not None:
                metadata["network_test"] = result.network_test
            import re

            cpu_state = re.search(
                r"\[SMP\] hardware_online=(\d+) scheduler=(\w+) dispatch_cpus=(\d+)",
                serial.text,
            )
            if cpu_state:
                metadata["guest"].update(
                    kernel_online_cpus=int(cpu_state[1]),
                    kernel_scheduler=cpu_state[2],
                    dispatch_cpus=int(cpu_state[3]),
                )
            clock_state = re.search(r"\[CLOCK\] source=(\w+) resolution_ns=(\d+) event=([\w-]+)", serial.text)
            if clock_state:
                metadata["clock"].update(guest_source=clock_state[1], guest_resolution_ns=int(clock_state[2]), guest_timer_event=clock_state[3])
            metadata["measurements"]["run_wall_ms"] = (
                time.monotonic() - launch_started
            ) * 1000
            metadata["result"] = {
                "ok": result.ok,
                "reason": result.reason,
                "missing": result.missing_markers,
                "forbidden": result.forbidden_hits,
            }
            output = Path(metadata_path)
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps(metadata, indent=2) + "\n")
    return result


def print_human(result: RunResult) -> None:
    print("=" * 72)
    print(f"ISO       : {result.iso}")
    print(f"Booted    : {result.booted}")
    print(f"Result    : {'OK' if result.ok else 'FAIL'} ({result.reason})")
    print(f"Serial log: {result.full_serial_log or 'unavailable'} ({result.full_serial_len} bytes)")
    if result.markers_checked:
        if not result.missing_markers and not result.forbidden_hits:
            print("Markers   : OK (marker budget satisfied)")
        else:
            print("Markers   : FAIL")
            for m in result.missing_markers:
                print(f"  MISSING required: {m!r}")
            for m in result.forbidden_hits:
                print(f"  FORBIDDEN seen  : {m!r}")
    if result.screenshot:
        print(f"Screenshot: {result.screenshot}")
    if not result.booted:
        print("-" * 72)
        print("BOOT SERIAL (tail):")
        print("\n".join(result.boot_serial.splitlines()[-40:]))
    for i, cmd in enumerate(result.commands, 1):
        print("-" * 72)
        flag = " [TIMED OUT]" if cmd.timed_out else ""
        print(f"[{i}] $ {cmd.command}  ({cmd.duration_s}s){flag}")
        text = cmd.serial_output.strip("\n")
        if text:
            print(text)
        else:
            print("(no serial output; builtins render to framebuffer only)")
    print("=" * 72)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    src = parser.add_mutually_exclusive_group()
    src.add_argument("--iso", help="path to an existing bootable ISO (cdrom boot)")
    src.add_argument("--disk", help="path to a raw bootable disk image (hdd boot)")
    src.add_argument("--build", action="store_true",
                     help="build the ISO with `make iso` before booting (needs xorriso)")
    src.add_argument("--build-disk", action="store_true",
                     help="build a raw bootable disk image and boot it (no xorriso needed)")
    parser.add_argument("--initrd-overlay", type=Path, help="VFS overlay for a development --build")
    parser.add_argument("--variant", choices=("dwm", "minimal"),
                        help="build a release variant with --build, using its own profile and boot config")
    parser.add_argument("--config", default="limine_test_terminal.conf",
                        help="LIMINE_CONFIG for --build (default: terminal test config)")
    parser.add_argument("--cmd", action="append", metavar="COMMAND",
                        help="a terminal command to type (repeatable, in order)")
    parser.add_argument("--timeout", type=float, default=90.0,
                        help="hard wall-clock budget in seconds; VM is force-quit after (default 90)")
    parser.add_argument("--boot-timeout", type=float, default=60.0,
                        help="max seconds to wait for the terminal ready marker (default 60)")
    parser.add_argument("--cmd-timeout", type=float, default=15.0,
                        help="max seconds to collect output per command (default 15)")
    parser.add_argument("--settle", type=float, default=1.5,
                        help="treat a command as done after this many seconds of serial silence (default 1.5)")
    parser.add_argument("--post-boot-delay", type=float, default=1.5,
                        help="seconds to wait after the ready marker before typing (default 1.5)")
    parser.add_argument("--linger", type=float, default=0.0,
                        help="after commands, keep collecting serial for N seconds")
    parser.add_argument("--ready-marker", default=DEFAULT_READY_MARKER,
                        help="serial substring that means the terminal is ready")
    parser.add_argument("--screenshot", metavar="FILE.ppm",
                        help="grab a framebuffer screenshot (PPM) before shutdown")
    parser.add_argument(
        "--accel",
        choices=["auto", "kvm", "tcg"],
        default="auto",
        help="QEMU acceleration (default auto: kvm if /dev/kvm writable)",
    )
    parser.add_argument("--cpus", type=int, help="explicit guest vCPU count (1..256)")
    parser.add_argument("--net-test", action="store_true",
                        help="attach the local bounded Ethernet test peer and reference E1000")
    parser.add_argument("--net-live", action="store_true",
                        help="attach the isolated userspace ARP/ICMP/UDP/DNS peer and reference E1000")
    parser.add_argument("--net-dhcp", action="store_true",
                        help="verify DHCP leases against an isolated server and reference E1000")
    parser.add_argument("--net-static", action="store_true",
                        help="verify static IPv4, gateway routing and conflict withdrawal over E1000")
    parser.add_argument("--net-socket", action="store_true",
                        help="verify outbound application UDP and TCP over isolated E1000")
    parser.add_argument("--net-dns", action="store_true",
                        help="verify outbound application DNS resolution over isolated E1000")
    parser.add_argument("--net-dns-posix", action="store_true",
                        help="verify C POSIX getaddrinfo DNS resolution over isolated E1000")
    parser.add_argument("--net-listen", action="store_true",
                        help="verify application TCP listen/accept and UDP bind over isolated E1000")
    parser.add_argument("--net-listen-posix", action="store_true",
                        help="verify C POSIX bind/listen/accept over isolated E1000")
    parser.add_argument("--net-reno", action="store_true",
                        help="verify TCP Reno slow start and fast retransmit over isolated E1000")
    parser.add_argument("--qmp-transport", choices=("unix", "stdio"), default="unix",
                        help="QMP control channel; stdio uses local pipes without sockets")
    parser.add_argument(
        "--performance-record",
        type=Path,
        help="write opt-in reproducibility and wall-time JSON",
    )
    parser.add_argument("--mem", default="512M", help="guest RAM (default 512M)")
    parser.add_argument(
        "--workdir", help="where to place qmp socket / serial log (default build/)"
    )
    parser.add_argument(
        "--qemu-arg",
        action="append",
        help="extra raw argument passed to qemu (repeatable)",
    )
    from image_profile import PROFILES

    parser.add_argument(
        "--image-profile",
        choices=PROFILES,
        default="full",
        help="composition for a developer --build ISO; independent of its boot config",
    )
    parser.add_argument("--scratch-disk", type=int, metavar="SIZE_MB",
                        help="attach a blank raw AHCI scratch disk of SIZE_MB MiB before "
                             "booting (for the installer self-test; enumerates as sda on an ISO boot)")
    parser.add_argument("--scratch-bus", choices=["ahci", "virtio"], default="ahci",
                        help="bus for --scratch-disk: ahci (if=ide -> sda) or virtio "
                             "(legacy virtio-blk-pci -> vd0); default ahci")
    parser.add_argument("--uefi", action="store_true",
                        help="boot under UEFI firmware (OVMF pflash) instead of the default "
                             "SeaBIOS; the same image's EFI boot path is exercised")
    parser.add_argument("--json", action="store_true",
                        help="print the result as JSON instead of human text")
    parser.add_argument("--interactive", action="store_true",
                        help="open a real QEMU window for hands-on GUI testing: no keystroke "
                             "injection, no force-quit timeout; runs until you close the window")
    parser.add_argument("--display", default="gtk", choices=["gtk", "sdl"],
                        help="QEMU display backend for --interactive (default gtk)")
    parser.add_argument("--markers-file", metavar="FILE.json",
                        help="markers manifest (required/forbidden serial markers + run defaults); "
                             "run FAILS if any required marker is missing or any forbidden one appears")
    parser.add_argument("--require-marker", action="append", metavar="TEXT",
                        help="serial substring that must appear (repeatable); merged with --markers-file")
    parser.add_argument("--forbid-marker", action="append", metavar="TEXT",
                        help="serial substring that must NOT appear (repeatable); merged with --markers-file")
    args = parser.parse_args()

    if args.image_profile!="full" and (not args.build or args.variant):
        parser.error("--image-profile requires a developer --build (without --variant)")
    if args.initrd_overlay and (not args.build or args.variant):
        parser.error("--initrd-overlay requires --build without a release --variant")
    if args.variant and not args.build:
        parser.error("--variant requires --build; use --iso to test an existing release image")
    if (not args.build and not args.build_disk and not args.iso and not args.disk
            and not (REPO_ROOT / "build" / "microkernel.iso").is_file()):
        parser.error("no image: pass --build, --build-disk, --iso, or --disk")

    result = run(args)
    if args.json:
        print(json.dumps(asdict(result), indent=2, ensure_ascii=False))
    else:
        print_human(result)
    return 0 if result.ok else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("\n[qemu_test] interrupted", file=sys.stderr)
        raise SystemExit(130)
