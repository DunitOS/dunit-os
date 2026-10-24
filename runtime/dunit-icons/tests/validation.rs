use dunit_icons::{
    archive::{crc32, Archive, ReadAt},
    theme::{strings, Index},
    Error, Result,
};
struct Bytes(Vec<u8>);
impl ReadAt for Bytes {
    fn len(&mut self) -> Result<u64> {
        Ok(self.0.len() as u64)
    }
    fn read_at(&mut self, p: u64, b: &mut [u8]) -> Result<()> {
        b.copy_from_slice(
            self.0
                .get(p as usize..p as usize + b.len())
                .ok_or(Error::Io)?,
        );
        Ok(())
    }
}
fn fixture(name: &str, bytes: &[u8]) -> Vec<u8> {
    let data = miniz_oxide::deflate::compress_to_vec(bytes, 6);
    let crc = crc32(bytes);
    let mut h = vec![0u8; 30];
    h[..4].copy_from_slice(b"PK\x03\x04");
    h[8] = 8;
    h[14..18].copy_from_slice(&crc.to_le_bytes());
    h[18..22].copy_from_slice(&(data.len() as u32).to_le_bytes());
    h[22..26].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    h[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
    h.extend_from_slice(name.as_bytes());
    h.extend_from_slice(&data);
    let offset = h.len() as u32;
    let mut c = vec![0; 46];
    c[..4].copy_from_slice(b"PK\x01\x02");
    c[10] = 8;
    c[16..20].copy_from_slice(&crc.to_le_bytes());
    c[20..24].copy_from_slice(&(data.len() as u32).to_le_bytes());
    c[24..28].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    c[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
    c.extend_from_slice(name.as_bytes());
    let size = c.len() as u32;
    h.extend_from_slice(&c);
    let mut end = vec![0; 22];
    end[..4].copy_from_slice(b"PK\x05\x06");
    end[8] = 1;
    end[10] = 1;
    end[12..16].copy_from_slice(&size.to_le_bytes());
    end[16..20].copy_from_slice(&offset.to_le_bytes());
    h.extend_from_slice(&end);
    h
}
#[test]
fn entry_integrity_and_bounds() {
    let good = fixture("icons/test.svg", b"bounded native SVG");
    let mut a = Archive::open(Bytes(good.clone())).unwrap();
    assert_eq!(
        a.read("icons/test.svg", 100).unwrap(),
        b"bounded native SVG"
    );
    assert_eq!(a.read("icons/test.svg", 4), Err(Error::Limit));
    assert_eq!(a.read("missing", 100), Err(Error::Missing));
    let mut bad = good.clone();
    bad[30 + "icons/test.svg".len()] ^= 0x11;
    let mut a = Archive::open(Bytes(bad)).unwrap();
    assert!(a.read("icons/test.svg", 100).is_err());
    for n in 0..good.len() {
        assert!(Archive::open(Bytes(good[..n].to_vec())).is_err());
    }
    for name in [
        "../escape.svg",
        "/absolute.svg",
        "icons/../escape.svg",
        "icons\\escape.svg",
    ] {
        assert!(Archive::open(Bytes(fixture(name, b"x"))).is_err());
    }
}
#[test]
fn manifest_is_atomic_and_directory_distance_uses_scale() {
    assert!(strings("[theme]\nroot = \"a\"\nroot = \"b\"").is_err());
    assert!(strings("[theme]\nroot = unquoted").is_err());
    let i=Index::parse("[Icon Theme]\nDirectories=apps/16,apps/32,apps/64\nScaledDirectories=apps/16@2\n[apps/16]\nSize=16\nType=Fixed\n[apps/32]\nSize=32\nType=Fixed\n[apps/64]\nSize=64\nType=Fixed\n[apps/16@2]\nSize=16\nScale=2\nType=Fixed").unwrap();
    assert_eq!(
        i.candidates(31),
        ["apps/32", "apps/16@2", "apps/16", "apps/64"]
    );
    assert!(Index::parse("[Icon Theme]\nDirectories=../escape\n").is_err());
}
