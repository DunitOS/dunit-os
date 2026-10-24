//! Executable name lookup belongs to userspace; kernel spawn takes a path.
use alloc::string::String;
use alloc::vec::Vec;

/// ABI v0 opt-in: unflagged legacy PTY blocks ignore the formerly unused r9.
pub const PTY_ARGV_BLOCK_FLAG: usize = 1usize << 63;

/// Explicit additions replace inherited keys instead of producing duplicate
/// entries whose meaning would depend on a libc's first/last-match behavior.
pub(crate) fn environment(extra: &[&str]) -> Vec<String> {
    let mut result = Vec::new();
    for index in 0..1024 {
        let Some(entry) = (unsafe { crate::env_get(crate::RUNTIME_ENVP, index) }) else {
            break;
        };
        let key = entry.split('=').next().unwrap_or("");
        if extra
            .iter()
            .any(|value| value.split('=').next() == Some(key))
        {
            continue;
        }
        result.push(String::from(entry));
    }
    for entry in extra {
        result.push(String::from(*entry));
    }
    result
}

pub fn resolve_program(name: &str) -> Result<String, isize> {
    let path = if name.contains('/') {
        String::new()
    } else if let Some(value) = crate::getenv("PATH") {
        String::from(value)
    } else {
        crate::read_to_string("/assets/exec-path")?
    };
    let mut buffer = [0u8; 256];
    let size = crate::getcwd(&mut buffer);
    if size < 0 {
        return Err(size);
    }
    let cwd = core::str::from_utf8(&buffer[..size as usize]).map_err(|_| crate::EINVAL)?;
    let mut error = crate::ENOENT;
    for candidate in crate::exec_path::candidates(name, &path, cwd) {
        let mut stat = crate::FileStat::default();
        let result = crate::stat(&candidate, &mut stat);
        if result == 0 && stat.file_type == crate::FILE_TYPE_FILE {
            return Ok(candidate);
        }
        if result < 0 && result != crate::ENOENT {
            error = result;
        }
    }
    Err(error)
}

/// A nonzero sixth argument selects argv followed by environment strings.
/// All strings are NUL terminated; empty arguments are preserved.
pub fn pty_spawn_args(path: &str, id: u32, argv: &[&str], env: &[&str]) -> isize {
    if argv.is_empty()
        || argv
            .iter()
            .chain(env.iter())
            .any(|s| s.as_bytes().contains(&0))
    {
        return crate::EINVAL;
    }
    let environment = environment(env);
    let env: Vec<&str> = environment.iter().map(|s| s.as_str()).collect();
    if argv.len() > 1024
        || argv
            .iter()
            .chain(env.iter())
            .try_fold(0usize, |size, s| size.checked_add(s.len())?.checked_add(1))
            .map(|size| size > 64 * 1024)
            .unwrap_or(true)
    {
        return crate::EINVAL;
    }
    let resolved = match resolve_program(path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let mut block = Vec::new();
    for string in argv.iter().chain(env.iter()) {
        block.extend_from_slice(string.as_bytes());
        block.push(0);
    }
    crate::syscall6(
        crate::SYSCALL_PTY_SPAWN,
        resolved.as_ptr() as usize,
        resolved.len(),
        id as usize,
        block.as_ptr() as usize,
        block.len() | PTY_ARGV_BLOCK_FLAG,
        argv.len(),
    )
}
