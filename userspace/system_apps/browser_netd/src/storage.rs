//! Crash-safe atomic VFS storage for cache slots, cookies, and endpoint discovery.
use alloc::{format, string::String, vec::Vec};
use dunit::{
    browser::{CACHE_DIR, COOKIES_PATH, ENDPOINT_PATH},
    fs::{self, FileType},
    io::Write,
    Result,
};
use dunit_browser_net::cache::CacheStorage;

const RUN_DIR: &str = "/run";
const RUN_SVC_DIR: &str = "/run/browser-netd";
const VAR_DIR: &str = "/var";
const VAR_LIB_DIR: &str = "/var/lib";
const VAR_SVC_DIR: &str = "/var/lib/browser-netd";
const GENERATION_PATH: &str = "/var/lib/browser-netd/generation";

pub fn ensure_dirs() {
    for dir in [RUN_DIR, RUN_SVC_DIR, VAR_DIR, VAR_LIB_DIR, VAR_SVC_DIR, CACHE_DIR] {
        let _ = fs::create_dir(dir);
    }
}

pub fn write_atomic_file(path: &str, data: &[u8]) -> Result<()> {
    let tmp = format!("{path}.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        f.close()?;
    }
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(path);
        fs::rename(&tmp, path)?;
    }
    Ok(())
}

pub fn publish_endpoint(pid: u32, generation: u32) -> Result<()> {
    let body = format!("pid={pid}\ngen={generation}\n");
    write_atomic_file(ENDPOINT_PATH, body.as_bytes())
}

pub fn next_generation() -> u32 {
    let prev = fs::read_to_string(GENERATION_PATH)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let next = prev.saturating_add(1);
    let _ = write_atomic_file(GENERATION_PATH, format!("{next}\n").as_bytes());
    next
}

pub fn read_cookies_blob() -> Option<Vec<u8>> {
    fs::read_bounded(COOKIES_PATH, 256 * 1024).ok()
}

pub fn write_cookies_blob(data: &[u8]) {
    let _ = write_atomic_file(COOKIES_PATH, data);
}

pub struct VfsCacheStorage {
    dir: String,
}

impl VfsCacheStorage {
    pub fn new(dir: &str) -> Self {
        Self {
            dir: String::from(dir),
        }
    }

    fn full_path(&self, name: &str) -> Option<String> {
        if name.is_empty() || name.contains('/') || name.contains("..") {
            return None;
        }
        Some(format!("{}/{name}", self.dir))
    }
}

impl CacheStorage for VfsCacheStorage {
    fn list_files(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .into_iter()
            .filter(|e| e.file_type == FileType::File && e.name.ends_with(".dnwc"))
            .map(|e| e.name)
            .collect()
    }

    fn read_file(&self, name: &str) -> Option<Vec<u8>> {
        let path = self.full_path(name)?;
        fs::read_bounded(&path, 512 * 1024).ok()
    }

    fn write_atomic(&mut self, name: &str, data: &[u8]) -> bool {
        let Some(path) = self.full_path(name) else {
            return false;
        };
        write_atomic_file(&path, data).is_ok()
    }

    fn remove_file(&mut self, name: &str) {
        if let Some(path) = self.full_path(name) {
            let _ = fs::remove_file(&path);
        }
    }
}

