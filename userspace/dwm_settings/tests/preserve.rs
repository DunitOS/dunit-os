//! Run without the OS linker: rustc --edition=2021 --test this_file.rs.
#[path = "../src/preserve.rs"]
mod preserve;

#[test]
fn settings_rewrite_preserves_shell_tree_and_styling_verbatim() {
    let shell = "[shell]\nstylesheet = \"/assets/dwm/custom.dss\"\n[panels.bottom]\nedge = \"bottom\"\nchildren = [\"apps\", \"clock\"]\n[components.apps]\nkind = \"app_group\"\nentries = [\"editor\", \"files\"]\n[components.clock]\nkind = \"clock\"\nformat = \"%H:%M\"\n";
    let old = format!("[theme]\ncolor = 1\n{shell}");
    let merged = preserve::merge("[theme]\ncolor = 2\n", &old);
    assert!(merged.ends_with(shell));
    assert_eq!(merged.matches("[panels.bottom]").count(), 1);
    assert_eq!(merged.matches("[components.apps]").count(), 1);
}
