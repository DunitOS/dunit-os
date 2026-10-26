"""Native Dunit SDK workflow, using Cargo artifacts and the system image builder."""
import argparse
import fcntl
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from dunit_manifest import load_manifest, parse_manifest, installed_manifest

ROOT = Path(__file__).resolve().parents[1]
FLAGS = ["--release", "--target", str(ROOT / "userspace/x86_64-unknown-none.json"),
         "-Z", "build-std=core,alloc", "-Z", "build-std-features=compiler-builtins-mem",
         "-Z", "json-target-spec"]
TEST_ATTRIBUTES = '''#![no_std]
#![no_main]
#![cfg_attr(test, feature(custom_test_frameworks))]
#![cfg_attr(test, test_runner(dunit::testing::runner))]
#![cfg_attr(test, reexport_test_harness_main = "__dunit_test_main")]
'''


def cargo():
    configured = os.environ.get("CARGO")
    rustup = Path.home() / ".cargo/bin/cargo"
    return configured or (str(rustup) if rustup.is_file() else "cargo")


def checked(command, **kwargs):
    return subprocess.run(command, check=True, **kwargs)


def project(path):
    manifest = path.expanduser().resolve()
    if manifest.is_dir():
        manifest /= "Cargo.toml"
    result = checked([cargo(), "metadata", "--format-version=1", "--no-deps", "--manifest-path", str(manifest)],
                     capture_output=True, text=True)
    packages = json.loads(result.stdout)["packages"]
    package = next((p for p in packages if Path(p["manifest_path"]).resolve() == manifest), None)
    if package is None:
        raise ValueError(f"{manifest}: select an application package, not a virtual workspace")
    bins = [target["name"] for target in package["targets"] if "bin" in target["kind"]]
    app = load_manifest(manifest.with_name("dunit.toml"), bins, ROOT / "abi/rights.abi")
    return manifest, package, app


def build(manifest, package, app, tests=False):
    command = [cargo(), "test" if tests else "build", "--manifest-path", str(manifest),
               "--bin", app.binary, *FLAGS, "--message-format=json"]
    if tests:
        command += ["--no-run"]
    process = subprocess.Popen(command, cwd=manifest.parent, stdout=subprocess.PIPE, text=True)
    artifacts = []
    for line in process.stdout:
        message = json.loads(line)
        if message.get("reason") == "compiler-message":
            rendered = message["message"].get("rendered")
            if rendered:
                print(rendered, file=sys.stderr, end="")
        if (message.get("reason") == "compiler-artifact" and message.get("package_id") == package["id"]
                and message["target"]["name"] == app.binary and message["profile"]["test"] == tests
                and message.get("executable")):
            artifacts.append(Path(message["executable"]))
    if process.wait():
        raise subprocess.CalledProcessError(process.returncode, command)
    if len(artifacts) != 1 or not artifacts[0].is_file():
        raise RuntimeError("Cargo did not report exactly one executable for the selected target")
    print(f"[dunit] {'test' if tests else 'application'} ELF: {artifacts[0]}", flush=True)
    return artifacts[0]


