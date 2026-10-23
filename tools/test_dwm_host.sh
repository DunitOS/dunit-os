#!/usr/bin/env bash
# Pure regressions import the same modules used by the kernel and compositor.
set -euo pipefail
cd "$(dirname "$0")/.."
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
for source in kernel/tests/input_geometry.rs kernel/tests/terminal_stdin.rs \
    userspace/dwm_settings/tests/preserve.rs \
    userspace/system_apps/gui_server/src/lua_policy.rs \
    userspace/system_apps/gui_server/src/animation.rs
do
    echo "Testing $source"
    rustc --edition=2021 --test "$source" -o "$test_dir/regression"
    "$test_dir/regression"
done
