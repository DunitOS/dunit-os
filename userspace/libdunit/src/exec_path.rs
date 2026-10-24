//! Pure userspace executable path policy: absolute search directories only.
use alloc::string::String;
use alloc::vec::Vec;

pub fn normalize(cwd: &str, path: &str) -> String {
    let combined = if path.starts_with('/') {
        String::from(path)
    } else {
        alloc::format!("{}/{}", cwd, path)
    };
    let mut parts = Vec::new();
    for part in combined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    alloc::format!("/{}", parts.join("/"))
}

pub fn candidates(name: &str, path: &str, cwd: &str) -> Vec<String> {
    if name.is_empty() || name.as_bytes().contains(&0) {
        return Vec::new();
    }
    if name.contains('/') {
        return alloc::vec![normalize(cwd, name)];
    }
    path.trim()
        .split(':')
        .filter(|dir| dir.starts_with('/') && !dir.as_bytes().contains(&0))
        .map(|dir| normalize(dir, name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_absolute_search() {
        assert_eq!(
            candidates("lua", "/system/bin:/app:/users/1/bin", "/tmp"),
            ["/system/bin/lua", "/app/lua", "/users/1/bin/lua"]
        );
    }
    #[test]
    fn no_implicit_current_directory() {
        assert_eq!(candidates("lua", ":.:relative:/app:", "/tmp"), ["/app/lua"]);
        assert!(candidates("lua", "", "/tmp").is_empty());
    }
    #[test]
    fn explicit_paths() {
        assert_eq!(candidates("./a b", "", "/tmp/dir"), ["/tmp/dir/a b"]);
        assert_eq!(candidates("../a", "/app", "/tmp/dir"), ["/tmp/a"]);
        assert_eq!(candidates("/../../tmp/a", "", "/"), ["/tmp/a"]);
        assert!(candidates("a\0b", "/app", "/").is_empty());
    }
}
