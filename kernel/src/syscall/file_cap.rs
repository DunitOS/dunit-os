//! Typed filesystem capability operations; authority is never recovered from paths.
use super::*;
use crate::handle::{RIGHT_READ, RIGHT_TRANSFER, RIGHT_WRITE};
use dunit_file_cap_abi::Operation;
const RIGHTS: u32 = RIGHT_READ | RIGHT_WRITE | RIGHT_TRANSFER;
fn flags(rights: u64) -> Result<crate::fs::vfs::OpenFlags, i64> {
    if rights > u32::MAX as u64
        || rights as u32 & !RIGHTS != 0
        || rights as u32 & (RIGHT_READ | RIGHT_WRITE) == 0
    {
        return Err(EINVAL);
    }
    Ok(crate::fs::vfs::OpenFlags::from_bits(
        rights as u32 & (RIGHT_READ | RIGHT_WRITE),
    ))
}
fn owned(
    handle: u64,
    rights: u32,
) -> Result<alloc::sync::Arc<crate::fs::file_cap::FileCapability>, i64> {
    if handle > u32::MAX as u64 {
        return Err(EBADF);
    }
    crate::process::with_current_process(|process| process.handle_file(handle as u32, rights))
        .ok_or(EINVAL)?
        .map_err(handle_error_to_errno)
}
pub(super) fn open(path: u64, len: u64, rights: u64) -> i64 {
    let _irq = crate::sync::InterruptGuard::new();
    let flags = match flags(rights) {
        Ok(f) => f,
        Err(e) => return e,
    };
    let path = match copy_string_from_user_len(path as *const u8, len as usize, MAX_USER_PATH) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let cwd = match crate::process::with_current_process(|p| p.cwd.clone()) {
        Some(value) => value,
        None => return EINVAL,
    };
    let file = match crate::fs::vfs::get_vfs().ok_or(EIO).and_then(|mut v| {
        v.open_capability(&cwd, &path, flags)
            .map_err(vfs_error_to_errno)
    }) {
        Ok(f) => f,
        Err(e) => return e,
    };
    crate::process::with_current_process_mut(|p| p.handle_create_file(file, rights as u32) as i64)
        .unwrap_or(EINVAL)
}
pub(super) fn operation(handle: u64, op: u64, ptr: u64, len: u64, offset: u64, rights: u64) -> i64 {
    let _irq = crate::sync::InterruptGuard::new();
    let Some(op) = Operation::from_raw(op) else {
        return EINVAL;
    };
    let result = match op {
        Operation::Stat => stat(handle, ptr, len, offset, rights),
        Operation::Read => read(handle, ptr, len, offset, rights),
        Operation::ReadDir => readdir(handle, ptr, len, offset, rights),
        Operation::OpenChild => child(handle, ptr, len, offset, rights, false),
        Operation::Remove => {
            if ptr | len | offset | rights != 0 {
                return EINVAL;
            }
            owned(handle, RIGHT_WRITE)
                .and_then(|f| f.remove().map_err(vfs_error_to_errno))
                .map(|_| 0)
        }
        Operation::Move => move_to(handle, ptr, len, offset, rights),
        Operation::CreateChild => child(handle, ptr, len, offset, rights, true),
        Operation::Write => write(handle, ptr, len, offset, rights),
        Operation::Sync => {
            if ptr | len | offset | rights != 0 {
                return EINVAL;
            }
            owned(handle, RIGHT_WRITE)
                .and_then(|file| file.sync().map_err(vfs_error_to_errno))
                .map(|_| 0)
        }
        Operation::Relation => {
            if len | offset | rights != 0 {
                return EINVAL;
            }
            owned(handle, RIGHT_READ).and_then(|source| {
                owned(ptr, RIGHT_READ).and_then(|dest| {
                    source
                        .relation(dest.as_ref())
                        .map(|r| r as u32 as i64)
                        .map_err(vfs_error_to_errno)
                })
            })
        }
    };
    result.unwrap_or_else(|error| error)
}
fn stat(handle: u64, ptr: u64, len: u64, offset: u64, rights: u64) -> Result<i64, i64> {
    if len != core::mem::size_of::<UserFileStat>() as u64 || offset | rights != 0 {
        return Err(EINVAL);
    }
    validate_user_mapping(ptr, len as usize, true)?;
    let s = owned(handle, RIGHT_READ)?
        .stat()
        .map_err(vfs_error_to_errno)?;
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(&user_file_type(s.file_type).to_le_bytes());
    bytes[8..16].copy_from_slice(&(s.size as u64).to_le_bytes());
    copy_buffer_to_user(ptr as *mut u8, &bytes)?;
    Ok(0)
}
fn read(handle: u64, ptr: u64, len: u64, offset: u64, rights: u64) -> Result<i64, i64> {
    if rights != 0 || len > MAX_USER_COPY as u64 {
        return Err(EINVAL);
    }
    let file = owned(handle, RIGHT_READ)?;
    if len == 0 {
        return Ok(0);
    }
    validate_user_mapping(ptr, len as usize, true)?;
    let mut bytes = alloc::vec![0;len as usize];
    let n = file
        .read_at(offset, &mut bytes)
        .map_err(vfs_error_to_errno)?;
    copy_buffer_to_user(ptr as *mut u8, &bytes[..n])?;
    Ok(n as i64)
}
fn write(handle: u64, ptr: u64, len: u64, offset: u64, rights: u64) -> Result<i64, i64> {
    if rights != 0 || len > MAX_USER_COPY as u64 {
        return Err(EINVAL);
    }
    let file = owned(handle, RIGHT_WRITE)?;
    if len == 0 {
        return Ok(0);
    }
    let bytes = copy_buffer_from_user(ptr as *const u8, len as usize)?;
    file.write_at(offset, &bytes)
        .map(|n| n as i64)
        .map_err(vfs_error_to_errno)
}
fn readdir(handle: u64, ptr: u64, capacity: u64, offset: u64, rights: u64) -> Result<i64, i64> {
    if capacity > MAX_USER_DIRENTS as u64 || offset | rights != 0 {
        return Err(EINVAL);
    }
    let file = owned(handle, RIGHT_READ)?;
    if capacity != 0 {
        validate_user_mapping(
            ptr,
            capacity as usize * core::mem::size_of::<UserDirEntry>(),
            true,
        )?;
    }
    let entries = file.readdir().map_err(vfs_error_to_errno)?;
    if entries.len() > MAX_USER_DIRENTS {
        return Err(ENOBUFS);
    }
    for (index, entry) in entries.iter().take(capacity as usize).enumerate() {
        let name = entry.name().as_bytes();
        if name.len() > 64 {
            return Err(ENAMETOOLONG);
        }
        let mut bytes = [0u8; core::mem::size_of::<UserDirEntry>()];
        bytes[..name.len()].copy_from_slice(name);
        bytes[64..72].copy_from_slice(&(name.len() as u64).to_le_bytes());
        bytes[72..76].copy_from_slice(&user_file_type(entry.file_type).to_le_bytes());
        copy_buffer_to_user((ptr as usize + index * bytes.len()) as *mut u8, &bytes)?;
    }
    Ok(entries.len() as i64)
}
fn child(
    handle: u64,
    ptr: u64,
    len: u64,
    kind: u64,
    rights: u64,
    create: bool,
) -> Result<i64, i64> {
    let access = flags(rights)?;
    if !create && kind != 0 {
        return Err(EINVAL);
    }
    let parent = owned(
        handle,
        if create {
            RIGHT_WRITE
        } else {
            rights as u32 & (RIGHT_READ | RIGHT_WRITE)
        },
    )?;
    let held = crate::process::with_current_process(|process| process.handle_rights(handle as u32))
        .ok_or(EINVAL)?
        .map_err(handle_error_to_errno)?;
    if rights as u32 & !held != 0 {
        return Err(EACCES);
    }
    let name = copy_string_from_user_len(ptr as *const u8, len as usize, MAX_USER_PATH)?;
    let file = if create {
        let kind = match kind {
            1 => crate::fs::vfs::FileType::File,
            2 => crate::fs::vfs::FileType::Directory,
            _ => return Err(EINVAL),
        };
        parent.create_child(&name, kind, access)
    } else {
        parent.child(&name, access)
    }
    .map_err(vfs_error_to_errno)?;
    Ok(crate::process::with_current_process_mut(|process| {
        process.handle_create_file(file, rights as u32)
    })
    .ok_or(EINVAL)? as i64)
}
fn move_to(handle: u64, ptr: u64, len: u64, offset: u64, rights: u64) -> Result<i64, i64> {
    if offset | rights != 0 || len < 9 || len > MAX_USER_PATH as u64 + 8 {
        return Err(EINVAL);
    }
    let source = owned(handle, RIGHT_WRITE)?;
    let bytes = copy_buffer_from_user(ptr as *const u8, len as usize)?;
    let destination = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let n = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if n != bytes.len() - 8 {
        return Err(EINVAL);
    }
    let leaf = core::str::from_utf8(&bytes[8..]).map_err(|_| EINVAL)?;
    let destination = owned(destination as u64, RIGHT_WRITE)?;
    source
        .move_to(destination.as_ref(), leaf)
        .map_err(vfs_error_to_errno)?;
    Ok(0)
}
