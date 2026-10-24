//! Bounded, versioned SpawnArgs transaction. A child is not made runnable until
//! argv/env, file actions, cwd, ELF and TLS preparation have all succeeded.
use super::*;
const EXTENDED: usize = 1usize << 63;
const MAX_ACTIONS: usize = 64;
const MAX_STRINGS: usize = 32;
const HEADER: usize = 24;
enum Action { Close(u32), Dup(u32, u32), Open(u32, u32, String), Chdir(String) }

fn parse_actions(bytes: &[u8]) -> Result<Vec<Action>, i64> {
    let mut result = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        if result.len() == MAX_ACTIONS || rest.len() < HEADER { return Err(EINVAL); }
        let word = |at: usize| u32::from_le_bytes(rest[at..at+4].try_into().unwrap());
        let (tag, fd, src, flags, mode, length) = (word(0), word(4), word(8), word(12), word(16), word(20) as usize);
        if length > MAX_USER_PATH || HEADER + length > rest.len() { return Err(EINVAL); }
        let path_bytes = &rest[HEADER..HEADER+length];
        if path_bytes.contains(&0) { return Err(EINVAL); }
        let path = String::from_utf8(path_bytes.to_vec()).map_err(|_| EINVAL)?;
        let valid_fd = |fd| fd < crate::process::MAX_PROCESS_FD;
        let action = match tag {
            1 if valid_fd(fd) && src == 0 && flags == 0 && mode == 0 && length == 0 => Action::Close(fd),
            2 if valid_fd(fd) && valid_fd(src) && flags == 0 && mode == 0 && length == 0 => Action::Dup(src, fd),
            3 if valid_fd(fd) && src == 0 && !path.is_empty() => {
                open_flags_from_u32(flags & !(64 | 128))?;
                Action::Open(fd, flags, path)
            }
            4 if fd == 0 && src == 0 && flags == 0 && mode == 0 && !path.is_empty() => Action::Chdir(path),
            5 => return Err(EOPNOTSUPP), // directory descriptors are not supported
            _ => return Err(EINVAL),
        };
        result.push(action);
        rest = &rest[HEADER+length..];
    }
    Ok(result)
}

