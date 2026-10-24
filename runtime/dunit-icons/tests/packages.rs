use dunit_icons::{
    archive::{Archive, ReadAt},
    theme::Index,
    Error, Result,
};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};
struct Input(File);
impl ReadAt for Input {
    fn len(&mut self) -> Result<u64> {
        self.0.metadata().map(|m| m.len()).map_err(|_| Error::Io)
    }
    fn read_at(&mut self, p: u64, b: &mut [u8]) -> Result<()> {
        self.0
            .seek(SeekFrom::Start(p))
            .and_then(|_| self.0.read_exact(b))
            .map_err(|_| Error::Io)
    }
}
#[test]
fn packaged_icons_render_at_requested_sizes() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
    let aliases = std::fs::read_to_string(base.join("chameleon/aliases.toml")).unwrap();
    let aliases = dunit_icons::theme::strings(&aliases).unwrap();
    let mut failures = Vec::new();
    for id in [
        "breeze-chameleon-dark",
        "breeze-chameleon-light",
        "breeze-round-chameleon-dark",
        "breeze-round-chameleon-light",
        "chameleon-symbolic-dark",
    ] {
        let m = std::fs::read_to_string(base.join(id).join("theme.toml")).unwrap();
        let fields = dunit_icons::theme::strings(&m).unwrap();
        let root = &fields.iter().find(|(_, k, _)| k == "root").unwrap().2;
        let path = base.join(format!("chameleon/{id}.zip"));
        let mut zip = Archive::open(Input(File::open(path).unwrap())).unwrap();
        let names: Vec<_> = zip.names().map(str::to_string).collect();
        assert!(names.len() > 60);
        let bytes = zip.read(&format!("{root}/index.theme"), 65536).unwrap();
        let index = Index::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
        for (_, semantic, name) in &aliases {
            for side in [24, 32, 74, 256] {
                let path = index
                    .candidates(side)
                    .iter()
                    .map(|d| format!("{root}/{d}/{name}.svg"))
                    .find(|p| zip.contains(p));
                let Some(path) = path else {
                    assert_eq!(id, "chameleon-symbolic-dark", "{id} {name}");
                    continue;
                };
                let bytes = zip.read(&path, 1024 * 1024).unwrap();
                match dunit_svg::render(std::str::from_utf8(&bytes).unwrap(), side, side) {
                    Ok(p) => {
                        assert_eq!(p.len(), (side * side) as usize);
                        assert!(p.iter().any(|p| p >> 24 != 0), "empty {path}");
                    }
                    Err(e) => failures.push(format!("{id} {semantic} {side} {path} {e:?}")),
                }
            }
        }
        // Every compressed entry is accessible by its exact original path.
        for name in names {
            let cap = 64 * 1024 * 1024;
            zip.read(&name, cap)
                .unwrap_or_else(|e| panic!("{name} {e:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
