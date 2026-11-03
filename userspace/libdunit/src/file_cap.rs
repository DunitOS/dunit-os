//! File and directory authority anchored by the kernel, independent of names.
use alloc::vec::Vec;
pub use dunit_file_cap_abi::{Operation, Relation};

pub fn open(path: &str, rights: u32) -> isize {
    crate::syscall3(
        crate::SYSCALL_FILE_CAP_OPEN,
        path.as_ptr() as usize,
        path.len(),
        rights as usize,
    )
}
pub fn stat(handle: u32, out: &mut crate::FileStat) -> isize {
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        handle as usize,
        Operation::Stat as usize,
        out as *mut _ as usize,
        core::mem::size_of::<crate::FileStat>(),
        0,
        0,
    )
}
pub fn read_at(handle: u32, offset: u64, out: &mut [u8]) -> isize {
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        handle as usize,
        Operation::Read as usize,
        out.as_mut_ptr() as usize,
        out.len(),
        offset as usize,
        0,
    )
}
pub fn readdir(handle: u32, out: &mut [crate::DirEntry]) -> isize {
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        handle as usize,
        Operation::ReadDir as usize,
        out.as_mut_ptr() as usize,
        out.len(),
        0,
        0,
    )
}
pub fn open_child(handle: u32, path: &str, rights: u32) -> isize {
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        handle as usize,
        Operation::OpenChild as usize,
        path.as_ptr() as usize,
        path.len(),
        0,
        rights as usize,
    )
}
pub fn remove(handle: u32) -> isize {
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        handle as usize,
        Operation::Remove as usize,
        0,
        0,
        0,
        0,
    )
}
pub fn move_to(handle: u32, destination: u32, leaf: &str) -> isize {
    let mut bytes = Vec::with_capacity(8 + leaf.len());
    bytes.extend_from_slice(&destination.to_le_bytes());
    bytes.extend_from_slice(&(leaf.len() as u32).to_le_bytes());
    bytes.extend_from_slice(leaf.as_bytes());
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        handle as usize,
        Operation::Move as usize,
        bytes.as_ptr() as usize,
        bytes.len(),
        0,
        0,
    )
}
pub fn create_child(handle: u32, leaf: &str, file_type: u32, rights: u32) -> isize {
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        handle as usize,
        Operation::CreateChild as usize,
        leaf.as_ptr() as usize,
        leaf.len(),
        file_type as usize,
        rights as usize,
    )
}
pub fn write_at(handle: u32, offset: u64, data: &[u8]) -> isize {
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        handle as usize,
        Operation::Write as usize,
        data.as_ptr() as usize,
        data.len(),
        offset as usize,
        0,
    )
}

pub fn relation(source: u32, destination: u32) -> isize {
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        source as usize,
        Operation::Relation as usize,
        destination as usize,
        0,
        0,
        0,
    )
}

pub fn sync(handle: u32) -> isize {
    crate::syscall6(
        crate::SYSCALL_FILE_CAP_OP,
        handle as usize,
        Operation::Sync as usize,
        0,
        0,
        0,
        0,
    )
}
