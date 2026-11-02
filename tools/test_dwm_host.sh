#!/usr/bin/env bash
# Pure regressions import the same modules used by the kernel and compositor.
set -euo pipefail
cd "$(dirname "$0")/.."
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
rustc --edition=2021 --crate-type rlib --crate-name dunit_pointer_abi abi/pointer-input/src/lib.rs -o "$test_dir/libdunit_pointer_abi.rlib"
for source in kernel/tests/input_geometry.rs kernel/tests/terminal_stdin.rs \
    kernel/src/drivers/keyboard_queue.rs \
    userspace/system_apps/gui_server/src/lua_policy.rs \
    userspace/system_apps/gui_server/src/shell_geometry.rs \
    userspace/system_apps/gui_settings/src/document_editor.rs \
    userspace/system_apps/gui_terminal/src/terminal_model_tests.rs \
    userspace/system_apps/gui_server/src/composite.rs \
    userspace/system_apps/gui_server/src/damage_plan.rs \
    userspace/system_apps/gui_server/src/cursor.rs \
    userspace/system_apps/gui_server/src/frame_pacing.rs \
    userspace/system_apps/gui_server/src/input_lifecycle.rs \
    userspace/system_apps/gui_server/src/window_effects_tests.rs \
    userspace/system_apps/gui_server/src/animation.rs
do
    echo "Testing $source"
    pointer_deps=()
    if [ "$source" = kernel/tests/input_geometry.rs ]; then
        pointer_deps=(--extern "dunit_pointer_abi=$test_dir/libdunit_pointer_abi.rlib")
    fi
    rustc --edition=2021 --test "$source" "${pointer_deps[@]}" -o "$test_dir/regression"
    "$test_dir/regression"
done

bash tools/test_dwm_config_host.sh

rustc --edition=2021 --crate-type rlib --crate-name term_theme runtime/term_theme/src/lib.rs -o "$test_dir/libterm_theme.rlib"
rustc --edition=2021 --test kernel/tests/terminal_display.rs --extern term_theme="$test_dir/libterm_theme.rlib" -o "$test_dir/terminal-display"
"$test_dir/terminal-display"

rustc --edition=2021 --crate-type rlib --crate-name dunit_style runtime/dunit-style/src/lib.rs -o "$test_dir/libdunit_style.rlib"
rustc --edition=2021 --test userspace/system_apps/gui_server/src/shell_motion_tests.rs --extern dunit_style="$test_dir/libdunit_style.rlib" -o "$test_dir/scene-motion"
"$test_dir/scene-motion"
