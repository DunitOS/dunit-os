//! Exercises the real VFS, MemFS and open-description capabilities. Only the
//! platform interrupt guard, initrd source and serial sink are replaced.
extern crate alloc;
mod sync {
    pub struct InterruptGuard;
    impl InterruptGuard {
        pub fn new() -> Self {
            Self
        }
    }
}
mod serial {
    pub fn serial_write(_: &str) {}
}
mod initrd {
    pub struct Entry {
        pub name: alloc::string::String,
        pub data: &'static [u8],
    }
    pub struct Archive;
    impl Archive {
        pub fn entries(&self) -> alloc::vec::IntoIter<Entry> {
            alloc::vec::Vec::new().into_iter()
        }
    }
    pub fn get_initrd() -> Option<Archive> {
        None
    }
}
#[path = "../kernel/src/fs/file_cap.rs"]
pub mod file_cap;
#[path = "../kernel/src/fs/memfs.rs"]
pub mod memfs;
#[path = "../kernel/src/fs/vfs.rs"]
pub mod vfs;
mod fs {
    pub use crate::memfs;
}
use vfs::{FileSystem, FileType, OpenFlags as F, VfsError as E};
// The real kernel is UP; its VFS resolve scratch buffer is shared.
static KERNEL: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn tree() -> vfs::VirtualFileSystem {
    let mut backend = Box::new(memfs::MemFs::new());
    backend.mkdir("/tmp/source").unwrap();
    backend.mkdir("/tmp/dest").unwrap();
    backend.add_file("/tmp/source/old", b"original".to_vec());
    let mut vfs = vfs::VirtualFileSystem::new();
    vfs.mount("/", Box::leak(backend)).unwrap();
    vfs
}
#[test]
fn moved_directory_and_child_keep_identity_without_ambient_path_authority() {
    let _up = KERNEL.lock().unwrap();
    let mut v = tree();
    let source = v
        .open_capability("/", "/tmp/source", F::READ_WRITE)
        .unwrap();
    let child = source.child("old", F::READ).unwrap();
    let dest = v.open_capability("/", "/tmp/dest", F::READ_WRITE).unwrap();
    assert_eq!(
        source.relation(&dest),
        Ok(dunit_file_cap_abi::Relation::Unrelated)
    );
    assert_eq!(
        source.relation(&child),
        Ok(dunit_file_cap_abi::Relation::SourceAncestor)
    );
    source.move_to(&dest, "moved").unwrap();
    assert_eq!(
        source.relation(&dest),
        Ok(dunit_file_cap_abi::Relation::DestinationAncestor)
    );
    let mut bytes = [0; 32];
    let n = child.read_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes[..n], b"original");
    assert_eq!(source.readdir().unwrap()[0].name(), "old");
    assert_eq!(
        source.child("old", F::READ).unwrap().stat().unwrap().size,
        8
    );
    assert_eq!(
        v.open_capability("/", "/tmp/source", F::READ).err(),
        Some(E::NotFound)
    );
    assert_eq!(source.move_to(&source, "cycle"), Err(E::InvalidPath));
}
#[test]
fn unlinked_file_is_not_rebound_when_the_name_is_reused() {
    let _up = KERNEL.lock().unwrap();
    let mut v = tree();
    let dir = v
        .open_capability("/", "/tmp/source", F::READ_WRITE)
        .unwrap();
    let old = dir.child("old", F::READ_WRITE).unwrap();
    old.remove().unwrap();
    let new = dir
        .create_child("old", FileType::File, F::READ_WRITE)
        .unwrap();
    new.write_at(0, b"new").unwrap();
    let mut bytes = [0; 32];
    let n = old.read_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes[..n], b"original");
    assert_eq!(old.remove(), Err(E::NotFound));
    let n = new.read_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes[..n], b"new");
}
#[test]
fn scoped_children_rights_and_no_overwrite_or_cross_backend_move() {
    let _up = KERNEL.lock().unwrap();
    let mut v = tree();
    let readonly = v.open_capability("/", "/tmp/source", F::READ).unwrap();
    assert_eq!(
        readonly
            .create_child("x", FileType::File, F::READ_WRITE)
            .err(),
        Some(E::PermissionDenied)
    );
    let child = readonly.child("old", F::READ).unwrap();
    assert_eq!(child.write_at(0, b"bad"), Err(E::PermissionDenied));
    for path in [
        "../dest",
        "/tmp/dest",
        "a/../old",
        "./old",
        "a//b",
        "",
        "a\0b",
    ] {
        assert_eq!(readonly.child(path, F::READ).err(), Some(E::InvalidPath));
    }
    let dir = v
        .open_capability("/", "/tmp/source", F::READ_WRITE)
        .unwrap();
    let writable = dir.child("old", F::READ_WRITE).unwrap();
    assert_eq!(writable.move_to(&dir, "old"), Ok(()));
    dir.create_child("occupied", FileType::File, F::READ_WRITE)
        .unwrap();
    assert_eq!(writable.move_to(&dir, "occupied"), Err(E::AlreadyExists));
    let mut another = tree();
    let other = another
        .open_capability("/", "/tmp/dest", F::READ_WRITE)
        .unwrap();
    assert_eq!(writable.move_to(&other, "x"), Err(E::Unsupported));
    assert_eq!(
        writable.relation(&other),
        Ok(dunit_file_cap_abi::Relation::DifferentBackend)
    );
    assert_eq!(dir.read_at(0, &mut [0]), Err(E::IsADirectory));
}
#[test]
fn directory_enumeration_is_complete_and_does_not_truncate_long_names() {
    let _up = KERNEL.lock().unwrap();
    let mut v = tree();
    let dir = v.open_capability("/", "/tmp/dest", F::READ_WRITE).unwrap();
    for i in 0..100 {
        dir.create_child(&format!("item-{i}"), FileType::File, F::READ_WRITE)
            .unwrap();
    }
    assert_eq!(dir.readdir().unwrap().len(), 100);
    dir.create_child(&"x".repeat(65), FileType::File, F::READ_WRITE)
        .unwrap();
    assert_eq!(dir.readdir(), Err(E::FileTooBig));
}

#[test]
fn ancestry_uses_inode_locations_and_component_boundaries_after_rename() {
    let _up = KERNEL.lock().unwrap();
    let mut v = tree();
    let root = v.open_capability("/", "/tmp", F::READ_WRITE).unwrap();
    let a = root
        .create_child("a", FileType::Directory, F::READ_WRITE)
        .unwrap();
    let ab = root
        .create_child("ab", FileType::Directory, F::READ_WRITE)
        .unwrap();
    assert_eq!(a.relation(&ab), Ok(dunit_file_cap_abi::Relation::Unrelated));
    let child = a
        .create_child("nested", FileType::Directory, F::READ_WRITE)
        .unwrap();
    assert_eq!(
        a.relation(&child),
        Ok(dunit_file_cap_abi::Relation::SourceAncestor)
    );
    child.move_to(&ab, "moved").unwrap();
    assert_eq!(
        a.relation(&child),
        Ok(dunit_file_cap_abi::Relation::Unrelated)
    );
    assert_eq!(
        ab.relation(&child),
        Ok(dunit_file_cap_abi::Relation::SourceAncestor)
    );
}
