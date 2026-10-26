#!/usr/bin/env bash
# Compile the actual no_std settings library for host-only pure validation.
# The installed catalog is absent; all other OS I/O panics, especially writes.
set -euo pipefail
cd "$(dirname "$0")/.."
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
host_target=$(rustc -vV | sed -n 's/^host: //p')
cargo build --manifest-path userspace/dwm_settings/Cargo.toml --target "$host_target" \
    --target-dir "$test_dir/deps" --message-format=json > "$test_dir/artifacts.json"
# Cargo's artifact layout differs between stable and nightly; use its reported
# filenames rather than assuming debug/deps (nightly may use build/*/*/out).
mapfile -t artifacts < <(python3 - "$test_dir/artifacts.json" <<'PY'
import json, sys
from pathlib import Path
records = [json.loads(line) for line in open(sys.argv[1])]
libraries = [r for r in records if r.get('reason') == 'compiler-artifact']
for name in ('libdunit', 'dunit_app_manifest'):
    print(next(p for r in libraries if r['target']['name'] == name
               for p in r['filenames'] if p.endswith('.rlib')))
print('\n'.join(sorted({str(Path(p).parent) for r in libraries for p in r['filenames']})))
PY
)
links=()
for directory in "${artifacts[@]:2}"; do links+=(-L "dependency=$directory"); done
cat > "$test_dir/libdunit.rs" <<'RS'
pub use native::{DirEntry, FileStat, FILE_TYPE_FILE, FILE_TYPE_DIRECTORY,
    ENOENT, EFBIG, EINVAL, OPEN_READ,
    KEYMOD_SHIFT, KEYMOD_CTRL, KEYMOD_ALT, KEYMOD_SUPER};
pub fn mkdir(_: &str)->isize {panic!("unexpected I/O")}
pub fn open(_: &str,_:usize)->isize {panic!("unexpected I/O")}
pub fn read(_:usize,_:&mut[u8])->isize {panic!("unexpected I/O")}
pub fn close(_:usize)->isize {panic!("unexpected I/O")}
pub fn write_string(_: &str,_:&str)->Result<(),isize> {panic!("unexpected I/O")}
pub fn println(_: &str) {panic!("unexpected I/O")}
pub fn readdir(path: &str,_:&mut[DirEntry])->isize {
    assert_eq!(path,"/system/share/dunit/apps");
    ENOENT
}
pub fn stat(_: &str,_:&mut FileStat)->isize {panic!("unexpected I/O")}
RS
rustc --edition=2021 --crate-type rlib --crate-name libdunit "$test_dir/libdunit.rs" \
    --extern native="${artifacts[0]}" "${links[@]}" -o "$test_dir/liblibdunit.rlib"
rustc --edition=2021 --crate-type rlib --crate-name term_theme runtime/term_theme/src/lib.rs -o "$test_dir/libterm_theme.rlib"
rustc --edition=2021 --crate-type rlib --crate-name dwm_settings userspace/dwm_settings/src/lib.rs \
    --extern libdunit="$test_dir/liblibdunit.rlib" --extern term_theme="$test_dir/libterm_theme.rlib" \
    --extern dunit_app_manifest="${artifacts[1]}" \
    -L dependency="$test_dir" "${links[@]}" -o "$test_dir/libdwm_settings.rlib"
DUNIT_ROOT="$PWD" rustc --edition=2021 --test --crate-name dwm_settings userspace/dwm_settings/src/lib.rs \
    --extern libdunit="$test_dir/liblibdunit.rlib" --extern term_theme="$test_dir/libterm_theme.rlib" \
    --extern dunit_app_manifest="${artifacts[1]}" -L dependency="$test_dir" \
    "${links[@]}" -o "$test_dir/unit-tests"
"$test_dir/unit-tests"
for source in userspace/dwm_settings/tests/*.rs; do
    DUNIT_ROOT="$PWD" rustc --edition=2021 --test "$source" \
        --extern dwm_settings="$test_dir/libdwm_settings.rlib" \
        --extern dunit_app_manifest="${artifacts[1]}" -L dependency="$test_dir" \
        "${links[@]}" -o "$test_dir/integration-tests"
    "$test_dir/integration-tests"
done