def create(args):
    import re
    destination = args.path.expanduser().absolute()
    identifier = args.id or destination.name.replace('-', '_')
    if not re.fullmatch(r'[a-z][a-z0-9_]{0,63}', identifier):
        raise ValueError("new: --id must be a lowercase Rust/Cargo identifier, 1..64 bytes")
    manifest_text = (
        f'schema_version = 1\n\n[application]\nid = {json.dumps(identifier)}\n'
        f'name = {json.dumps(args.name or identifier, ensure_ascii=False)}\nkind = "{args.kind}"\n'
        f'binary = {json.dumps(identifier)}\ncapabilities = []\n')
    if args.kind == "gui":
        manifest_text += 'icon = "/assets/icons/monitor.rgba"\n'
    parse_manifest(manifest_text, destination / "dunit.toml", [identifier], ROOT / "abi/rights.abi")
    # mkdir is intentionally exclusive: never overwrite an existing project.
    destination.mkdir(parents=True)
    (destination / "src").mkdir()
    (destination / "Cargo.toml").write_text(
        f'[package]\nname = {json.dumps(identifier)}\nversion = "0.1.0"\nedition = "2021"\n\n'
        f'[dependencies]\ndunit = {{ path = {json.dumps(str(ROOT / "runtime/dunit"))}'
        + (', features = ["gui"]' if args.kind == "gui" else '') + ' }\n'
        + (f'dwm_settings = {{ path = {json.dumps(str(ROOT / "userspace/dwm_settings"))} }}\n' if args.kind == "gui" else ''))
    (destination / "dunit.toml").write_text(manifest_text, encoding="utf-8")
    source = TEST_ATTRIBUTES + '''
#[dunit::main]
fn main() -> dunit::Result<()> {
    dunit::println!("[SDKAPP] ok")?;
    Ok(())
}

#[dunit::test]
fn native_file_roundtrip() -> dunit::Result<()> {
    let path = "sdk-example.txt";
    dunit::fs::write(path, b"Hello, Dunit!")?;
    assert_eq!(dunit::fs::read(path)?, b"Hello, Dunit!");
    dunit::fs::remove_file(path)?;
    let manifest = dunit::fs::read_to_string("/system/share/dunit/apps/__APP_ID__/dunit.toml")?;
    assert!(manifest.contains("schema_version = 1"));
    Ok(())
}
'''.replace("__APP_ID__", identifier)
    if args.kind == "gui":
        template = (ROOT / "tools/templates/gui.rs").read_text()
        source = TEST_ATTRIBUTES + template
    (destination / "src/main.rs").write_text(source, encoding="utf-8")
    print(f"[dunit] created {destination}")


