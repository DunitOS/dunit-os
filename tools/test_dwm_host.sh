#!/usr/bin/env bash
# Pure regressions import the same modules used by the kernel and compositor.
set -euo pipefail
cd "$(dirname "$0")/.."
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
for source in kernel/tests/input_geometry.rs kernel/tests/terminal_stdin.rs \
    userspace/system_apps/gui_server/src/lua_policy.rs \
    userspace/system_apps/gui_server/src/shell_geometry.rs \
    userspace/system_apps/gui_settings/src/document_editor.rs \
    userspace/system_apps/gui_server/src/composite.rs \
    userspace/system_apps/gui_server/src/damage_plan.rs \
    userspace/system_apps/gui_server/src/window_effects_tests.rs \
    userspace/system_apps/gui_server/src/animation.rs
do
    echo "Testing $source"
    rustc --edition=2021 --test "$source" -o "$test_dir/regression"
    "$test_dir/regression"
done

bash tools/test_dwm_config_host.sh

rustc --edition=2021 --crate-type rlib --crate-name dunit_style runtime/dunit-style/src/lib.rs -o "$test_dir/libdunit_style.rlib"
rustc --edition=2021 --test userspace/system_apps/gui_server/src/shell_motion_tests.rs --extern dunit_style="$test_dir/libdunit_style.rlib" -o "$test_dir/scene-motion"
"$test_dir/scene-motion"
