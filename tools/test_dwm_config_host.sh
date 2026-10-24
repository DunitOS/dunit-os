#!/usr/bin/env bash
# Compile the actual no_std settings library for host-only pure validation.
# Syscall test doubles panic: the validated paths must perform no OS I/O.
set -euo pipefail
cd "$(dirname "$0")/.."
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
cat > "$test_dir/libdunit.rs" <<'RS'
pub const ENOENT:isize=-2;
pub const EFBIG:isize=-27;
pub const EINVAL:isize=-22;
pub const OPEN_READ:u32=1;
pub fn open(_: &str,_:u32)->isize {panic!("unexpected I/O")}
pub fn read(_:usize,_:&mut[u8])->isize {panic!("unexpected I/O")}
pub fn close(_:usize)->isize {panic!("unexpected I/O")}
pub fn write_string(_: &str,_:&str)->Result<(),isize> {panic!("unexpected I/O")}
pub fn println(_: &str) {panic!("unexpected I/O")}
RS
rustc --edition=2021 --crate-type rlib --crate-name libdunit "$test_dir/libdunit.rs" -o "$test_dir/liblibdunit.rlib"
rustc --edition=2021 --crate-type rlib --crate-name term_theme runtime/term_theme/src/lib.rs -o "$test_dir/libterm_theme.rlib"
rustc --edition=2021 --crate-type rlib --crate-name dwm_settings userspace/dwm_settings/src/lib.rs \
    --extern libdunit="$test_dir/liblibdunit.rlib" --extern term_theme="$test_dir/libterm_theme.rlib" \
    -L dependency="$test_dir" -o "$test_dir/libdwm_settings.rlib"
DUNIT_ROOT="$PWD" rustc --edition=2021 --test userspace/dwm_settings/tests/validation.rs \
    --extern dwm_settings="$test_dir/libdwm_settings.rlib" -L dependency="$test_dir" -o "$test_dir/validation"
"$test_dir/validation"