def boot(args, app, artifact, tests):
    if app.kind == "gui" and (args.args or getattr(args, "input", []) or getattr(args, "input_ready", None)):
        raise ValueError("GUI launch currently accepts no guest arguments or terminal input")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        raise ValueError("--timeout must be finite and positive")
    for argument in args.args:
        if not argument or any(c.isspace() or ord(c) < 32 or ord(c) > 126 for c in argument):
            raise ValueError("guest arguments must be nonempty ASCII words: the recovery console has no quoting syntax")
    build_dir = ROOT / "build"
    build_dir.mkdir(exist_ok=True)
    # The repository has one shared image directory. Prevent overlapping SDK builds.
    with (build_dir / "cargo-dunit.lock").open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        with tempfile.TemporaryDirectory(prefix="sdk-", dir=build_dir) as temporary:
            staging = Path(temporary)
            overlay = staging / "overlay"
            executable = overlay / "app" / app.binary
            executable.parent.mkdir(parents=True)
            shutil.copyfile(artifact, executable)
            metadata = overlay / "system/share/dunit/apps" / app.id / "dunit.toml"
            metadata.parent.mkdir(parents=True)
            metadata.write_text(installed_manifest(app), encoding="utf-8")
            required = ["[TERM-007] Starting main loop", f"[EXEC] /app/{app.binary} returned code=0"]
            if tests:
                required.append("[SDKTEST] OK count=")
            markers = {"required": required, "forbidden": ["[PANIC]", "[SDK] PANIC", "[SDKTEST] FAIL"]}
            guest_command = " ".join(["exec", f"/app/{app.binary}", *args.args])
            inputs = getattr(args, "input", [])
            if inputs:
                if any(any(ord(c) < 32 or ord(c) > 126 for c in line) for line in inputs):
                    raise ValueError("--input lines must contain printable ASCII")
                ready = getattr(args, "input_ready", None)
                steps = ([{"wait_marker": ready}] if ready else [{"delay_ms": 500}])
                steps += [{"type_text": line} for line in inputs]
                markers["run"] = {"command_actions": {guest_command: steps}}
            if app.kind == "gui":
                # Only this installed app is autostarted; compositor owns launch.
                config = overlay / "sdk-desktop.toml"
                config.write_text(
                    '[meta]\nschema_version = 2\n[dock]\nentries = [' + json.dumps(app.id) + ']\n'
                    '[launcher]\nentries = [' + json.dumps(app.id) + ']\n'
                    '[startup]\nentries = [' + json.dumps(app.id) + ']\nself_test = false\n'
                    '[shortcuts]\ncmd_mod = "super"\nwin_close = true\n')
                exit_marker = f"[GUI-EXIT] app={app.id} kind=0 code=0"
                presented = f"[GUI-PRESENT] app={app.id}"
                markers["required"] = ["[TERM-007] Starting main loop", "gui_server: desktop ready", exit_marker]
                steps = []
                if tests:
                    markers["required"].append("[SDKTEST] OK count=")
                else:
                    markers["required"].append(presented)
                    steps = [{"wait_marker": presented, "timeout": args.timeout},
                             {"wait_marker": "gui_server: desktop ready", "timeout": args.timeout},
                             {"delay_ms": 2000},
                             {"execute": "send-key", "arguments": {"keys": [
                                 {"type": "qcode", "data": "meta_l"}, {"type": "qcode", "data": "q"}], "hold-time": 80}}]
                steps.append({"wait_marker": f"[GUI-EXIT] app={app.id} ", "timeout": args.timeout})
                markers["run"] = {"command_actions": {"exec /app/gui_server": steps}}
                markers["forbidden"] += ["gui_server: FAIL", "gui_server: PANIC"]
            marker_path = staging / "markers.json"
            marker_path.write_text(json.dumps(markers))
            checked([cargo(), "build", "--release", "--features", "boot-smoke-tests",
                     "-Z", "build-std=core,alloc,compiler_builtins",
                     "-Z", "build-std-features=compiler-builtins-mem", "-Z", "json-target-spec"], cwd=ROOT / "kernel")
            command = [sys.executable, str(ROOT / "tools/qemu_test.py"), "--build",
                       "--initrd-overlay", str(overlay), "--markers-file", str(marker_path),
                       "--accel", args.accel, "--timeout", str(args.timeout),
                       "--boot-timeout", str(args.timeout), "--cmd-timeout", str(args.timeout),
                       "--post-boot-delay", "3", "--settle", "4",
                       "--cmd", guest_command]
            if app.kind == "gui":
                command[-2:] = ["--cmd", "exec /app/setup import-desktop /sdk-desktop.toml", "--cmd", "exec /app/gui_server"]
            checked(command, cwd=ROOT)


def main(argv=None):
    parser = argparse.ArgumentParser(prog="cargo dunit", description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    new = commands.add_parser("new", help="create a native CLI/TUI/GUI application")
    new.add_argument("path", type=Path)
    new.add_argument("--id")
    new.add_argument("--name")
    new.add_argument("--kind", choices=("cli", "tui", "gui"), default="cli")
    for name in ("build", "run", "test"):
        command = commands.add_parser(name)
        command.add_argument("--manifest-path", type=Path, default=Path("Cargo.toml"))
        if name != "build":
            command.add_argument("--accel", choices=("auto", "kvm", "tcg"), default="auto")
            command.add_argument("--timeout", type=float, default=240)
            command.add_argument("args", nargs="*", help="guest arguments (after --)")
            if name == "run":
                command.add_argument("--input", action="append", default=[], help="type a line into the guest, repeatable")
                command.add_argument("--input-ready", help="wait for an application prompt before typing")
    arguments = list(sys.argv[1:] if argv is None else argv)
    if arguments[:1] == ["dunit"]:
        arguments.pop(0)
    args = parser.parse_args(arguments)
    os.environ["RUSTUP_TOOLCHAIN"] = "nightly"
    # Make's cargo and the subprocesses must use the same rustup toolchain.
    os.environ["PATH"] = str(Path(cargo()).parent) + os.pathsep + os.environ["PATH"]
    try:
        if args.command == "new":
            create(args)
        else:
            manifest, package, app = project(args.manifest_path)
            artifact = build(manifest, package, app, args.command == "test")
            if args.command != "build":
                boot(args, app, artifact, args.command == "test")
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"[dunit] {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
