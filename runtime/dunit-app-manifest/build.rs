use std::{env, fs, path::PathBuf};
fn main() {
    let source =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../abi/rights.abi");
    println!("cargo:rerun-if-changed={}", source.display());
    let content = fs::read_to_string(source).unwrap();
    let names: Vec<_> = content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| l.split_whitespace().nth(1).unwrap())
        .collect();
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("rights.rs"),
        format!("const RIGHTS: &[&str] = &{:?};", names),
    )
    .unwrap();
}