pub(super) fn spawn(path: *const u8, path_len: usize, block: *const u8,
    encoded_len: usize, argc: usize, boundary: usize) -> i64 {
    let requested = match copy_string_from_user_len(path, path_len, MAX_USER_PATH) { Ok(path) if !path.is_empty() => path, Ok(_) => return EINVAL, Err(error) => return error };
    let extended = encoded_len & EXTENDED != 0;
    let length = encoded_len & !EXTENDED;
    if length == 0 || length > MAX_USER_COPY { return EINVAL; }
    // An old five-argument syscall leaves r9 unspecified. Never inspect it
    // unless the explicit version bit in the length opts in to the extension.
    let strings_end = if extended { boundary } else { length };
    if strings_end == 0 || strings_end > length { return EINVAL; }
    let bytes = match copy_buffer_from_user(block, length) { Ok(bytes) => bytes, Err(error) => return error };
    if bytes[strings_end-1] != 0 { return EINVAL; }
    let mut strings = Vec::new();
    for piece in bytes[..strings_end-1].split(|&byte| byte == 0) {
        if strings.len() == MAX_STRINGS || piece.len() > 128 { return -7; }
        match String::from_utf8(piece.to_vec()) { Ok(value) => strings.push(value), Err(_) => return EINVAL }
    }
    if argc == 0 || argc > 16 || argc > strings.len() || strings.len() - argc > 16 { return -7; }
    let actions = match parse_actions(&bytes[strings_end..]) { Ok(actions) => actions, Err(error) => return error };
    let parent_fds = match crate::process::current_process() { Some(parent) => parent.snapshot_fds(), None => return EINVAL };
    let pid = match crate::process::create_user_process_record(requested.clone(), true, false) { Ok(pid) => pid, Err(error) => return process_error_to_errno(error) };
    let transaction = (|| -> Result<(), i64> {
        crate::process::with_process_mut(pid, |child| child.inherit_fds(&parent_fds)).map_err(process_error_to_errno)?;
        apply(pid, &actions)?;
        // CLOEXEC is applied after actions so a dup2 action can deliberately
        // preserve an otherwise close-on-exec source descriptor.
        let close = crate::process::with_process_mut(pid, |child| Ok(child.snapshot_fds().into_iter()
            .filter(|(_, entry)| entry.cloexec).map(|(fd, _)| fd).collect::<Vec<_>>())).map_err(process_error_to_errno)?;
        for fd in close { let cleanup = crate::process::with_process_mut(pid, |child| child.drop_fd(fd)).map_err(process_error_to_errno)?; apply_fd_cleanup(cleanup); }
        let cwd = crate::process::with_process_mut(pid, |child| Ok(child.cwd.clone())).map_err(process_error_to_errno)?;
        let resolved = resolve_exec_path(&cwd, &requested)?;
        let data = read_vfs_file(&cwd, &resolved).map_err(vfs_error_to_errno)?;
        let env: Vec<&str> = strings[argc..].iter().map(|entry| entry.as_str()).collect();
        crate::elf::prepare_process_elf_env_suspended(pid, &data, &strings[..argc], &env).map_err(|_| EIO)?;
        crate::process::mark_process_prepared_as_ready(pid).map_err(process_error_to_errno)?;
        Ok(())
    })();
    match transaction {
        Ok(()) => pid.0 as i64,
        Err(error) => { let _ = crate::process::autoreap_process(pid, "spawn-transaction-failed"); error }
    }
}
fn apply(pid: crate::process::ProcessId, actions: &[Action]) -> Result<(), i64> {
    for action in actions {
        match action {
            Action::Close(fd) => {
                if let Ok(cleanup) = crate::process::with_process_mut(pid, |child| child.drop_fd(*fd)) { apply_fd_cleanup(cleanup); }
            }
            Action::Dup(source, target) => {
                let (_, cleanup) = crate::process::with_process_mut(pid, |child| child.dup_fd(*source, Some(*target))).map_err(process_error_to_errno)?;
                apply_fd_cleanup(cleanup);
                crate::process::with_process_mut(pid, |child| child.set_cloexec(*target, false)).map_err(process_error_to_errno)?;
            }
            Action::Open(fd, bits, path) => {
                let cwd = crate::process::with_process_mut(pid, |child| Ok(child.cwd.clone())).map_err(process_error_to_errno)?;
                let flags = open_flags_from_u32(bits & !(64 | 128))?;
                let vfs_fd = crate::fs::vfs::get_vfs().ok_or(EIO)?.open_at(&cwd, path, flags).map_err(vfs_error_to_errno)?;
                let mut entry = crate::process::FdEntry::vfs(vfs_fd);
                entry.set_status((bits & (1 | 2 | 16 | 64)) as u32);
                entry.cloexec = bits & 128 != 0;
                match crate::process::with_process_mut(pid, |child| child.place_open_vfs_fd(*fd, entry)) {
                    Ok(cleanup) => apply_fd_cleanup(cleanup),
                    Err(error) => { let _ = crate::fs::vfs::get_vfs().map(|vfs| vfs.close(vfs_fd)); return Err(process_error_to_errno(error)); }
                }
            }
            Action::Chdir(path) => {
                let cwd = crate::process::with_process_mut(pid, |child| Ok(child.cwd.clone())).map_err(process_error_to_errno)?;
                let vfs = crate::fs::vfs::get_vfs().ok_or(EIO)?;
                if vfs.stat_at(&cwd, path).map_err(vfs_error_to_errno)?.file_type != crate::fs::vfs::FileType::Directory { return Err(ENOTDIR); }
                let normalized = vfs.normalize_at(&cwd, path).map_err(vfs_error_to_errno)?;
                crate::process::with_process_mut(pid, |child| { child.cwd = normalized; Ok(()) }).map_err(process_error_to_errno)?;
            }
        }
    }
    Ok(())
}
